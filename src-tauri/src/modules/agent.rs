use serde_json::{json, Value};

use crate::modules::brain::gist::artifact::HOOK_ARTIFACT_BASENAME;
use crate::modules::brain::memory::MEMORY_DIR;

const HOOK_EVENTS: [(&str, &str); 3] = [
    ("UserPromptSubmit", "working"),
    ("Notification", "attention"),
    ("Stop", "finished"),
];

// Includes the pre-v2.1.139 /dev/tty variant so re-running migrates it, plus
// the bus marker so subagent hooks are recognized for idempotent re-install.
// "notify;Terax" / ".terax/…" are legacy pre-rename "terax" hooks (gated on
// TERAX_TERMINAL, so inert under Koden) — recognized so a re-install
// MIGRATES/removes them instead of leaving dead cruft alongside the live Koden
// hooks. The artifact basename marks the ADR-019 gist-injection group (its own
// ownership CLASS — see `is_gist_group`), so it too is replaced, never
// duplicated, on re-install.
const OWNED_MARKERS: [&str; 6] = [
    "notify;Koden;",
    "koden;notify",
    "director-bus.jsonl",
    "notify;Terax",
    ".terax/agent-bus.jsonl",
    HOOK_ARTIFACT_BASENAME,
];

// The OSC 777 status marker, chosen at run time by the hook's shell. Inside
// tmux (a Koden ssh Space runs every terminal in a tmux window) a raw OSC
// never reaches the client: tmux consumes unknown OSC. Wrapped in a DCS
// passthrough (`ESC P tmux;` + the sequence with every ESC doubled + `ESC \`)
// tmux forwards it verbatim once `allow-passthrough` is on (shell_ssh.rs
// sets it). Every byte is spelled as a JSON escape (`\u001b`, and `\u005c`
// for the final backslash) so the hook text carries no literal backslash:
// Windows shells and argv quoting eat those (the test below caught it).
fn osc_notify_pick(event: &str) -> String {
    format!(
        r#"s='\u001b]777;notify;Koden;{event}\u0007'; [ -n "$TMUX" ] && s='\u001bPtmux;\u001b\u001b]777;notify;Koden;{event}\u0007\u001b\u005c'"#
    )
}

// Gated on KODEN_TERMINAL; no-op outside Koden. Returns the sequence via
// `terminalSequence` because hooks lost /dev/tty access in v2.1.139.
fn hook_cmd(event: &str) -> String {
    let pick = osc_notify_pick(event);
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && {{ {pick}; printf '{{"terminalSequence":"%s"}}' "$s"; }} || true"#
    )
}

// Remote pane events (M2.8): inside a tmux pane on an ssh host, append one
// JSONL line per lifecycle event to ~/.koden/pane-events.jsonl, which Koden
// tails over ssh and joins to the tab through the pane -> window-name ->
// restore-key chain. Its OWN ownership class (own hook group, own stdin): the
// status/turn command already consumes stdin for the bus, and a foreign
// command appended inside an owned group is stripped on every re-install,
// which is exactly how these hooks vanished from HQ on 2026-09-03.
const PANE_EVENTS_MARKER: &str = "pane-events.jsonl";

const PANE_EVENT_HOOKS: [(&str, &str); 4] = [
    ("UserPromptSubmit", "user-prompt"),
    ("Notification", "notification"),
    ("Stop", "stop"),
    ("SessionStart", "session-start"),
];

fn pane_event_hook_cmd(kind: &str) -> String {
    format!(
        r#"[ -n "$TMUX_PANE" ] && {{ mkdir -p "$HOME/.koden"; sid=$(grep -o '"session_id":"[^"]*"' | head -n1 | cut -d'"' -f4); printf '{{"pane":"%s","sessionId":"%s","event":"{kind}","ts":%s}}\n' "$TMUX_PANE" "$sid" "$(date +%s)" >> "$HOME/.koden/{PANE_EVENTS_MARKER}"; }} >/dev/null 2>&1; true"#
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
// Koden can surface the orchestrator's subagents in real time. `parent` stamps
// the emitting session's pty (KODEN_SESSION): the bus is shared by EVERY Koden
// pane's hooks, so without it any pane's claude would steer the Director's
// roster and per-pane subagent nodes couldn't be routed at all.
fn bus_hook_cmd(cmd: &str) -> String {
    let bus = bus_path_str();
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && printf '{{"cmd":"{cmd}","parent":"%s"}}\n' "$KODEN_SESSION" >> "{bus}" || true"#
    )
}

// PreToolUse(Task): append the raw hook input (compacted to one line), wrapped
// as {"parent":"<pty>","task":<raw>}, so Koden can read the subagent's task
// description, name the node, and attribute it to this session. Still
// non-atomic (three writes); the tolerant tool_use_id scanner on the frontend
// (subagentBus.ts) recovers interleaved/corrupt appends.
fn bus_cat_cmd() -> String {
    let bus = bus_path_str();
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && {{ printf '{{"parent":"%s","task":' "$KODEN_SESSION"; cat | tr -d '\r\n'; printf '}}\n'; }} >> "{bus}" || true"#
    )
}

