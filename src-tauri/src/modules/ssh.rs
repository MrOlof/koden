use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use shared_child::SharedChild;

pub const SSH_BINARY_MISSING: &str =
    "OpenSSH client (ssh) not found. Install it or add it to PATH.";

const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const MAX_TIMEOUT: Duration = Duration::from_secs(300);

// BatchMode keeps a missing key or an untrusted host key from turning into a
// prompt nobody can answer; the interactive PTY path deliberately omits it.
const BATCH_ARGS: &[&str] = &[
    "-T",
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "LogLevel=ERROR",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHost {
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Concrete `Host` aliases from an OpenSSH client config. Wildcard (`*`, `?`)
/// and negated (`!`) patterns are skipped, `Match` blocks end the current
/// block, and `Include` is not followed: hosts that only exist in included
/// files do not show up in the picker.
pub fn parse_ssh_config(text: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = split_directive(line) else {
            continue;
        };
        if key.eq_ignore_ascii_case("host") {
            current.clear();
            for pattern in value.split_whitespace() {
                let alias = unquote(pattern);
                if !is_concrete_alias(alias) || !seen.insert(alias.to_string()) {
                    continue;
                }
                hosts.push(SshHost {
                    alias: alias.to_string(),
                    host_name: None,
                    user: None,
                    port: None,
                });
                current.push(hosts.len() - 1);
            }
            continue;
        }
        if key.eq_ignore_ascii_case("match") {
            current.clear();
            continue;
        }
        if current.is_empty() {
            continue;
        }
        let value = unquote(value);
        // OpenSSH keeps the first value it sees for a key.
        match key.to_ascii_lowercase().as_str() {
            "hostname" => {
                for &i in &current {
                    hosts[i].host_name.get_or_insert_with(|| value.to_string());
                }
            }
            "user" => {
                for &i in &current {
                    hosts[i].user.get_or_insert_with(|| value.to_string());
                }
            }
            "port" => {
                if let Ok(port) = value.parse::<u16>() {
                    for &i in &current {
                        hosts[i].port.get_or_insert(port);
                    }
                }
            }
            _ => {}
        }
    }
    hosts
}

fn split_directive(line: &str) -> Option<(&str, &str)> {
    let end = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let key = &line[..end];
    let rest = line[end..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
    if key.is_empty() {
        return None;
    }
    Some((key, rest))
}

fn unquote(value: &str) -> &str {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

fn is_concrete_alias(alias: &str) -> bool {
    !alias.contains(['*', '?']) && !alias.starts_with('!') && is_safe_ssh_host(alias)
}

/// Aliases and hostnames safe to hand to `ssh` as a positional argument:
/// alphanumerics plus `.`, `_`, `-`, `:` (IPv6) and one `user@` prefix. A
/// leading `-` would be parsed as an option, so it is refused along with
/// whitespace and every shell metacharacter.
pub fn is_safe_ssh_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 255 || host.starts_with('-') {
        return false;
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '@'))
    {
        return false;
    }
    match host.split_once('@') {
        None => true,
        Some((user, rest)) => {
            !user.is_empty() && !rest.is_empty() && !rest.contains('@') && !rest.starts_with('-')
        }
    }
}

pub fn validate_ssh_host(host: &str) -> Result<(), String> {
    if is_safe_ssh_host(host) {
        Ok(())
    } else {
        Err(format!("unsafe ssh host: {host:?}"))
    }
}

pub fn resolve_ssh_binary() -> Option<PathBuf> {
    resolve_ssh_binary_with(
        cfg!(windows),
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("SystemRoot").as_deref(),
        &|p| p.is_file(),
    )
}

fn resolve_ssh_binary_with(
    windows: bool,
    path_var: Option<&OsStr>,
    system_root: Option<&OsStr>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let name = if windows { "ssh.exe" } else { "ssh" };
    if let Some(path) = path_var {
        for dir in std::env::split_paths(path) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let candidate = dir.join(name);
            if exists(&candidate) {
                return Some(candidate);
            }
        }
    }
    if windows {
        let root = system_root
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let candidate = root.join("System32").join("OpenSSH").join("ssh.exe");
        if exists(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Runs `remote_command` on `host` through the system OpenSSH client and
/// returns stdout. Never blocks past `timeout`: the child is killed and an
/// error returned, and stdin is closed (or fed `stdin` then closed) so a
/// probe can never sit waiting on the terminal the way Cate's did.
pub fn ssh_exec_capture(
    host: &str,
    remote_command: &str,
    timeout: Duration,
    stdin: Option<&[u8]>,
) -> Result<String, String> {
    validate_ssh_host(host)?;
    let bin = resolve_ssh_binary().ok_or_else(|| SSH_BINARY_MISSING.to_string())?;
    let timeout = timeout.clamp(Duration::from_secs(1), MAX_TIMEOUT);

    let mut cmd = Command::new(bin);
    cmd.args(BATCH_ARGS)
        .arg(host)
        .arg(remote_command)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::modules::proc::hide_console(&mut cmd);

    let child =
        Arc::new(SharedChild::spawn(&mut cmd).map_err(|e| format!("spawn ssh: {e}"))?);
    if let Some(bytes) = stdin {
        if let Some(mut pipe) = child.take_stdin() {
            let data = bytes.to_vec();
            thread::spawn(move || {
                let _ = pipe.write_all(&data);
            });
        }
    }
    let mut stdout_pipe = child
        .take_stdout()
        .ok_or_else(|| "ssh: no stdout pipe".to_string())?;
    let mut stderr_pipe = child
        .take_stderr()
        .ok_or_else(|| "ssh: no stderr pipe".to_string())?;
    let stdout_handle = thread::spawn(move || drain(&mut stdout_pipe));
    let stderr_handle = thread::spawn(move || drain(&mut stderr_pipe));

    let (tx, rx) = mpsc::channel();
    let waiter = Arc::clone(&child);
    thread::spawn(move || {
        let _ = tx.send(waiter.wait());
    });
    let status = match rx.recv_timeout(timeout) {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Err(format!("ssh {host}: {e}")),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "ssh {host}: timed out after {}s",
                timeout.as_secs()
            ));
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr).trim().to_string();
        let detail = if detail.is_empty() {
            format!("exited with {}", status.code().unwrap_or(-1))
        } else {
            detail
        };
        return Err(format!("ssh {host}: {detail}"));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

fn drain<R: Read>(reader: &mut R) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < MAX_CAPTURE_BYTES {
                    let take = (MAX_CAPTURE_BYTES - out.len()).min(n);
                    out.extend_from_slice(&buf[..take]);
                }
            }
        }
    }
    out
}

