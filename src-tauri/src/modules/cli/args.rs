//! Pure argv parser for `koden ...` plus the help text that doubles as the
//! agent-facing documentation. No clap: the grammar is a dozen commands.

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    /// Wire command, e.g. `terminal.read`.
    pub cmd: String,
    pub args: Map<String, Value>,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Help(String),
    Usage(String),
    Invoke(Invocation),
}

pub const MAX_LINES: u64 = 5000;
pub const DEFAULT_LINES: u64 = 200;
pub const PRESS_KEYS: &[&str] = &[
    "enter",
    "escape",
    "ctrl-c",
    "ctrl-d",
    "ctrl-l",
    "ctrl-z",
    "tab",
    "backspace",
    "up",
    "down",
    "left",
    "right",
];
pub const SPLIT_DIRS: &[&str] = &["left", "right", "up", "down"];
pub const TAB_KINDS: &[&str] = &["terminal", "note", "notes", "tasks", "board"];
pub const SPLIT_KINDS: &[&str] = &["terminal", "note", "notes", "tasks"];

#[derive(Clone, Copy)]
enum Positional {
    None,
    /// All remaining words joined with single spaces.
    Text(&'static str),
    /// Exactly one word.
    One(&'static str),
}

struct Spec {
    words: [&'static str; 2],
    cmd: &'static str,
    flags: &'static [&'static str],
    positional: Positional,
}

/// Every flag the parser knows and whether it consumes a value. Unknown
/// `--x` is an error everywhere so a typo never becomes typed text.
const FLAGS: &[(&str, bool)] = &[
    ("--json", false),
    ("--help", false),
    ("--panel", true),
    ("--lines", true),
    ("--raw", false),
    ("--cwd", true),
    ("--title", true),
    ("--dir", true),
    ("--root", true),
];

const SPECS: &[Spec] = &[
    Spec {
        words: ["terminal", "list"],
        cmd: "terminal.list",
        flags: &[],
        positional: Positional::None,
    },
    Spec {
        words: ["terminal", "read"],
        cmd: "terminal.read",
        flags: &["--panel", "--lines", "--raw"],
        positional: Positional::None,
    },
    Spec {
        words: ["terminal", "type"],
        cmd: "terminal.type",
        flags: &["--panel"],
        positional: Positional::Text("text"),
    },
    Spec {
        words: ["terminal", "press"],
        cmd: "terminal.press",
        flags: &["--panel"],
        positional: Positional::One("key"),
    },
    Spec {
        words: ["terminal", "run"],
        cmd: "terminal.run",
        flags: &["--panel"],
        positional: Positional::Text("text"),
    },
    Spec {
        words: ["tab", "open"],
        cmd: "tab.open",
        flags: &["--cwd", "--title"],
        positional: Positional::One("kind"),
    },
    Spec {
        words: ["pane", "split"],
        cmd: "pane.split",
        flags: &["--dir", "--title"],
        positional: Positional::One("kind"),
    },
    Spec {
        words: ["space", "list"],
        cmd: "space.list",
        flags: &[],
        positional: Positional::None,
    },
    Spec {
        words: ["space", "new"],
        cmd: "space.new",
        flags: &["--root"],
        positional: Positional::Text("name"),
    },
    Spec {
        words: ["notify", ""],
        cmd: "notify",
        flags: &[],
        positional: Positional::Text("message"),
    },
    Spec {
        words: ["ping", ""],
        cmd: "ping",
        flags: &[],
        positional: Positional::None,
    },
];

const GROUPS: &[&str] = &["terminal", "tab", "pane", "space", "notify", "ping"];

pub fn parse(argv: &[String]) -> Parsed {
    let mut positionals: Vec<String> = Vec::new();
    let mut flags: Vec<(String, Option<String>)> = Vec::new();
    let mut passthrough = false;
    let mut i = 0;
    while i < argv.len() {
        let tok = argv[i].as_str();
        i += 1;
        if passthrough {
            positionals.push(tok.to_string());
            continue;
        }
        if tok == "--" {
            passthrough = true;
            continue;
        }
        if tok == "-h" {
            flags.push(("--help".into(), None));
            continue;
        }
        if tok.starts_with("--") && tok.len() > 2 {
            let (name, inline) = match tok.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (tok.to_string(), None),
            };
            let Some((_, takes_value)) = FLAGS.iter().find(|(f, _)| *f == name) else {
                return Parsed::Usage(format!(
                    "unknown option '{name}' (put '--' before text that starts with a dash)"
                ));
            };
            let value = if *takes_value {
                match inline {
                    Some(v) => Some(v),
                    None => {
                        if i >= argv.len() {
                            return Parsed::Usage(format!("'{name}' needs a value"));
                        }
                        i += 1;
                        Some(argv[i - 1].clone())
                    }
                }
            } else {
                if inline.is_some() {
                    return Parsed::Usage(format!("'{name}' does not take a value"));
                }
                None
            };
            flags.push((name, value));
            continue;
        }
        positionals.push(tok.to_string());
    }

    let json = flags.iter().any(|(f, _)| f == "--json");
    let wants_help = flags.iter().any(|(f, _)| f == "--help");
    let first = positionals.first().map(String::as_str).unwrap_or("");
    let group_word = if first == "help" {
        positionals.get(1).map(String::as_str).unwrap_or("")
    } else {
        first
    };
    if positionals.is_empty() || first == "help" {
        return Parsed::Help(help_for(group_word));
    }
    if !GROUPS.contains(&first) {
        return Parsed::Usage(format!(
            "unknown command '{first}'. Commands: {}",
            GROUPS.join(", ")
        ));
    }
    let second = positionals.get(1).map(String::as_str).unwrap_or("");
    let spec = SPECS.iter().find(|s| {
        s.words[0] == first && (s.words[1].is_empty() || s.words[1] == second)
    });
    if wants_help {
        return Parsed::Help(help_for(first));
    }
    let Some(spec) = spec else {
        return Parsed::Usage(format!(
            "unknown subcommand '{first} {second}'. Run 'koden {first} --help'."
        ));
    };
    let rest_from = if spec.words[1].is_empty() { 1 } else { 2 };
    let rest: Vec<&str> = positionals[rest_from..].iter().map(String::as_str).collect();

    let mut args = Map::new();
    for (name, value) in &flags {
        if name == "--json" || name == "--help" {
            continue;
        }
        if !spec.flags.contains(&name.as_str()) {
            return Parsed::Usage(format!("'{name}' is not valid for 'koden {}'", spec.cmd.replace('.', " ")));
        }
        let key = name.trim_start_matches("--");
        match (key, value) {
            ("raw", None) => {
                args.insert("raw".into(), Value::Bool(true));
            }
            ("lines", Some(v)) => match v.parse::<u64>() {
                Ok(n) if (1..=MAX_LINES).contains(&n) => {
                    args.insert("lines".into(), Value::from(n));
                }
                _ => {
                    return Parsed::Usage(format!(
                        "'--lines' must be a number between 1 and {MAX_LINES}"
                    ))
                }
            },
            ("dir", Some(v)) => {
                let d = match v.to_ascii_lowercase().as_str() {
                    "top" => "up".to_string(),
                    "bottom" => "down".to_string(),
                    other => other.to_string(),
                };
                if !SPLIT_DIRS.contains(&d.as_str()) {
                    return Parsed::Usage(format!(
                        "'--dir' must be one of: {}",
                        SPLIT_DIRS.join(", ")
                    ));
                }
                args.insert("dir".into(), Value::String(d));
            }
            (k, Some(v)) => {
                if v.trim().is_empty() {
                    return Parsed::Usage(format!("'{name}' must not be empty"));
                }
                args.insert(k.into(), Value::String(v.clone()));
            }
            (k, None) => {
                args.insert(k.into(), Value::Bool(true));
            }
        }
    }

    match spec.positional {
        Positional::None => {
            if !rest.is_empty() {
                return Parsed::Usage(format!(
                    "'koden {}' takes no arguments (got '{}')",
                    spec.cmd.replace('.', " "),
                    rest.join(" ")
                ));
            }
        }
        Positional::Text(key) => {
            let text = rest.join(" ");
            if text.trim().is_empty() {
                return Parsed::Usage(format!(
                    "'koden {}' needs <{key}>",
                    spec.cmd.replace('.', " ")
                ));
            }
            args.insert(key.into(), Value::String(text));
        }
        Positional::One(key) => {
            if rest.len() != 1 {
                return Parsed::Usage(format!(
                    "'koden {}' needs exactly one <{key}>",
                    spec.cmd.replace('.', " ")
                ));
            }
            let v = rest[0].to_ascii_lowercase();
            let allowed: &[&str] = match spec.cmd {
                "terminal.press" => PRESS_KEYS,
                "tab.open" => TAB_KINDS,
                "pane.split" => SPLIT_KINDS,
                _ => &[],
            };
            if !allowed.is_empty() && !allowed.contains(&v.as_str()) {
                return Parsed::Usage(format!(
                    "<{key}> must be one of: {}",
                    allowed.join(", ")
                ));
            }
            args.insert(key.into(), Value::String(v));
        }
    }
    if spec.cmd == "pane.split" && !args.contains_key("dir") {
        return Parsed::Usage("'koden pane split' needs --dir left|right|up|down".into());
    }

    Parsed::Invoke(Invocation {
        cmd: spec.cmd.to_string(),
        args,
        json,
    })
}

pub fn help_for(group: &str) -> String {
    match group {
        "terminal" => HELP_TERMINAL.to_string(),
        "tab" | "pane" => HELP_LAYOUT.to_string(),
        "space" => HELP_SPACE.to_string(),
        "notify" => HELP_NOTIFY.to_string(),
        "ping" => HELP_PING.to_string(),
        _ => HELP_TOP.to_string(),
    }
}

const HELP_TOP: &str = "\
koden: control the Koden window that owns this terminal.

Usage: koden [--json] <command> [options]

Commands
  terminal list                      every pane in every space; this one is [current]
  terminal read [--lines N] [--raw]  last N lines (default 200) of a pane
  terminal type <text>               type at the pane, no Enter
  terminal press <key>               enter | escape | ctrl-c | tab | up | down | ...
  terminal run <text>                type and press Enter
  tab open <kind>                    terminal | note | tasks | board  [--cwd DIR] [--title T]
  pane split <kind> --dir <side>     terminal | note | tasks; left | right | up | down
  space list                         spaces and the active one
  space new <name> [--root DIR]      create a space and switch to it
  notify <message>                   in-app notification attributed to this terminal
  ping                               round trip; prints the instance version and pid

Options
  --panel <id|title>   act on another pane (id or fuzzy title from 'terminal list').
                       Default: the terminal running this command.
  --json               print the raw JSON reply envelope instead of text
  --help               per-group help with examples: koden terminal --help

Read before you type. Input goes to whatever owns the pane's PTY: a foreground
TUI (an agent prompt, vim, less) receives the keystrokes, not the shell. Run
'koden terminal read' first and look at what is on screen.

Exit status: 0 ok, 1 error (message on stderr), 2 usage.
Access is governed by Settings > CLI (Read / Control per surface). If
KODEN_CLI_ENDPOINT is unset, this shell was not opened by Koden.";

const HELP_TERMINAL: &str = "\
koden terminal: read and drive terminal panes.

