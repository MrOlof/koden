//! PTY over the system OpenSSH client. The rc scripts are pushed to
//! `~/.koden/shell` on the host once per content hash; the login shell is
//! detected on the host at exec time so no probe round-trip is needed.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use portable_pty::CommandBuilder;

use crate::modules::ssh;

const ZSHENV: &str = include_str!("scripts/zshenv.zsh");
const ZPROFILE: &str = include_str!("scripts/zprofile.zsh");
const ZLOGIN: &str = include_str!("scripts/zlogin.zsh");
const ZSHRC: &str = include_str!("scripts/zshrc.zsh");
const BASHRC: &str = include_str!("scripts/bashrc.bash");

const REMOTE_DIR: &str = ".koden/shell";
const MARKER_PREFIX: &str = ".koden-";
const HEREDOC_EOF: &str = "__KODEN_RC_EOF__";
const INSTALL_OK: &str = "koden-rc-ok";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30);

// Launch lines are evaluated by a second `sh -c` on the host (or by tmux), so
// `$HOME` / `$SHELL` stay unexpanded here: `\"` and `\$` survive the outer
// single-quoted `sh -c '...'` in every login shell that honours POSIX quotes.
const ZSH_INTEGRATED: &str = r#"exec env KODEN_TERMINAL=1 COLORTERM=truecolor ZDOTDIR=\"\$HOME/.koden/shell\" \"\$SHELL\" -l"#;
const ZSH_PLAIN: &str = r#"exec \"\$SHELL\" -l"#;
const BASH_INTEGRATED: &str = r#"exec env KODEN_TERMINAL=1 COLORTERM=truecolor \"\$SHELL\" --rcfile \"\$HOME/.koden/shell/bashrc.bash\" -i"#;
const BASH_PLAIN: &str = r#"exec \"\$SHELL\" -i"#;
const OTHER_SHELL: &str = r#"exec \"\$SHELL\" -l"#;

struct RcFile {
    name: &'static str,
    content: String,
}

fn bundle() -> Vec<RcFile> {
    [
        (".zshenv", ZSHENV),
        (".zprofile", ZPROFILE),
        (".zshrc", ZSHRC),
        (".zlogin", ZLOGIN),
        ("bashrc.bash", BASHRC),
    ]
    .into_iter()
    .map(|(name, raw)| RcFile {
        name,
        content: normalize_script(raw),
    })
    .collect()
}

fn normalize_script(raw: &str) -> String {
    let mut s = raw.replace("\r\n", "\n");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn bundle_hash(files: &[RcFile]) -> String {
    let mut hasher = blake3::Hasher::new();
    for f in files {
        hasher.update(f.name.as_bytes());
        hasher.update(b"\0");
        hasher.update(f.content.as_bytes());
        hasher.update(b"\0");
    }
    hasher.finalize().to_hex()[..16].to_string()
}

// Fed to `sh -s` on the host with cwd already inside the rc dir. Quoted
// heredocs keep the contents verbatim; the marker is replaced last so an
// interrupted upload is retried on the next spawn.
fn installer_script(files: &[RcFile], hash: &str) -> String {
    let mut s = String::from("set -e\n");
    s.push_str(&format!(
        "if [ -f \"{MARKER_PREFIX}{hash}\" ]; then printf %s {INSTALL_OK}; exit 0; fi\n"
    ));
    for f in files {
        s.push_str(&format!(
            "cat > \"{}.tmp\" <<'{HEREDOC_EOF}'\n{}{HEREDOC_EOF}\n",
            f.name, f.content
        ));
        s.push_str(&format!("mv -f \"{0}.tmp\" \"{0}\"\n", f.name));
    }
    s.push_str(&format!(
        "rm -f {MARKER_PREFIX}*\n: > \"{MARKER_PREFIX}{hash}\"\nprintf %s {INSTALL_OK}\n"
    ));
    s
}

fn install_command() -> String {
    format!("sh -c 'mkdir -p \"$HOME/{REMOTE_DIR}\" && cd \"$HOME/{REMOTE_DIR}\" && exec sh -s'")
}

fn installed_cache() -> &'static Mutex<HashSet<String>> {
    static CACHE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashSet::new()))
}

