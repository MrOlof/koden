use serde_json::{json, Value};

const HOOK_EVENTS: [(&str, &str); 3] = [
    ("UserPromptSubmit", "working"),
    ("Notification", "attention"),
    ("Stop", "finished"),
];

// Includes the pre-v2.1.139 /dev/tty variant so re-running migrates it, plus
// the bus marker so subagent hooks are recognized for idempotent re-install.
const OWNED_MARKERS: [&str; 3] = ["notify;Koden;", "koden;notify", "director-bus.jsonl"];

// Gated on KODEN_TERMINAL; no-op outside Koden. Returns the sequence via
// `terminalSequence` because hooks lost /dev/tty access in v2.1.139.
fn hook_cmd(event: &str) -> String {
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && printf '{{"terminalSequence":"\\u001b]777;notify;Koden;{event}\\u0007"}}' || true"#
    )
}

// Absolute path of the Director command bus, forward-slashed for the POSIX
// shell Claude Code runs hooks in.
fn bus_path_str() -> String {
    dirs::home_dir()
        .map(|h| {
            h.join(".koden")
                .join("director-bus.jsonl")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .unwrap_or_default()
}

// Subagent lifecycle hooks append a command to the bus file directly (rather
// than emitting a terminalSequence, which isn't honored for these events) so
// Koden can surface the orchestrator's subagents in real time.
fn bus_hook_cmd(cmd: &str) -> String {
    let bus = bus_path_str();
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && printf '%s\n' '{{"cmd":"{cmd}"}}' >> "{bus}" || true"#
    )
}

// PreToolUse(Task): append the raw hook input (compacted to one line) so Koden
// can read the subagent's task description and name the node for it.
fn bus_cat_cmd() -> String {
    let bus = bus_path_str();
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && {{ cat | tr -d '\r\n'; printf '\n'; }} >> "{bus}" || true"#
    )
}

fn is_ours(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hs| {
            hs.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| OWNED_MARKERS.iter().any(|m| c.contains(m)))
            })
        })
}

// A group with no hooks is inert cruft (e.g. left behind when someone deletes
// our command but not its wrapper). Drop it so the file stays clean.
fn is_empty_group(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_none_or(|hs| hs.is_empty())
}

fn add_command_group(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) {
    let arr = hooks.entry(event).or_insert_with(|| json!([]));
    if !arr.is_array() {
        *arr = json!([]);
    }
    let arr = arr.as_array_mut().unwrap();
    arr.retain(|group| !is_ours(group) && !is_empty_group(group));
    let mut group = serde_json::Map::new();
    if let Some(m) = matcher {
        group.insert("matcher".into(), json!(m));
    }
    group.insert(
        "hooks".into(),
        json!([ { "type": "command", "command": command } ]),
    );
    arr.push(Value::Object(group));
}

fn merge_hooks(mut root: Value) -> Value {
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();

    for (event, marker) in HOOK_EVENTS {
        add_command_group(hooks, event, None, &hook_cmd(marker));
    }
    // Subagent lifecycle: surface the orchestrator's Claude Code subagents in
    // Koden in real time by appending to the bus file. PreToolUse needs a Task
    // matcher; SubagentStop fires when a subagent completes.
    add_command_group(hooks, "PreToolUse", Some("Task"), &bus_cat_cmd());
    add_command_group(hooks, "SubagentStop", None, &bus_hook_cmd("subagent-stop"));
    // PostToolUse fires after every tool the orchestrator runs (including the
    // AskUserQuestion it answers mid-turn, which never re-fires UserPromptSubmit)
    // so Koden can keep the Director shown as "working" while it's active.
    add_command_group(hooks, "PostToolUse", None, &bus_hook_cmd("director-active"));
    root
}

fn existing_config(contents: Option<&str>, path: &std::path::Path) -> Result<Value, String> {
    match contents {
        Some(s) if !s.trim().is_empty() => serde_json::from_str::<Value>(s).map_err(|e| {
            format!("{} is not valid JSON ({e}); refusing to overwrite", path.display())
        }),
        _ => Ok(json!({})),
    }
}

fn settings_path() -> Result<std::path::PathBuf, String> {
    Ok(dirs::home_dir()
        .ok_or_else(|| "could not resolve home dir".to_string())?
        .join(".claude")
        .join("settings.json"))
}

#[tauri::command]
pub fn agent_enable_claude_hooks() -> Result<(), String> {
    let path = settings_path()?;
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => existing_config(Some(&s), &path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };

    let merged = merge_hooks(existing);
    let out = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;

    // Write to a sibling temp file then rename so a crash mid-write can't leave
    // a truncated settings.json.
    let tmp = path.with_extension("json.koden-tmp");
    std::fs::write(&tmp, out).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename into {}: {e}", path.display())
    })?;
    Ok(())
}