/// Last non-empty line of a probe's stdout. Login banners and rc noise land
/// before the value, never after it.
pub fn last_line(output: &str) -> String {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

pub fn ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("config"))
}

fn list_hosts_blocking() -> Vec<SshHost> {
    let Some(path) = ssh_config_path() else {
        return Vec::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_ssh_config(&text),
        Err(_) => Vec::new(),
    }
}

#[tauri::command]
pub async fn ssh_list_hosts() -> Result<Vec<SshHost>, String> {
    tauri::async_runtime::spawn_blocking(list_hosts_blocking)
        .await
        .map_err(|e| e.to_string())
}

fn home_blocking(host: &str) -> Result<String, String> {
    let out = ssh_exec_capture(
        host,
        "printf %s \"$HOME\"",
        Duration::from_secs(20),
        None,
    )?;
    let home = last_line(&out);
    if home.is_empty() {
        Err(format!("ssh {host}: could not resolve the remote home directory"))
    } else {
        Ok(home)
    }
}

#[tauri::command]
pub async fn ssh_home(host: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || home_blocking(&host))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxWindow {
    pub name: String,
    pub command: String,
    pub path: String,
}

/// Lines of `tmux list-windows -F "#W\t#{pane_current_command}\t#{pane_current_path}"`.
/// Malformed lines and window names outside the tmux-safe charset are skipped;
/// command/path are host-controlled display strings, passed through verbatim.
pub fn parse_tmux_windows(out: &str) -> Vec<TmuxWindow> {
    let mut windows = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(name), Some(command), Some(path)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if name.is_empty()
            || name.len() > 64
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            continue;
        }
        windows.push(TmuxWindow {
            name: name.to_string(),
            command: command.to_string(),
            path: path.to_string(),
        });
    }
    windows
}