  koden terminal list
      One line per pane: #id, title, space, cwd, agent (name:state) and marks
      [current] (this terminal), [active] (focused in the window), [private],
      [cold] (restored, not yet opened). Use the #id or a title with --panel.

  koden terminal read [--panel P] [--lines N] [--raw]
      Last N lines (default 200, max 5000) of the pane, screen plus scrollback,
      ANSI stripped and obvious secrets redacted. --raw keeps escape codes.
      Reading your own pane returns what a human sees, including the line
      you are typing on.

  koden terminal type <text> [--panel P]
      Types the text, no Enter. Multi-line text is collapsed to one line.

  koden terminal press <key> [--panel P]
      enter, escape, ctrl-c, ctrl-d, ctrl-l, ctrl-z, tab, backspace, up, down,
      left, right.

  koden terminal run <text> [--panel P]
      Types the text and presses Enter. Into a shell the line passes a safety
      filter; into an agent or TUI it is delivered as a paste plus Enter.

Targets
  --panel accepts a pane id (7 or #7) or a title fragment. Exact title beats
  case-insensitive beats substring; an ambiguous fragment is an error listing
  the candidates. Without --panel the calling terminal is the target.

Read before you type: whatever owns the pane's PTY gets the keystrokes.
Examples
  koden terminal read --lines 40
  koden terminal read --panel worker --json
  koden terminal run \"pnpm test\" --panel api
  koden terminal type -- --help";

const HELP_LAYOUT: &str = "\
koden tab / pane: build layout in the window.

  koden tab open <kind> [--cwd DIR] [--title T]
      kind: terminal | note | tasks | board. Opens in the active space and
      becomes the active tab. --cwd (terminal only) starts the shell there;
      relative paths resolve against the calling terminal's cwd.

  koden pane split <kind> --dir <side> [--title T]
      kind: terminal | note | tasks. Splits the focused pane of the ACTIVE tab
      (the one the user is looking at), placing the new pane on that side
      (left | right | up | down). The new pane takes focus, so sequential
      splits compose layouts. Only terminal tabs hold splits.

Examples
  koden tab open terminal --cwd ../api --title api
  koden pane split tasks --dir right
  koden pane split note --dir down --title scratch";

const HELP_SPACE: &str = "\
koden space: the header tab groups.

  koden space list
      id, name, tab count; the active space is marked with *.

  koden space new <name> [--root DIR]
      Creates a space with one terminal tab and switches to it. --root sets
      the space's folder (default: the calling terminal's cwd). Names need
      not be unique; the id in 'space list' is.

Examples
  koden space new \"review\" --root ../worktrees/review";

const HELP_NOTIFY: &str = "\
koden notify <message>