#[tauri::command]
pub fn agent_claude_hooks_status() -> bool {
    let Some(content) = settings_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
    else {
        return false;
    };
    HOOK_EVENTS
        .iter()
        .all(|(_, m)| content.contains(&format!("notify;Koden;{m}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook_count(root: &Value, event: &str) -> usize {
        root["hooks"][event].as_array().map_or(0, Vec::len)
    }

    fn command(root: &Value, event: &str, idx: usize) -> String {
        root["hooks"][event][idx]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn adds_all_event_hooks_to_empty_config() {
        let out = merge_hooks(json!({}));
        assert_eq!(hook_count(&out, "UserPromptSubmit"), 1);
        assert_eq!(hook_count(&out, "Notification"), 1);
        assert_eq!(hook_count(&out, "Stop"), 1);
        assert!(command(&out, "Notification", 0).contains("notify;Koden;attention"));
        assert!(command(&out, "Stop", 0).contains("notify;Koden;finished"));
        assert!(command(&out, "UserPromptSubmit", 0).contains("notify;Koden;working"));
        assert!(command(&out, "Stop", 0).contains("terminalSequence"));
        assert!(!command(&out, "Stop", 0).contains("/dev/tty"));
    }

    #[test]
    fn is_idempotent() {
        let once = merge_hooks(json!({}));
        let twice = merge_hooks(once.clone());
        assert_eq!(once, twice);
        assert_eq!(hook_count(&twice, "Notification"), 1);
        assert_eq!(hook_count(&twice, "PreToolUse"), 1);
        assert_eq!(hook_count(&twice, "SubagentStop"), 1);
        assert_eq!(hook_count(&twice, "PostToolUse"), 1);
    }

    #[test]
    fn adds_subagent_lifecycle_hooks() {
        let out = merge_hooks(json!({}));
        assert_eq!(hook_count(&out, "PreToolUse"), 1);
        assert_eq!(hook_count(&out, "SubagentStop"), 1);
        assert_eq!(out["hooks"]["PreToolUse"][0]["matcher"], "Task");
        // PreToolUse(Task) appends the raw hook input to the bus (for naming);
        // SubagentStop appends a stop command. Both target the bus file.
        let pre = command(&out, "PreToolUse", 0);
        assert!(pre.contains("cat"));
        assert!(pre.contains("director-bus.jsonl"));
        assert!(command(&out, "SubagentStop", 0).contains(r#"{"cmd":"subagent-stop"}"#));
        assert!(command(&out, "PostToolUse", 0).contains(r#"{"cmd":"director-active"}"#));
    }

    #[test]
    fn migrates_legacy_dev_tty_hook() {
        let legacy = json!({
            "hooks": {
                "Notification": [
                    { "hooks": [ {
                        "type": "command",
                        "command": "[ -n \"$KODEN_TERMINAL\" ] && printf '\\033]777;koden;notify\\033\\\\' > /dev/tty || true"
                    } ] }
                ]
            }
        });
        let out = merge_hooks(legacy);
        assert_eq!(hook_count(&out, "Notification"), 1);
        assert!(command(&out, "Notification", 0).contains("terminalSequence"));
        assert!(!command(&out, "Notification", 0).contains("/dev/tty"));
    }

    #[test]
    fn preserves_unrelated_settings_and_foreign_hooks() {
        let input = json!({
            "permissions": { "allow": ["Bash"] },
            "hooks": {
                "Notification": [
                    { "hooks": [ { "type": "command", "command": "say hi" } ] }
                ]
            }
        });
        let out = merge_hooks(input);
        assert_eq!(out["permissions"]["allow"][0], "Bash");
        assert_eq!(hook_count(&out, "Notification"), 2);
        assert_eq!(command(&out, "Notification", 0), "say hi");
    }

    #[test]
    fn replaces_non_object_root() {
        let out = merge_hooks(json!("garbage"));
        assert_eq!(hook_count(&out, "Notification"), 1);
    }

    #[test]
    fn prunes_empty_groups_and_collapses_duplicates() {
        let input = json!({
            "hooks": {
                "Notification": [
                    { "hooks": [] },
                    { "hooks": [ { "type": "command", "command": hook_cmd("attention") } ] }
                ]
            }
        });
        let out = merge_hooks(input);
        assert_eq!(hook_count(&out, "Notification"), 1);
        assert!(command(&out, "Notification", 0).contains("notify;Koden;attention"));
    }

    #[test]
    fn existing_config_absent_or_empty_starts_fresh() {
        let p = std::path::Path::new("/x/settings.json");
        assert_eq!(existing_config(None, p).unwrap(), json!({}));
        assert_eq!(existing_config(Some("   \n"), p).unwrap(), json!({}));
    }

    #[test]
    fn existing_config_refuses_to_clobber_invalid_json() {
        let p = std::path::Path::new("/x/settings.json");
        assert!(existing_config(Some("{ not json,"), p).is_err());
        assert_eq!(
            existing_config(Some(r#"{"permissions":{}}"#), p).unwrap(),
            json!({ "permissions": {} })
        );
    }
}