fn tmux_windows_blocking(host: &str, space_key: &str) -> Result<Vec<TmuxWindow>, String> {
    let session = crate::modules::pty::shell_ssh::tmux_session_name(space_key);
    // `sh -c '...'` so the remote login shell only passes a single-quoted
    // string through (same convention as shell_ssh's remote_command).
    // `|| true`: no session / no tmux reads as "0 live windows", not an error.
    let cmd = format!(
        "sh -c 'tmux list-windows -t ={session} -F \"#W\t#{{pane_current_command}}\t#{{pane_current_path}}\" 2>/dev/null || true'"
    );
    let out = ssh_exec_capture(host, &cmd, Duration::from_secs(10), None)?;
    Ok(parse_tmux_windows(&out))
}

/// Live tmux windows of a Space's base session on `host`. Powers the
/// launcher's liveness badges and connect-time session adoption (M2.5 F2 /
/// M2.7): existence comes from tmux itself — no host-side manifest needed
/// for the lean version.
#[tauri::command]
pub async fn ssh_tmux_windows(
    host: String,
    space_key: String,
) -> Result<Vec<TmuxWindow>, String> {
    tauri::async_runtime::spawn_blocking(move || tmux_windows_blocking(&host, &space_key))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_blocks_with_options() {
        let cfg = "\
# comment
Host proxmox
    HostName 192.168.1.240
    User root
    IdentityFile ~/.ssh/id_ed25519

Host docker
  hostname=192.168.1.207
  Port 2222
  User snorlax
";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(
            hosts,
            vec![
                SshHost {
                    alias: "proxmox".into(),
                    host_name: Some("192.168.1.240".into()),
                    user: Some("root".into()),
                    port: None,
                },
                SshHost {
                    alias: "docker".into(),
                    host_name: Some("192.168.1.207".into()),
                    user: Some("snorlax".into()),
                    port: Some(2222),
                },
            ]
        );
    }

    #[test]
    fn multi_alias_host_line_yields_one_entry_per_alias() {
        let cfg = "Host livetek kewh-vps\n  HostName 143.14.50.231\n  User snorlax\n";
        let hosts = parse_ssh_config(cfg);
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, vec!["livetek", "kewh-vps"]);
        assert!(hosts
            .iter()
            .all(|h| h.host_name.as_deref() == Some("143.14.50.231")));
    }

    #[test]
    fn skips_wildcard_and_negated_patterns() {
        let cfg = "Host *\n  User me\nHost *.example.com\nHost !bad good\n  Port 22\nHost web?\n";
        let hosts = parse_ssh_config(cfg);
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, vec!["good"]);
        assert_eq!(hosts[0].port, Some(22));
        assert_eq!(hosts[0].user, None);
    }

    #[test]
    fn match_block_ends_host_block_and_include_is_ignored() {
        let cfg = "Include ~/.ssh/extra\nHost a\n  User u\nMatch host b\n  User other\n  Port 5\nHost c\n";
        let hosts = parse_ssh_config(cfg);
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, vec!["a", "c"]);
        assert_eq!(hosts[0].user.as_deref(), Some("u"));
        assert_eq!(hosts[0].port, None);
    }

    #[test]
    fn first_value_wins_and_duplicates_are_dropped() {
        let cfg = "Host a\n  User first\n  User second\n  Port notanumber\nHost a\n  User third\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].user.as_deref(), Some("first"));
        assert_eq!(hosts[0].port, None);
    }

    #[test]
    fn quoted_values_are_unwrapped() {
        let cfg = "Host \"q\"\n  HostName \"box.local\"\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts[0].alias, "q");
        assert_eq!(hosts[0].host_name.as_deref(), Some("box.local"));
    }

    #[test]
    fn empty_or_garbage_config_yields_nothing() {
        assert!(parse_ssh_config("").is_empty());
        assert!(parse_ssh_config("   \n#only comments\n").is_empty());
        assert!(parse_ssh_config("User me\nPort 22\n").is_empty());
    }

    #[test]
    fn host_serializes_camel_case_and_omits_missing_fields() {
        let host = SshHost {
            alias: "a".into(),
            host_name: Some("h".into()),
            user: None,
            port: Some(22),
        };
        let json = serde_json::to_string(&host).unwrap();
        assert_eq!(json, r#"{"alias":"a","hostName":"h","port":22}"#);
    }

    #[test]
    fn host_validator_accepts_aliases_hostnames_and_addresses() {
        for host in [
            "workbench",
            "kewh-vps",
            "box.example.com",
            "192.168.1.240",
            "fe80::1",
            "kosta@workbench",
            "user_name@10.0.0.1",
        ] {
            assert!(is_safe_ssh_host(host), "should accept {host}");
        }
    }

    #[test]
    fn host_validator_rejects_option_injection_and_metachars() {
        for host in [
            "",
            "-oProxyCommand=calc",
            "-v",
            "host name",
            "host\tname",
            "host;rm",
            "host`id`",
            "host$(id)",
            "host|x",
            "host&x",
            "host'x",
            "host\"x",
            "host\\x",
            "host/x",
            "host\nx",
            "@host",
            "user@",
            "user@-host",
            "a@b@c",
            "host*",
            "host?",
        ] {
            assert!(!is_safe_ssh_host(host), "should reject {host:?}");
        }
        assert!(!is_safe_ssh_host(&"a".repeat(256)));
        assert!(validate_ssh_host("-x").is_err());
        assert!(validate_ssh_host("ok").is_ok());
    }

    #[test]
    fn last_line_skips_banners_and_blank_tails() {
        assert_eq!(last_line("Welcome!\n/home/kosta\n\n"), "/home/kosta");
        assert_eq!(last_line("  \n"), "");
    }

    fn exists_set(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p: &Path| set.iter().any(|s| s == p)
    }

    #[test]
    fn resolver_prefers_path_entries_in_order() {
        let exists = exists_set(&["/opt/bin/ssh", "/usr/bin/ssh"]);
        let path = std::env::join_paths(["/nope", "/usr/bin", "/opt/bin"]).unwrap();
        let found = resolve_ssh_binary_with(false, Some(&path), None, &exists);
        assert_eq!(found, Some(PathBuf::from("/usr/bin/ssh")));
    }

    // Backslash paths only join as Windows paths on Windows; the Unix leg would
    // compare `D:\Win/System32/...` against the expected form.
    #[cfg(windows)]
    #[test]
    fn resolver_falls_back_to_system32_openssh_on_windows() {
        let exists = exists_set(&[r"D:\Win\System32\OpenSSH\ssh.exe"]);
        let path = std::env::join_paths([r"C:\tools", r"C:\other"]).unwrap();
        let found = resolve_ssh_binary_with(
            true,
            Some(&path),
            Some(OsStr::new(r"D:\Win")),
            &exists,
        );
        assert_eq!(
            found,
            Some(PathBuf::from(r"D:\Win\System32\OpenSSH\ssh.exe"))
        );
    }

    #[test]
    fn resolver_returns_none_when_nothing_exists() {
        let exists = |_: &Path| false;
        assert_eq!(
            resolve_ssh_binary_with(true, Some(OsStr::new(r"C:\x")), None, &exists),
            None
        );
        assert_eq!(resolve_ssh_binary_with(false, None, None, &exists), None);
    }

    #[test]
    fn exec_capture_refuses_unsafe_host_before_spawning() {
        let err = ssh_exec_capture("-oProxyCommand=x", "true", Duration::from_secs(1), None)
            .unwrap_err();
        assert!(err.contains("unsafe ssh host"), "got: {err}");
    }

    #[test]
    fn parses_tmux_window_lines_and_skips_junk() {
        let out = "w-rk-abc\tclaude\t/home/k/proj\n\
                   w-rk-def\tbash\t/home/k\n\
                   bad name!\tzsh\t/tmp\n\
                   only-two-fields\tbash\n\
                   \n";
        let ws = parse_tmux_windows(out);
        assert_eq!(ws.len(), 2);
        assert_eq!(ws[0].name, "w-rk-abc");
        assert_eq!(ws[0].command, "claude");
        assert_eq!(ws[0].path, "/home/k/proj");
        assert_eq!(ws[1].name, "w-rk-def");
    }

    #[test]
    fn tmux_window_serializes_camel_case() {
        let w = TmuxWindow {
            name: "w-a".into(),
            command: "claude".into(),
            path: "/x".into(),
        };
        assert_eq!(
            serde_json::to_string(&w).unwrap(),
            r#"{"name":"w-a","command":"claude","path":"/x"}"#
        );
    }
}