  Raises an in-app notification through Koden's agent router, attributed to
  the calling terminal: a toast when the window is focused, an OS notification
  when it is not, plus a bell entry either way. Use it to hand control back to
  the user (\"tests green, ready for review\") instead of waiting silently.

Example
  koden notify \"build finished, 2 warnings\"";

const HELP_PING: &str = "\
koden ping

  Full round trip through the socket, the Rust bridge and the window. Prints
  the instance version, pid and endpoint. Use it to check the link before
  scripting anything.";

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn invoke(s: &[&str]) -> Invocation {
        match parse(&argv(s)) {
            Parsed::Invoke(i) => i,
            other => panic!("expected invocation, got {other:?}"),
        }
    }

    fn usage(s: &[&str]) -> String {
        match parse(&argv(s)) {
            Parsed::Usage(m) => m,
            other => panic!("expected usage error, got {other:?}"),
        }
    }

    #[test]
    fn help_variants() {
        assert!(matches!(parse(&argv(&[])), Parsed::Help(_)));
        assert!(matches!(parse(&argv(&["--help"])), Parsed::Help(_)));
        assert!(matches!(parse(&argv(&["-h"])), Parsed::Help(_)));
        assert!(matches!(parse(&argv(&["help"])), Parsed::Help(_)));
        let Parsed::Help(t) = parse(&argv(&["terminal", "--help"])) else {
            panic!()
        };
        assert!(t.contains("koden terminal read"));
        let Parsed::Help(t) = parse(&argv(&["help", "space"])) else {
            panic!()
        };
        assert!(t.contains("space new"));
        let Parsed::Help(t) = parse(&argv(&["terminal", "read", "-h"])) else {
            panic!()
        };
        assert!(t.contains("Read before you type"));
        // Top-level help documents every command and the warning.
        for needle in [
            "terminal list",
            "pane split",
            "space new",
            "notify",
            "ping",
            "--json",
            "Read before you type",
        ] {
            assert!(HELP_TOP.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn simple_commands() {
        let i = invoke(&["terminal", "list"]);
        assert_eq!(i.cmd, "terminal.list");
        assert!(i.args.is_empty());
        assert!(!i.json);
        assert_eq!(invoke(&["ping"]).cmd, "ping");
        assert_eq!(invoke(&["space", "list"]).cmd, "space.list");
    }

    #[test]
    fn json_flag_anywhere() {
        assert!(invoke(&["--json", "ping"]).json);
        assert!(invoke(&["ping", "--json"]).json);
        assert!(invoke(&["terminal", "--json", "list"]).json);
    }

    #[test]
    fn quoting_is_preserved_and_words_are_joined() {
        // A shell-quoted argument arrives as one element and stays intact.
        let i = invoke(&["terminal", "type", "echo \"hi there\"  ok"]);
        assert_eq!(i.args["text"], "echo \"hi there\"  ok");
        // Unquoted words are joined with single spaces.
        let i = invoke(&["terminal", "run", "pnpm", "test", "--panel", "api"]);
        assert_eq!(i.args["text"], "pnpm test");
        assert_eq!(i.args["panel"], "api");
        let i = invoke(&["notify", "build", "done"]);
        assert_eq!(i.cmd, "notify");
        assert_eq!(i.args["message"], "build done");
        let i = invoke(&["space", "new", "code", "review", "--root", "/tmp/x"]);
        assert_eq!(i.args["name"], "code review");
        assert_eq!(i.args["root"], "/tmp/x");
    }

    #[test]
    fn double_dash_passes_dashed_text_through() {
        let i = invoke(&["terminal", "type", "--", "--help", "--json"]);
        assert_eq!(i.args["text"], "--help --json");
        assert!(!i.json);
        let i = invoke(&["terminal", "run", "--panel", "7", "--", "ls", "-la"]);
        assert_eq!(i.args["text"], "ls -la");
        assert_eq!(i.args["panel"], "7");
    }

    #[test]
    fn unknown_and_misplaced_flags_are_errors() {
        assert!(usage(&["terminal", "type", "--verbose"]).contains("unknown option"));
        // Single-dash words are text, not options (only -h is special).
        assert_eq!(invoke(&["terminal", "type", "ls", "-la"]).args["text"], "ls -la");
        assert!(usage(&["space", "list", "--panel", "x"]).contains("not valid"));
        assert!(usage(&["terminal", "read", "--panel"]).contains("needs a value"));
        assert!(usage(&["terminal", "read", "--raw=1"]).contains("does not take"));
        assert!(usage(&["bogus"]).contains("unknown command"));
        assert!(usage(&["terminal", "bogus"]).contains("unknown subcommand"));
        assert!(usage(&["terminal", "list", "extra"]).contains("takes no arguments"));
    }

    #[test]
    fn read_flags_and_validation() {
        let i = invoke(&["terminal", "read", "--lines=40", "--raw", "--panel", "#3"]);
        assert_eq!(i.args["lines"], 40);
        assert_eq!(i.args["raw"], true);
        assert_eq!(i.args["panel"], "#3");
        assert!(usage(&["terminal", "read", "--lines", "0"]).contains("--lines"));
        assert!(usage(&["terminal", "read", "--lines", "9999999"]).contains("--lines"));
        assert!(usage(&["terminal", "read", "--lines", "ten"]).contains("--lines"));
        assert!(usage(&["terminal", "read", "--panel", "  "]).contains("empty"));
    }

    #[test]
    fn press_keys_are_validated_and_lowercased() {
        assert_eq!(invoke(&["terminal", "press", "Enter"]).args["key"], "enter");
        assert_eq!(invoke(&["terminal", "press", "ctrl-c"]).args["key"], "ctrl-c");
        assert!(usage(&["terminal", "press", "f13"]).contains("must be one of"));
        assert!(usage(&["terminal", "press"]).contains("exactly one"));
        assert!(usage(&["terminal", "press", "enter", "enter"]).contains("exactly one"));
    }

    #[test]
    fn layout_commands() {
        let i = invoke(&["tab", "open", "terminal", "--cwd", "../api", "--title", "api"]);
        assert_eq!(i.cmd, "tab.open");
        assert_eq!(i.args["kind"], "terminal");
        assert_eq!(i.args["cwd"], "../api");
        assert_eq!(i.args["title"], "api");
        assert!(usage(&["tab", "open", "editor"]).contains("must be one of"));
        let i = invoke(&["pane", "split", "tasks", "--dir", "Right"]);
        assert_eq!(i.cmd, "pane.split");
        assert_eq!(i.args["dir"], "right");
        assert_eq!(invoke(&["pane", "split", "note", "--dir", "bottom"]).args["dir"], "down");
        assert!(usage(&["pane", "split", "note"]).contains("--dir"));
        assert!(usage(&["pane", "split", "note", "--dir", "sideways"]).contains("--dir"));
        assert!(usage(&["pane", "split", "board", "--dir", "left"]).contains("must be one of"));
    }

    #[test]
    fn missing_text_is_a_usage_error() {
        assert!(usage(&["terminal", "type"]).contains("<text>"));
        assert!(usage(&["terminal", "type", "   "]).contains("<text>"));
        assert!(usage(&["notify"]).contains("<message>"));
        assert!(usage(&["space", "new"]).contains("<name>"));
    }
}