// UserPromptSubmit: the reliable turn-capture path. Claude Code passes the
// submitted prompt as JSON on the hook's stdin; we append one bus line
// {"cmd":"user-turn","id":<KODEN_SESSION>,"data":<raw hook json>} (newlines
// stripped so the bus stays JSONL) and STILL emit the `working` status sequence
// on stdout so the dock status is unchanged. Note the CC drift: pre-2.1.206
// never honored terminalSequence for this event; 2.1.206 honors it
// INTERMITTENTLY (emission is UI-lifecycle-gated inside the CLI, silently
// dropped whenever its emitter is unregistered). The bus line stays the
// authoritative turn channel; the OSC marker is best-effort status only.
fn user_turn_hook_cmd() -> String {
    let bus = bus_path_str();
    let pick = osc_notify_pick("working");
    format!(
        r#"[ -n "$KODEN_TERMINAL" ] && {{ mkdir -p "$(dirname "{bus}")" 2>/dev/null; p="$(cat | tr -d '\r\n')"; [ -z "$p" ] && p=null; printf '{{"cmd":"user-turn","id":"%s","data":%s}}\n' "$KODEN_SESSION" "$p" >> "{bus}"; {pick}; printf '{{"terminalSequence":"%s"}}' "$s"; }} || true"#
    )
}

// ADR-019 — real-time memory injection. A SECOND owned UserPromptSubmit group:
// bounded upward search from $PWD (max 12 levels) for the project's derived
// `.koden-memory/.koden-gist.json`; `cat` it if found — the file already IS the
// complete hook stdout JSON ({"hookSpecificOutput":{... additionalContext}}),
// pre-escaped by serde on the Rust side, so this command never interpolates
// gist bytes through printf. Its stdout stays exactly one JSON document (the
// status group's stdout is a separate hook process — Claude Code merges
// additionalContext across hooks, it never concatenates raw stdouts).
//
// Deliberately UNGATED on $KODEN_TERMINAL: the status/turn-capture hooks gate
// because they need a Koden pane (an OSC consumer + the bus), but memory
// injection is valuable in ANY terminal — plain Windows Terminal sessions get
// the Brain's context too. That is a feature, not an oversight (ADR-019).
//
// The walk stops at the first project marker (`.koden-memory` dir or `.git` —
// file or dir, so worktrees/submodules count) that lacks an artifact: a nested
// project without its own artifact must NOT inherit the outer project's gist,
// and a `.claude/worktrees` agent (never indexed) must not inject main-tree
// context into its stale copy. Fail-open: no artifact → no output; `; true`
// pins exit 0 (a non-zero UserPromptSubmit hook would surface as an error).
fn gist_inject_hook_cmd() -> String {
    format!(
        r#"d="$PWD"; i=0; while [ "$i" -lt 12 ]; do g="$d/{MEMORY_DIR}/{HOOK_ARTIFACT_BASENAME}"; if [ -f "$g" ]; then cat "$g"; break; fi; if [ -d "$d/{MEMORY_DIR}" ] || [ -e "$d/.git" ]; then break; fi; p=$(dirname "$d"); [ "$p" = "$d" ] && break; d="$p"; i=$((i+1)); done; true"#
    )
}

fn group_has_marker(group: &Value, markers: &[&str]) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hs| {
            hs.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| markers.iter().any(|m| c.contains(m)))
            })
        })
}

fn is_ours(group: &Value) -> bool {
    group_has_marker(group, &OWNED_MARKERS)
}

// The ADR-019 gist-injection group is its own ownership CLASS: `add_command_group`
// replaces every owned group for an event before pushing one, so without this
// split the status/turn group install would strip the gist group (and vice
// versa) — one event could never carry both. Each class replaces only itself.
fn is_gist_group(group: &Value) -> bool {
    group_has_marker(group, &[HOOK_ARTIFACT_BASENAME])
}

// The M2.8 pane-events class: replaces only itself, survives every other
// re-install. (Not in OWNED_MARKERS on purpose: `is_ours` must not see it.)
fn is_pane_events_group(group: &Value) -> bool {
    group_has_marker(group, &[PANE_EVENTS_MARKER])
}

