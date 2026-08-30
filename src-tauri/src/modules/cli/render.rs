//! Compact human output for each command. Anything unexpected falls back to
//! pretty JSON so the CLI never hides a result it does not understand.

use serde_json::Value;

pub fn render(cmd: &str, result: Option<&Value>) -> String {
    let Some(v) = result else {
        return String::new();
    };
    match cmd {
        "terminal.list" => terminal_list(v),
        "terminal.read" => str_field(v, "output").unwrap_or_default().to_string(),
        "terminal.type" | "terminal.run" | "terminal.press" => send_result(cmd, v),
        "tab.open" => tab_open(v),
        "pane.split" => pane_split(v),
        "space.list" => space_list(v),
        "space.new" => space_new(v),
        "notify" => str_field(v, "via")
            .map(|via| format!("notified ({via})"))
            .unwrap_or_else(|| "notified".into()),
        "ping" => ping(v),
        _ => pretty(v),
    }
    .trim_end()
    .to_string()
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn num_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

fn terminal_list(v: &Value) -> String {
    let Some(items) = v.get("terminals").and_then(Value::as_array) else {
        return pretty(v);
    };
    if items.is_empty() {
        return "no terminal panes are open".into();
    }
    let mut out = Vec::with_capacity(items.len());
    for t in items {
        let id = num_field(t, "paneId").unwrap_or(0);
        let title = str_field(t, "title").unwrap_or("");
        let space = str_field(t, "space").unwrap_or("");
        let cwd = str_field(t, "cwd").unwrap_or("-");
        let agent = t
            .get("agent")
            .filter(|a| !a.is_null())
            .map(|a| {
                format!(
                    "  agent={}:{}",
                    str_field(a, "name").unwrap_or("?"),
                    str_field(a, "status").unwrap_or("?")
                )
            })
            .unwrap_or_default();
        let mut marks = Vec::new();
        if t.get("current").and_then(Value::as_bool) == Some(true) {
            marks.push("current");
        }
        if t.get("active").and_then(Value::as_bool) == Some(true) {
            marks.push("active");
        }
        if t.get("private").and_then(Value::as_bool) == Some(true) {
            marks.push("private");
        }
        if t.get("cold").and_then(Value::as_bool) == Some(true) {
            marks.push("cold");
        }
        let marks = if marks.is_empty() {
            String::new()
        } else {
            format!("  [{}]", marks.join(","))
        };
        out.push(format!(
            "#{id:<4} {title:<24} space={space}  cwd={cwd}{agent}{marks}"
        ));
    }
    out.join("\n")
}

fn pane_label(v: &Value) -> String {
    match v.get("pane") {
        Some(p) => format!(
            "#{} '{}'",
            num_field(p, "paneId").unwrap_or(0),
            str_field(p, "title").unwrap_or("")
        ),
        None => "pane".into(),
    }
}

fn send_result(cmd: &str, v: &Value) -> String {
    let kind = str_field(v, "target_kind").unwrap_or("terminal");
    let verb = match cmd {
        "terminal.run" => "submitted to",
        "terminal.press" => "pressed in",
        _ => "typed into",
    };
    let what = match cmd {
        "terminal.press" => str_field(v, "key").map(|k| format!("{k} ")).unwrap_or_default(),
        _ => String::new(),
    };
    format!("{what}{verb} {} ({kind})", pane_label(v))
}

fn tab_open(v: &Value) -> String {
    match (num_field(v, "tabId"), str_field(v, "action")) {
        (Some(id), Some(action)) => format!(
            "{action} tab #{id} '{}'",
            str_field(v, "title").unwrap_or("")
        ),
        _ => pretty(v),
    }
}

fn pane_split(v: &Value) -> String {
    match (num_field(v, "tabId"), num_field(v, "paneId")) {
        (Some(tab), Some(pane)) => {
            let note = str_field(v, "note")
                .map(|n| format!("\nnote: {n}"))
                .unwrap_or_default();
            format!("split: new pane #{pane} in tab #{tab}{note}")
        }
        _ => pretty(v),
    }
}

fn space_list(v: &Value) -> String {
    let Some(items) = v.get("spaces").and_then(Value::as_array) else {
        return pretty(v);
    };
    let mut out = Vec::with_capacity(items.len());
    for s in items {
        let active = if s.get("active").and_then(Value::as_bool) == Some(true) {
            "*"
        } else {
            " "
        };
        let tabs = num_field(s, "tabCount").unwrap_or(0);
        out.push(format!(
            "{active} {:<24} id={}  tabs={tabs}",
            str_field(s, "name").unwrap_or(""),
            str_field(s, "id").unwrap_or("")
        ));
    }
    out.join("\n")
}

fn space_new(v: &Value) -> String {
    match (str_field(v, "spaceId"), str_field(v, "name")) {
        (Some(id), Some(name)) => {
            let note = str_field(v, "note")
                .map(|n| format!("\nnote: {n}"))
                .unwrap_or_default();
            format!("created space '{name}' (id={id}) and switched to it{note}")
        }
        _ => pretty(v),
    }
}

fn ping(v: &Value) -> String {
    format!(
        "pong: koden {} pid {} at {}",
        str_field(v, "version").unwrap_or("?"),
        num_field(v, "pid").unwrap_or(0),
        str_field(v, "endpoint").unwrap_or("?")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn terminal_list_marks_current_and_active() {
        let v = json!({"count": 2, "terminals": [
            {"paneId": 3, "title": "api", "space": "Default", "cwd": "C:/code/api", "agent": null, "current": true, "active": false},
            {"paneId": 4, "title": "worker", "space": "Fleet", "cwd": null, "agent": {"name": "claude", "status": "working"}, "current": false, "active": true, "private": true},
        ]});
        let out = render("terminal.list", Some(&v));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("#3 "));
        assert!(lines[0].contains("[current]"));
        assert!(lines[1].contains("agent=claude:working"));
        assert!(lines[1].contains("[active,private]"));
        assert!(lines[1].contains("cwd=-"));
    }

    #[test]
    fn read_prints_output_verbatim_and_ping_is_one_line() {
        let v = json!({"output": "line 1\nline 2\n"});
        assert_eq!(render("terminal.read", Some(&v)), "line 1\nline 2");
        let p = json!({"pong": true, "version": "0.11.0", "pid": 12, "endpoint": "e"});
        assert_eq!(render("ping", Some(&p)), "pong: koden 0.11.0 pid 12 at e");
    }

    #[test]
    fn unknown_commands_fall_back_to_json() {
        let v = json!({"x": 1});
        assert_eq!(render("future.cmd", Some(&v)), "{\n  \"x\": 1\n}");
        assert_eq!(render("ping", None), "");
    }

    #[test]
    fn send_and_layout_results_are_compact() {
        let v = json!({"pane": {"paneId": 7, "title": "api"}, "target_kind": "shell"});
        assert_eq!(render("terminal.type", Some(&v)), "typed into #7 'api' (shell)");
        let v = json!({"pane": {"paneId": 7, "title": "api"}, "target_kind": "agent", "key": "enter"});
        assert_eq!(render("terminal.press", Some(&v)), "enter pressed in #7 'api' (agent)");
        let v = json!({"tabId": 5, "action": "opened", "title": "shell"});
        assert_eq!(render("tab.open", Some(&v)), "opened tab #5 'shell'");
        let v = json!({"tabId": 5, "paneId": 9, "note": "landed in the active tab"});
        assert!(render("pane.split", Some(&v)).contains("note: landed"));
        let v = json!({"spaces": [{"id": "s1", "name": "Default", "active": true, "tabCount": 2}]});
        assert!(render("space.list", Some(&v)).starts_with("* Default"));
    }
}