fn ensure_installed(host: &str) -> Result<(), String> {
    let files = bundle();
    let hash = bundle_hash(&files);
    let key = format!("{host}\0{hash}");
    if installed_cache()
        .lock()
        .expect("ssh rc cache poisoned")
        .contains(&key)
    {
        return Ok(());
    }
    let script = installer_script(&files, &hash);
    let out = ssh::ssh_exec_capture(
        host,
        &install_command(),
        INSTALL_TIMEOUT,
        Some(script.as_bytes()),
    )?;
    let last = ssh::last_line(&out);
    if last != INSTALL_OK {
        return Err(format!("unexpected rc installer output: {last:?}"));
    }
    installed_cache()
        .lock()
        .expect("ssh rc cache poisoned")
        .insert(key);
    Ok(())
}

/// `<prefix><sanitized id>` restricted to the characters tmux accepts
/// unquoted, capped, with a fallback when nothing survives.
fn tmux_name(prefix: &str, id: &str, fallback: &str) -> String {
    let mut out = String::from(prefix);
    let mut last_dash = prefix.ends_with('-');
    for c in id.chars() {
        let mapped = if c.is_ascii_alphanumeric() || c == '_' {
            c
        } else {
            '-'
        };
        if mapped == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(mapped);
        if out.len() >= 48 {
            break;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.len() <= prefix.trim_end_matches('-').len() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// `koden-<id>` for the Space's base tmux session.
pub fn tmux_session_name(space_id: &str) -> String {
    tmux_name("koden-", space_id, "koden-space")
}

/// `w-<leaf restore key>`: one tmux window per Koden pane, named by the
/// restart-stable restore key so a restored pane reattaches to its own
/// window (M2.5 Feature 1).
pub fn tmux_window_name(leaf_key: &str) -> String {
    tmux_name("w-", leaf_key, "w-pane")
}

fn remote_path_literal(path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Ok("\"\"".into());
    }
    if path.chars().any(|c| c == '\'' || c == '\\' || c.is_control()) {
        return Err(format!("unsupported characters in remote path: {path:?}"));
    }
    let (prefix, rest) = if path == "~" {
        ("$HOME", "")
    } else if let Some(rest) = path.strip_prefix("~/") {
        ("$HOME/", rest)
    } else if path.starts_with('/') {
        ("", path)
    } else {
        return Err(format!("remote path must be absolute: {path}"));
    };
    let escaped = rest
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    Ok(format!("\"{prefix}{escaped}\""))
}

/// The single argument handed to `ssh -t <host>`: a POSIX script wrapped in
/// `sh -c '...'` so the remote login shell (bash, zsh, fish, csh) only has to
/// pass a single-quoted string through. It cds into `path` (falling back to
/// `$HOME`), then execs the login shell with Koden's rc files when they are
/// installed, plain otherwise. With a tmux session name the shell runs inside
/// tmux when it exists on the host — windowed mode (session + window, M2.5)
/// gives every Koden pane its own window in the Space's base session and
/// attaches through a per-client grouped session, so panes and devices don't
/// mirror each other; legacy mode (session only) is the old `new-session -A`.
pub fn remote_command(
    path: &str,
    tmux_session: Option<&str>,
    tmux_window: Option<&str>,
) -> Result<String, String> {
    let p = remote_path_literal(path)?;
    let mut parts: Vec<String> = vec![
        format!("d=\"$HOME/{REMOTE_DIR}\""),
        format!("p={p}"),
        "if [ -n \"$p\" ] && [ -d \"$p\" ]; then cd \"$p\"; else cd \"$HOME\"; fi".into(),
        "SHELL=\"${SHELL:-/bin/sh}\"; export SHELL".into(),
        "if [ -n \"$ZDOTDIR\" ] && [ \"$ZDOTDIR\" != \"$d\" ]; then KODEN_USER_ZDOTDIR=\"$ZDOTDIR\"; export KODEN_USER_ZDOTDIR; fi".into(),
        format!(
            "case \"$SHELL\" in \
             */zsh) if [ -f \"$d/.zshrc\" ]; then c=\"{ZSH_INTEGRATED}\"; else c=\"{ZSH_PLAIN}\"; fi ;; \
             */bash) if [ -f \"$d/bashrc.bash\" ]; then c=\"{BASH_INTEGRATED}\"; else c=\"{BASH_PLAIN}\"; fi ;; \
             *) c=\"{OTHER_SHELL}\" ;; \
             esac"
        ),
    ];
    match (tmux_session, tmux_window) {
        (Some(name), Some(win)) => {
            // Windowed mode. Guarded creates tolerate two panes racing on a
            // fresh host (the loser's create fails silently, the window check
            // runs after); `destroy-unattached` reaps the grouped viewport on
            // detach while the base session keeps running. `$$` (remote sh
            // pid) makes the viewport name unique per connection.
            // ponytail: a pid-reuse collision on the viewport name fails the
            // attach; the pane's Enter-to-retry respawns with a fresh pid.
            parts.push(format!(
                "if command -v tmux >/dev/null 2>&1; then \
                 tmux has-session -t ={name} 2>/dev/null || tmux new-session -d -s {name} -n {win} -c \"$PWD\" \"$c\" 2>/dev/null || true; \
                 tmux list-windows -t ={name} -F \"#W\" 2>/dev/null | grep -qx -- {win} || tmux new-window -d -t ={name} -n {win} -c \"$PWD\" \"$c\" 2>/dev/null || true; \
                 tmux set-option -t {name} status off 2>/dev/null || true; \
                 tmux set-option -g fill-character \" \" 2>/dev/null || true; \
                 tmux set-option -g allow-passthrough all 2>/dev/null || tmux set-option -g allow-passthrough on 2>/dev/null || true; \
                 tmux set-option -w -t ={name}:={win} window-size latest 2>/dev/null || true; \
                 tmux set-option -w -t ={name}:={win} aggressive-resize on 2>/dev/null || true; \
                 exec tmux new-session -t ={name} -s {name}-$$ \\; set-option destroy-unattached on \\; set-option status off \\; select-window -t :={win}; \
                 fi"
            ));
        }
        (Some(name), None) => {
            parts.push(format!(
                "if command -v tmux >/dev/null 2>&1; then exec tmux new-session -A -s {name} -c \"$PWD\" \"$c\"; fi"
            ));
        }
        (None, _) => {}
    }
    parts.push("exec sh -c \"$c\"".into());
    let script = parts.join("; ");
    debug_assert!(!script.contains('\''));
    Ok(format!("sh -c '{script}'"))
}

/// A tab restored from a local session can carry a Windows cwd into an ssh
/// Space; anything that is not a POSIX absolute or `~` path falls back to the
/// Space's remote path (empty = remote home) instead of refusing to spawn.
pub fn remote_cwd_or<'a>(cwd: Option<&'a str>, env_path: &'a str) -> &'a str {
    match cwd.map(str::trim) {
        Some(c) if c == "~" || c.starts_with("~/") || c.starts_with('/') => c,
        _ => env_path,
    }
}

pub fn build(
    cwd: Option<String>,
    env_path: &str,
    host: &str,
    tmux_key: Option<&str>,
    tmux_window_key: Option<&str>,
) -> Result<CommandBuilder, String> {
    ssh::validate_ssh_host(host)?;
    let bin = ssh::resolve_ssh_binary().ok_or_else(|| ssh::SSH_BINARY_MISSING.to_string())?;
    let path = remote_cwd_or(cwd.as_deref(), env_path);
    let session = tmux_key.map(tmux_session_name);
    // A window only means something inside a session.
    let window = session
        .as_deref()
        .and(tmux_window_key)
        .map(tmux_window_name);
    let remote = remote_command(path, session.as_deref(), window.as_deref())?;
    if let Err(e) = ensure_installed(host) {
        log::warn!("ssh shell integration disabled for {host}: {e}");
    }

    let mut cmd = CommandBuilder::new(bin);
    // 15s x 2 probes: a dead link (sleep, cable pull, VPN drop) surfaces as an
    // exit within ~30s instead of OpenSSH's default 30s x 3 = 90s of a
    // frozen-looking pane.
    for arg in [
        "-t",
        "-o",
        "ConnectTimeout=20",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=2",
    ] {
        cmd.arg(arg);
    }
    cmd.arg(host);
    cmd.arg(remote);
    cmd.env("TERM", "xterm-256color");
    if let Some(home) = dirs::home_dir() {
        cmd.cwd(home);
    }
    log::info!("spawning ssh shell: {host} ({path})");
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn inner_script(remote: &str) -> &str {
        remote
            .strip_prefix("sh -c '")
            .and_then(|s| s.strip_suffix('\''))
            .expect("wrapped in sh -c '...'")
    }

    fn find_sh() -> Option<PathBuf> {
        let name = if cfg!(windows) { "sh.exe" } else { "sh" };
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    }

    fn sh_syntax_ok(sh: &Path, script: &str) -> bool {
        let dir = std::env::temp_dir().join(format!(
            "koden-ssh-syntax-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("script.sh");
        std::fs::write(&file, script).unwrap();
        let ok = Command::new(sh)
            .arg("-n")
            .arg(&file)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_dir_all(&dir);
        ok
    }

    #[test]
    fn remote_command_cds_into_path_and_execs_detected_shell() {
        let remote = remote_command("/home/kosta/repo", None, None).unwrap();
        let script = inner_script(&remote);
        assert!(!script.contains('\''), "inner script must not contain single quotes");
        assert!(script.contains("p=\"/home/kosta/repo\"; "));
        assert!(script.contains("cd \"$p\"; else cd \"$HOME\""));
        assert!(script.contains("*/zsh) if [ -f \"$d/.zshrc\" ]"));
        assert!(script.contains(r#"ZDOTDIR=\"\$HOME/.koden/shell\" \"\$SHELL\" -l"#));
        assert!(script.contains(r#"--rcfile \"\$HOME/.koden/shell/bashrc.bash\" -i"#));
        assert!(script.contains("KODEN_TERMINAL=1"));
        assert!(script.ends_with("; exec sh -c \"$c\""));
        assert!(!script.contains("tmux"));
        assert!(!script.contains('\n'));
    }

    #[test]
    fn remote_command_expands_tilde_and_accepts_empty_path() {
        let home = remote_command("~", None, None).unwrap();
        assert!(inner_script(&home).contains("p=\"$HOME\"; "));
        let sub = remote_command("~/src/koden", None, None).unwrap();
        assert!(inner_script(&sub).contains("p=\"$HOME/src/koden\"; "));
        let empty = remote_command("", None, None).unwrap();
        assert!(inner_script(&empty).contains("p=\"\"; "));
    }

    #[test]
    fn remote_command_escapes_double_quote_specials() {
        let remote = remote_command("/tmp/a \"b\" $c `d`", None, None).unwrap();
        assert!(inner_script(&remote).contains(r#"p="/tmp/a \"b\" \$c \`d\`"; "#));
    }

    #[test]
    fn remote_command_rejects_unsafe_or_relative_paths() {
        assert!(remote_command("/tmp/it's", None, None).is_err());
        assert!(remote_command("/tmp/back\\slash", None, None).is_err());
        assert!(remote_command("/tmp/new\nline", None, None).is_err());
        assert!(remote_command("relative/path", None, None).is_err());
        assert!(remote_command("~user/x", None, None).is_err());
    }

    #[test]
    fn remote_cwd_falls_back_to_env_path_for_non_posix_cwd() {
        assert_eq!(remote_cwd_or(Some("/srv/app"), "/home/k"), "/srv/app");
        assert_eq!(remote_cwd_or(Some("~/src"), "/home/k"), "~/src");
        assert_eq!(remote_cwd_or(Some("C:/Users/k/repo"), "/home/k"), "/home/k");
        assert_eq!(remote_cwd_or(Some(r"C:\Users\k"), ""), "");
        assert_eq!(remote_cwd_or(Some("  "), "/home/k"), "/home/k");
        assert_eq!(remote_cwd_or(None, "~"), "~");
    }

    #[test]
    fn remote_command_wraps_shell_in_tmux_when_requested() {
        let remote = remote_command("/srv/app", Some("koden-abc"), None).unwrap();
        let script = inner_script(&remote);
        assert!(script.contains(
            "if command -v tmux >/dev/null 2>&1; then exec tmux new-session -A -s koden-abc -c \"$PWD\" \"$c\"; fi; exec sh -c \"$c\""
        ));
    }

    #[test]
    fn remote_command_windowed_creates_window_and_grouped_attach() {
        let remote =
            remote_command("/srv/app", Some("koden-abc"), Some("w-rk1")).unwrap();
        let script = inner_script(&remote);
        assert!(!script.contains('\''), "no single quotes: {script}");
        assert!(!script.contains('\n'));
        // Guarded base-session create with our window as window 0.
        assert!(script.contains(
            "tmux has-session -t =koden-abc 2>/dev/null || tmux new-session -d -s koden-abc -n w-rk1 -c \"$PWD\" \"$c\" 2>/dev/null || true"
        ));
        // Guarded window create for an existing base session.
        assert!(script.contains(
            "tmux list-windows -t =koden-abc -F \"#W\" 2>/dev/null | grep -qx -- w-rk1 || tmux new-window -d -t =koden-abc -n w-rk1 -c \"$PWD\" \"$c\" 2>/dev/null || true"
        ));
        // Multi-client defaults: active machine wins the window size, no
        // tmux status bar (Koden's UI is the chrome).
        assert!(script.contains("tmux set-option -w -t =koden-abc:=w-rk1 window-size latest"));
        assert!(script.contains("tmux set-option -w -t =koden-abc:=w-rk1 aggressive-resize on"));
        assert!(script.contains("tmux set-option -t koden-abc status off"));
        // Per-client grouped viewport, reaped on detach, pinned to our window.
        assert!(script.contains(
            "exec tmux new-session -t =koden-abc -s koden-abc-$$ \\; set-option destroy-unattached on \\; set-option status off \\; select-window -t :=w-rk1"
        ));
        // Plain-shell fallback still closes the script.
        assert!(script.ends_with("; exec sh -c \"$c\""));
    }

    #[test]
    fn tmux_window_name_is_sanitised() {
        assert_eq!(tmux_window_name("rk-mthk-u0pjur"), "w-rk-mthk-u0pjur");
        assert_eq!(tmux_window_name("My Pane: v2"), "w-My-Pane-v2");
        assert_eq!(tmux_window_name("..."), "w-pane");
        assert_eq!(tmux_window_name(""), "w-pane");
        assert!(tmux_window_name(&"x".repeat(200)).len() <= 48);
    }

    #[test]
    fn tmux_session_name_is_sanitised() {
        assert_eq!(tmux_session_name("abc-123"), "koden-abc-123");
        assert_eq!(tmux_session_name("My Space: v2.0"), "koden-My-Space-v2-0");
        assert_eq!(tmux_session_name("..."), "koden-space");
        assert_eq!(tmux_session_name(""), "koden-space");
        let long = tmux_session_name(&"x".repeat(200));
        assert!(long.len() <= 48);
        assert!(long.starts_with("koden-xxx"));
    }

    #[test]
    fn installer_script_writes_every_rc_file_behind_a_content_marker() {
        let files = bundle();
        let hash = bundle_hash(&files);
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        let script = installer_script(&files, &hash);
        assert!(script.starts_with("set -e\n"));
        assert!(script.contains(&format!("if [ -f \".koden-{hash}\" ]; then printf %s koden-rc-ok; exit 0; fi")));
        for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin", "bashrc.bash"] {
            assert!(script.contains(&format!("cat > \"{name}.tmp\" <<'__KODEN_RC_EOF__'\n")));
            assert!(script.contains(&format!("mv -f \"{name}.tmp\" \"{name}\"\n")));
        }
        assert!(script.ends_with(&format!(
            "rm -f .koden-*\n: > \".koden-{hash}\"\nprintf %s koden-rc-ok\n"
        )));
        for f in &files {
            assert!(!f.content.lines().any(|l| l == HEREDOC_EOF));
            assert!(!f.content.contains('\r'));
        }
    }

    #[test]
    fn bundle_hash_tracks_content() {
        let a = bundle();
        let mut b = bundle();
        b[0].content.push_str("# changed\n");
        assert_ne!(bundle_hash(&a), bundle_hash(&b));
        assert_eq!(bundle_hash(&a), bundle_hash(&bundle()));
    }

    #[test]
    fn install_command_is_single_quoted_posix() {
        let cmd = install_command();
        assert_eq!(
            cmd,
            "sh -c 'mkdir -p \"$HOME/.koden/shell\" && cd \"$HOME/.koden/shell\" && exec sh -s'"
        );
    }

    #[test]
    fn generated_scripts_pass_sh_syntax_check_when_sh_is_available() {
        let Some(sh) = find_sh() else {
            eprintln!("sh not found on PATH, skipping syntax check");
            return;
        };
        let plain = remote_command("/home/k/repo", None, None).unwrap();
        assert!(sh_syntax_ok(&sh, inner_script(&plain)));
        let tmux = remote_command("~/x", Some("koden-s1"), None).unwrap();
        assert!(sh_syntax_ok(&sh, inner_script(&tmux)));
        let windowed = remote_command("~/x", Some("koden-s1"), Some("w-rk1")).unwrap();
        assert!(sh_syntax_ok(&sh, inner_script(&windowed)));
        let files = bundle();
        assert!(sh_syntax_ok(&sh, &installer_script(&files, &bundle_hash(&files))));
    }

    struct FakeHost {
        dir: PathBuf,
        home: PathBuf,
        shell: String,
    }

    // A throwaway $HOME with Koden's rc files installed and a fake login shell
    // that prints what it was exec'd with, so the generated script can be run
    // for real through `sh`.
    fn fake_host(shell_name: &str) -> FakeHost {
        let dir = std::env::temp_dir().join(format!(
            "koden-ssh-fn-{}-{}-{}",
            shell_name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let home = dir.join("home");
        let rc = home.join(REMOTE_DIR);
        std::fs::create_dir_all(&rc).unwrap();
        for f in bundle() {
            std::fs::write(rc.join(f.name), &f.content).unwrap();
        }
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let shell = bin.join(shell_name);
        std::fs::write(
            &shell,
            "#!/bin/sh\nprintf 'ARGS=%s\\n' \"$*\"\nprintf 'KT=%s\\n' \"$KODEN_TERMINAL\"\nprintf 'ZD=%s\\n' \"$ZDOTDIR\"\nprintf 'UZD=%s\\n' \"$KODEN_USER_ZDOTDIR\"\nprintf 'HOME=%s\\n' \"$HOME\"\nprintf 'CWD=%s\\n' \"$(pwd)\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        FakeHost {
            dir,
            home,
            shell: shell.to_string_lossy().replace('\\', "/"),
        }
    }

    fn run_remote(sh: &Path, host: &FakeHost, remote: &str, zdotdir: Option<&str>) -> String {
        let mut cmd = Command::new(sh);
        cmd.arg("-c")
            .arg(inner_script(remote))
            .env("HOME", host.home.to_string_lossy().replace('\\', "/"))
            .env("SHELL", &host.shell)
            .env_remove("KODEN_TERMINAL")
            .env_remove("KODEN_USER_ZDOTDIR");
        match zdotdir {
            Some(z) => {
                cmd.env("ZDOTDIR", z);
            }
            None => {
                cmd.env_remove("ZDOTDIR");
            }
        }
        let out = cmd.output().expect("run generated script");
        let _ = std::fs::remove_dir_all(&host.dir);
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
    }

    fn line<'a>(out: &'a str, key: &str) -> &'a str {
        out.lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .unwrap_or_else(|| panic!("no {key} line in {out:?}"))
    }

    #[test]
    fn remote_script_execs_bash_with_rcfile_and_falls_back_to_home() {
        let Some(sh) = find_sh() else {
            eprintln!("sh not found on PATH, skipping functional check");
            return;
        };
        let host = fake_host("bash");
        let remote = remote_command("/definitely/not/here", None, None).unwrap();
        let out = run_remote(&sh, &host, &remote, None);
        // Compared against the $HOME the shell saw: MSYS sh reports the temp
        // dir as /tmp/... while the Rust side holds C:/..., same directory.
        let home = line(&out, "HOME");
        assert_eq!(
            line(&out, "ARGS"),
            format!("--rcfile {home}/.koden/shell/bashrc.bash -i")
        );
        assert_eq!(line(&out, "KT"), "1");
        assert_eq!(line(&out, "UZD"), "");
        assert!(line(&out, "CWD").ends_with("/home"), "got: {out}");
    }

    #[test]
    fn remote_script_execs_zsh_with_zdotdir_and_preserves_user_zdotdir() {
        let Some(sh) = find_sh() else {
            eprintln!("sh not found on PATH, skipping functional check");
            return;
        };
        let host = fake_host("zsh");
        let target = host.home.join("proj");
        std::fs::create_dir_all(&target).unwrap();
        let remote = remote_command("~/proj", None, None).unwrap();
        let out = run_remote(&sh, &host, &remote, Some("/custom/zdot"));
        let home = line(&out, "HOME");
        assert_eq!(line(&out, "ARGS"), "-l");
        assert_eq!(line(&out, "ZD"), format!("{home}/.koden/shell"));
        assert_eq!(line(&out, "UZD"), "/custom/zdot");
        assert_eq!(line(&out, "KT"), "1");
        assert!(line(&out, "CWD").ends_with("/home/proj"), "got: {out}");
    }

    #[test]
    fn remote_script_runs_unknown_shell_plain() {
        let Some(sh) = find_sh() else {
            eprintln!("sh not found on PATH, skipping functional check");
            return;
        };
        let host = fake_host("nu");
        let remote = remote_command("", None, None).unwrap();
        let out = run_remote(&sh, &host, &remote, None);
        assert_eq!(line(&out, "ARGS"), "-l");
        assert_eq!(line(&out, "KT"), "");
        assert_eq!(line(&out, "ZD"), "");
    }

    #[test]
    fn build_refuses_unsafe_host() {
        let err = build(None, "/home/k", "-oProxyCommand=x", None, None).unwrap_err();
        assert!(err.contains("unsafe ssh host"), "got: {err}");
    }
}