// A group with no hooks is inert cruft (e.g. left behind when someone deletes
// our command but not its wrapper). Drop it so the file stays clean.
fn is_empty_group(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_none_or(|hs| hs.is_empty())
}

// Push one fresh `{matcher?, hooks:[{type:"command", command}]}` group after
// stripping every group `replaces` claims (plus inert empty groups). `replaces`
// is the group's ownership class — passing a narrower predicate than `is_ours`
// lets two Koden-owned groups coexist on one event (ADR-019).
fn add_command_group_where(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    matcher: Option<&str>,
    command: &str,
    replaces: impl Fn(&Value) -> bool,
) {
    let arr = hooks.entry(event).or_insert_with(|| json!([]));
    if !arr.is_array() {
        *arr = json!([]);
    }
    let arr = arr.as_array_mut().unwrap();
    arr.retain(|group| !replaces(group) && !is_empty_group(group));
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

// The status/turn/bus class: replaces every owned group EXCEPT the ADR-019
// gist-injection group (which replaces only itself). Legacy /dev/tty + Terax
// variants match `is_ours` via OWNED_MARKERS, so migration behavior is unchanged.
fn add_command_group(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) {
    add_command_group_where(hooks, event, matcher, command, |g| {
        is_ours(g) && !is_gist_group(g)
    });
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
        // UserPromptSubmit also captures the prompt text to the bus (the reliable
        // channel) so Koden's Inputs list shows every turn, not just the first
        // scraped one; it still emits the `working` status sequence on stdout.
        let cmd = if event == "UserPromptSubmit" {
            user_turn_hook_cmd()
        } else {
            hook_cmd(marker)
        };
        add_command_group(hooks, event, None, &cmd);
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
    // ADR-019: the SECOND owned UserPromptSubmit group — per-turn memory
    // injection from the project's derived gist artifact. Its own ownership
    // class, so it coexists with (and is replaced independently of) the
    // status/turn group above.
    add_command_group_where(hooks, "UserPromptSubmit", None, &gist_inject_hook_cmd(), is_gist_group);
    // M2.8 remote pane events: one group per event, own class. Legacy copies
    // that were hand-appended inside the status groups are stripped with
    // those groups above and come back here, as their own groups.
    for (event, kind) in PANE_EVENT_HOOKS {
        add_command_group_where(
            hooks,
            event,
            None,
            &pane_event_hook_cmd(kind),
            is_pane_events_group,
        );
    }
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
    // Gate on the gist-injection group too (ADR-019): a pre-ADR-019 install
    // reads "not installed" until the startup auto-install upgrades it — honest,
    // and self-healing on the next launch.
    HOOK_EVENTS
        .iter()
        .all(|(_, m)| content.contains(&format!("notify;Koden;{m}")))
        && content.contains(HOOK_ARTIFACT_BASENAME)
        // Pre-M2.8-generator installs (hand-added pane-events, or none, or the
        // unwrapped OSC) read "not installed" until the startup auto-install
        // rewrites them.
        && content.contains(PANE_EVENTS_MARKER)
        && content.contains("Ptmux;")
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
        // UserPromptSubmit carries THREE owned groups: status/turn-capture, the
        // ADR-019 gist-injection group, and the M2.8 pane-events group.
        assert_eq!(hook_count(&out, "UserPromptSubmit"), 3);
        assert_eq!(hook_count(&out, "Notification"), 2);
        assert_eq!(hook_count(&out, "Stop"), 2);
        assert_eq!(hook_count(&out, "SessionStart"), 1);
        assert!(command(&out, "SessionStart", 0).contains("session-start"));
        assert!(command(&out, "Notification", 1).contains(PANE_EVENTS_MARKER));
        // Inside tmux the OSC 777 rides a DCS passthrough; outside it is raw.
        assert!(command(&out, "Notification", 0).contains("Ptmux;"));
        assert!(command(&out, "Notification", 0).contains(r#"[ -n "$TMUX" ]"#));
        assert!(command(&out, "Notification", 0).contains("notify;Koden;attention"));
        assert!(command(&out, "Stop", 0).contains("notify;Koden;finished"));
        assert!(command(&out, "UserPromptSubmit", 0).contains("notify;Koden;working"));
        // UserPromptSubmit also appends the prompt to the bus for the Inputs list.
        assert!(command(&out, "UserPromptSubmit", 0).contains("user-turn"));
        assert!(command(&out, "UserPromptSubmit", 0).contains("director-bus.jsonl"));
        assert!(command(&out, "UserPromptSubmit", 1).contains(HOOK_ARTIFACT_BASENAME));
        assert!(command(&out, "Stop", 0).contains("terminalSequence"));
        assert!(!command(&out, "Stop", 0).contains("/dev/tty"));
    }

    /// 2026-09-03: the host's pane-events hooks had been appended inside the
    /// Koden-owned Notification/Stop groups; every Koden launch on HQ replaced
    /// those groups and the remote status pipeline silently lost its source.
    /// A re-install must leave exactly one pane-events group per event, never
    /// nested in a status group.
    #[test]
    fn pane_events_survive_reinstall_even_when_legacy_copies_sit_in_owned_groups() {
        let legacy = json!({
            "hooks": {
                "Notification": [{
                    "hooks": [
                        { "type": "command", "command": hook_cmd("attention") },
                        { "type": "command", "command": pane_event_hook_cmd("notification") }
                    ]
                }],
                "Stop": [{
                    "hooks": [
                        { "type": "command", "command": hook_cmd("finished") },
                        { "type": "command", "command": pane_event_hook_cmd("stop") }
                    ]
                }]
            }
        });
        let out = merge_hooks(legacy);
        for (event, kind) in PANE_EVENT_HOOKS {
            let groups = out["hooks"][event].as_array().unwrap();
            let pane_groups: Vec<_> = groups.iter().filter(|g| is_pane_events_group(g)).collect();
            assert_eq!(pane_groups.len(), 1, "{event}");
            assert!(pane_groups[0]["hooks"][0]["command"].as_str().unwrap().contains(kind));
            assert!(!groups.iter().any(|g| is_ours(g) && is_pane_events_group(g)));
        }
    }

    /// The hook must hand Claude Code valid JSON in both worlds. Run the shell
    /// text through sh (present on every CI runner and dev box) with TMUX
    /// unset and set, and parse what it prints.
    #[test]
    fn hook_cmd_prints_valid_json_raw_and_tmux_wrapped() {
        for (tmux, wrapped) in [(false, false), (true, true)] {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg(hook_cmd("attention")).env("KODEN_TERMINAL", "1");
            if tmux {
                c.env("TMUX", "/tmp/tmux-1/default,1,0");
            } else {
                c.env_remove("TMUX");
            }
            let Ok(out) = c.output() else { return }; // no sh here: skip
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            let v: Value = serde_json::from_str(&text).expect(&text);
            let seq = v["terminalSequence"].as_str().unwrap();
            assert_eq!(seq.starts_with("\u{1b}Ptmux;"), wrapped, "{text}");
            assert!(seq.contains("]777;notify;Koden;attention\u{7}"));
            if wrapped {
                assert!(seq.ends_with("\u{1b}\\"));
                assert!(seq.contains("\u{1b}\u{1b}]777;"));
            }
        }
    }

    #[test]
    fn is_idempotent() {
        let once = merge_hooks(json!({}));
        let twice = merge_hooks(once.clone());
        assert_eq!(once, twice);
        assert_eq!(hook_count(&twice, "UserPromptSubmit"), 3);
        assert_eq!(hook_count(&twice, "Notification"), 2);
        assert_eq!(hook_count(&twice, "SessionStart"), 1);
        assert_eq!(hook_count(&twice, "PreToolUse"), 1);
        assert_eq!(hook_count(&twice, "SubagentStop"), 1);
        assert_eq!(hook_count(&twice, "PostToolUse"), 1);
    }

    /// ADR-019: the gist-injection group's shape — bounded upward walk, cat of
    /// the pre-escaped artifact, marker-stops, and deliberately UNGATED on
    /// KODEN_TERMINAL (memory injection works in ANY terminal, not just Koden
    /// panes — that is the feature).
    #[test]
    fn gist_injection_group_is_ungated_and_bounded() {
        let out = merge_hooks(json!({}));
        let gist = command(&out, "UserPromptSubmit", 1);
        assert!(gist.contains(".koden-memory/.koden-gist.json"));
        assert!(gist.contains("cat "), "injects by cat'ing the pre-escaped artifact");
        assert!(!gist.contains("KODEN_TERMINAL"), "ungated: any terminal gets memory");
        assert!(!gist.contains("printf"), "never interpolates gist bytes through printf");
        assert!(gist.contains("-lt 12"), "upward walk is bounded");
        assert!(gist.contains(".git"), "stops at project markers (nested repos/worktrees)");
        assert!(gist.ends_with("true"), "exit status pinned to 0 (fail-open)");
        // The status group is untouched by the second add (distinct ownership class).
        assert!(command(&out, "UserPromptSubmit", 0).contains("notify;Koden;working"));
    }

    /// ADR-019 migration: a pre-ADR-019 install (single status/turn group) gains
    /// the gist group on re-install; a post-ADR-019 re-install replaces each
    /// group in place — never duplicates, never strips the other class.
    #[test]
    fn gist_injection_group_migrates_and_survives_reinstall() {
        let prev = json!({
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [ { "type": "command", "command": user_turn_hook_cmd() } ] }
                ]
            }
        });
        let migrated = merge_hooks(prev);
        assert_eq!(hook_count(&migrated, "UserPromptSubmit"), 3);
        assert!(command(&migrated, "UserPromptSubmit", 0).contains("user-turn"));
        assert!(command(&migrated, "UserPromptSubmit", 1).contains(HOOK_ARTIFACT_BASENAME));
        // Re-install over BOTH groups is a fixed point (idempotent bytes).
        let again = merge_hooks(migrated.clone());
        assert_eq!(again, migrated);
        // A stale gist-group variant (same marker, older command) is REPLACED,
        // not accumulated.
        let stale = json!({
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [ { "type": "command",
                        "command": "cat \"$PWD/.koden-memory/.koden-gist.json\" 2>/dev/null || true" } ] }
                ]
            }
        });
        let upgraded = merge_hooks(stale);
        assert_eq!(hook_count(&upgraded, "UserPromptSubmit"), 3);
        assert_eq!(
            command(&upgraded, "UserPromptSubmit", 1),
            gist_inject_hook_cmd(),
            "old gist variant upgraded in place"
        );
    }

    #[test]
    fn adds_subagent_lifecycle_hooks() {
        let out = merge_hooks(json!({}));
        assert_eq!(hook_count(&out, "PreToolUse"), 1);
        assert_eq!(hook_count(&out, "SubagentStop"), 1);
        assert_eq!(out["hooks"]["PreToolUse"][0]["matcher"], "Task");
        // PreToolUse(Task) appends the raw hook input wrapped with the session
        // pty ({"parent":"<pty>","task":...}); SubagentStop / PostToolUse
        // append parent-stamped commands. All target the bus file.
        let pre = command(&out, "PreToolUse", 0);
        assert!(pre.contains("cat"));
        assert!(pre.contains("director-bus.jsonl"));
        assert!(pre.contains(r#"{"parent":"%s","task":"#));
        assert!(pre.contains("$KODEN_SESSION"));
        let stop = command(&out, "SubagentStop", 0);
        assert!(stop.contains(r#"{"cmd":"subagent-stop","parent":"%s"}"#));
        assert!(stop.contains("$KODEN_SESSION"));
        assert!(command(&out, "PostToolUse", 0)
            .contains(r#"{"cmd":"director-active","parent":"%s"}"#));
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
        assert_eq!(hook_count(&out, "Notification"), 2);
        assert!(command(&out, "Notification", 0).contains("terminalSequence"));
        assert!(!command(&out, "Notification", 0).contains("/dev/tty"));
    }

    #[test]
    fn migrates_legacy_terax_hooks() {
        // A pre-rename build installed TERAX-gated hooks; a Koden re-install must
        // remove them (they're inert under Koden) rather than leave dead cruft.
        let legacy = json!({
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [ { "type": "command",
                        "command": "[ -n \"$TERAX_TERMINAL\" ] && printf '{\"terminalSequence\":\"\\u001b]777;notify;Terax;working\\u0007\"}' || true" } ] },
                    { "hooks": [ { "type": "command",
                        "command": "[ -n \"$TERAX_SESSION\" ] && printf '{}' >> \"/x/.terax/agent-bus.jsonl\" || true" } ] }
                ]
            }
        });
        let out = merge_hooks(legacy);
        // Both dead Terax groups are gone; what remains is the current pair
        // (status/turn + ADR-019 gist injection).
        assert_eq!(hook_count(&out, "UserPromptSubmit"), 3);
        let cmd = command(&out, "UserPromptSubmit", 0);
        assert!(cmd.contains("notify;Koden;working"));
        assert!(cmd.contains("user-turn"));
        assert!(!cmd.contains("Terax"));
        assert!(!command(&out, "UserPromptSubmit", 1).contains("Terax"));
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
        assert_eq!(hook_count(&out, "Notification"), 3);
        assert_eq!(command(&out, "Notification", 0), "say hi");
    }

    #[test]
    fn replaces_non_object_root() {
        let out = merge_hooks(json!("garbage"));
        assert_eq!(hook_count(&out, "Notification"), 2);
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
        assert_eq!(hook_count(&out, "Notification"), 2);
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
