//! Codex CLI turn capture — the Codex sibling of `agent.rs`.
//!
//! Codex (>= ~0.116) has a `UserPromptSubmit` lifecycle hook that, like Claude
//! Code, gets the submitted prompt as JSON on stdin (field `prompt`). We register
//! a CAPTURE-ONLY hook that appends the SAME bus line Koden already reads —
//! `{"cmd":"user-turn","id":<KODEN_SESSION>,"data":<raw hook json>}` — so the
//! Inputs list shows every Codex turn with zero frontend changes
//! (AgentBusBridge reads `data.prompt`). Capture-only = it writes NOTHING to
//! stdout, because Codex treats hook stdout as extra developer context.
//!
//! Config lives in ~/.codex/config.toml (TOML, not JSON). To avoid mangling the
//! user's existing config (MCP servers, model settings, comments) we APPEND an
//! array-of-tables block rather than parse + reserialize. The Windows command
//! points at a tiny `.ps1` we write to ~/.koden so there's no inline pwsh/JSON
//! quoting inside the TOML.

const MARKER: &str = "koden:codex-user-turn";

fn home() -> Result<std::path::PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "could not resolve home dir".to_string())
}

// Forward-slashed bus path for the POSIX hook (mac/linux).
fn bus_posix(home: &std::path::Path) -> String {
    home.join(".koden")
        .join("director-bus.jsonl")
        .to_string_lossy()
        .replace('\\', "/")
}

// POSIX command (mac/linux): same wrap-raw-stdin trick as agent.rs's
// user_turn_hook_cmd, but with NO stdout (Codex would inject it as context).
fn posix_cmd(home: &std::path::Path) -> String {
    let bus = bus_posix(home);
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && {{ mkdir -p "$(dirname "{bus}")" 2>/dev/null; p="$(cat | tr -d '\r\n')"; [ -z "$p" ] && p=null; printf '{{"cmd":"user-turn","id":"%s","data":%s}}\n' "$KODEN_SESSION" "$p" >> "{bus}"; }} || true"#
    )
}

// Windows hook body (a real file → no TOML/shell escaping). Mirrors posix_cmd:
// wrap the raw stdin JSON as `data`, append one line, never write stdout.
const PS1_BODY: &str = r#"# koden:codex-user-turn — Koden turn capture for Codex. Capture-only (no stdout).
if (-not $env:KODEN_TERMINAL) { exit 0 }
try {
  $raw = ([Console]::In.ReadToEnd()) -replace '[\r\n]', ''
  if ([string]::IsNullOrWhiteSpace($raw)) { $raw = 'null' }
  $bus = Join-Path $env:USERPROFILE '.koden\director-bus.jsonl'
  New-Item -ItemType Directory -Force -Path (Split-Path $bus) | Out-Null
  $line = '{"cmd":"user-turn","id":"' + $env:KODEN_SESSION + '","data":' + $raw + '}'
  Add-Content -LiteralPath $bus -Value $line -Encoding utf8
} catch { }
exit 0
"#;

// The TOML block we append. `command` runs on mac/linux; Codex uses
// `command_windows` on Windows. Triple-single-quote literal for the POSIX
// command (it contains both quote kinds); single-quote literal for the Windows
// path (no escaping needed).
fn hook_block(home: &std::path::Path, ps1_win_path: &str) -> String {
    let posix = posix_cmd(home);
    format!(
        "\n# {MARKER} — Koden turn capture for Codex (safe to remove this whole block)\n\
         [[hooks.UserPromptSubmit]]\n\
         [[hooks.UserPromptSubmit.hooks]]\n\
         type = \"command\"\n\
         command = '''{posix}'''\n\
         command_windows = 'pwsh -NoProfile -NonInteractive -File \"{ps1_win_path}\"'\n"
    )
}

// Already installed? Idempotent on the marker.
// ponytail: marker-contains skip — a format change won't auto-update an existing
// block; the block is self-labelled "safe to remove", so the upgrade path is
// delete-the-block + re-run. Good enough until the hook shape actually changes.
fn needs_install(contents: &str) -> bool {
    !contents.contains(MARKER)
}

/// Install the Codex `UserPromptSubmit` turn-capture hook. No-op (Ok) if Codex
/// isn't installed (~/.codex absent) so we never create config for a non-Codex
/// user, and idempotent if already present.
#[tauri::command]
pub fn agent_enable_codex_hooks() -> Result<(), String> {
    let home = home()?;
    let codex_dir = home.join(".codex");
    if !codex_dir.is_dir() {
        return Ok(()); // Codex not installed — nothing to wire.
    }

    // Write the Windows hook script under ~/.koden (overwrite = idempotent).
    let koden_dir = home.join(".koden");
    std::fs::create_dir_all(&koden_dir)
        .map_err(|e| format!("create {}: {e}", koden_dir.display()))?;
    let ps1 = koden_dir.join("koden-codex-turn.ps1");
    std::fs::write(&ps1, PS1_BODY).map_err(|e| format!("write {}: {e}", ps1.display()))?;

    let cfg = codex_dir.join("config.toml");
    let existing = match std::fs::read_to_string(&cfg) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("read {}: {e}", cfg.display())),
    };
    if !needs_install(&existing) {
        return Ok(());
    }

    let block = hook_block(&home, &ps1.to_string_lossy());
    let mut out = existing;
    out.push_str(&block);

    // Atomic temp + rename so a crash mid-write can't truncate the user's config.
    let tmp = cfg.with_extension("toml.koden-tmp");
    std::fs::write(&tmp, &out).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &cfg).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename into {}: {e}", cfg.display())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotent_on_marker() {
        assert!(needs_install("")); // empty config → install
        assert!(needs_install("[tui]\nnotifications = []\n")); // foreign-only → install
        let home = std::path::Path::new("/home/u");
        let block = hook_block(home, "C:\\Users\\u\\.koden\\koden-codex-turn.ps1");
        // The freshly-built block must carry the marker AND the bus contract...
        assert!(block.contains(MARKER));
        assert!(block.contains(r#""cmd":"user-turn""#));
        assert!(block.contains("[[hooks.UserPromptSubmit]]"));
        assert!(block.contains("command_windows ="));
        // ...and re-reading a config that already has it must skip.
        let prior = format!("[tui]\nnotifications = []\n{block}");
        assert!(!needs_install(&prior));
    }

    #[test]
    fn block_has_no_stdout_emit() {
        // Capture-only: the POSIX command must not printf a terminalSequence
        // (that's the Claude path; Codex would inject stdout as context).
        let cmd = posix_cmd(std::path::Path::new("/home/u"));
        assert!(!cmd.contains("terminalSequence"));
        assert!(cmd.contains("director-bus.jsonl"));
    }
}
