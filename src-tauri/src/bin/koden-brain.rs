//! `koden-brain` — a READ-ONLY MCP server over the Koden Brain index.
//!
//! Why this exists: the brain already indexes every registered project (FTS5 +
//! BM25/RRF, tree-sitter symbol graph, freshness), and the running app injects a
//! per-project gist into agent prompts. But that gist is a one-shot TEXT BLOB —
//! an external Claude Code session cannot ask it anything. So when an agent needs
//! "where is X defined", it falls back to Grep/Glob/Read across the whole tree
//! and pays for reading files the brain had already indexed.
//!
//! This binary closes that loop. It speaks MCP over stdio, so any agent that
//! supports MCP can query the index directly instead of re-reading the folder.
//!
//! Safety: every call opens its own SQLite connection with SQLITE_OPEN_READ_ONLY.
//! The store runs in WAL, so these reads are wait-free against the running app's
//! writer and cannot block or corrupt it (store/sqlite.rs: "their own READ-ONLY
//! connections (WAL -> wait-free reads). CONCEPT §8"). There is deliberately NO
//! write path here: the Librarian owns curation, this is a reader.
//!
//! It works whether or not Koden is running — it is just a reader of the store.
//! When Koden IS running the index is live (a watcher keeps it current); when it
//! is not, the answers are as fresh as the last session.
//!
//! Wire format: line-delimited JSON-RPC 2.0 on stdin/stdout. Implemented directly
//! rather than pulling an MCP SDK — the surface is `initialize`, `tools/list`,
//! `tools/call`, and that is not worth a dependency in a binary that ships inside
//! a bundle-size-sensitive app.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use koden_lib::modules::brain::ast::ImpactDirection;
use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::registry::project_id_for_root;
use koden_lib::modules::brain::store::{
    code_impact_readonly, get_symbol_readonly, outline_for_path_readonly, projects_readonly,
    recent_activity_readonly, search_readonly, SqliteIndex,
};
use koden_lib::modules::brain::worker::index_dir;
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";
const DEFAULT_LIMIT: usize = 20;

/// Resolve the store the app writes. Mirrors `worker::brain_loop`'s
/// `app_local_data_dir()/koden/brain/index.sqlite`; Tauri's local-data dir is
/// `dirs::data_local_dir()/<bundle-identifier>`. `KODEN_BRAIN_DB` overrides it,
/// which is also how the tests and a non-default install point at their own store.
fn db_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KODEN_BRAIN_DB") {
        return Some(PathBuf::from(p));
    }
    let bundle = std::env::var("KODEN_BUNDLE_ID").unwrap_or_else(|_| "app.mrolof.koden".to_string());
    Some(dirs::data_local_dir()?.join(bundle).join("koden").join("brain").join("index.sqlite"))
}

fn tool_defs() -> Value {
    json!([
        {
            "name": "brain_search",
            "description": "Search the Koden Brain index for files relevant to a query, ranked (BM25 + path/symbol fusion). Use this INSTEAD of grepping or listing a project tree: it is already indexed, so it is far cheaper and returns ranked paths directly. Omit `project` to search every indexed project at once - useful for 'have I solved this before'. Progressive disclosure: follow up with brain_outline on a hit to see its definitions + line numbers, then Read only the relevant line range instead of the whole file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Free text. Identifiers, file names and prose all work." },
                    "project": { "type": "string", "description": "Optional project id (see brain_projects). Omit to search ALL indexed projects - useful for 'have I solved this before'." },
                    "limit": { "type": "integer", "description": "Max hits (default 20)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "brain_symbol",
            "description": "Find where a symbol is DEFINED (path, kind, line) via the tree-sitter symbol graph. Use before opening files to locate a definition without reading the tree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string" },
                    "path": { "type": "string", "description": "Any directory inside the project (defaults to the server's cwd). Resolved to the project automatically - prefer this over `project`." },
                    "project": { "type": "string", "description": "Explicit project id; overrides `path`." }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "brain_outline",
            "description": "Definition outline of ONE indexed file: every symbol (name, kind, start line) in source order. The middle step between brain_search and Read: pick a search hit, outline it, then Read only the line range you need instead of the whole file. `file` must be the root-relative path exactly as brain_search/brain_symbol return it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Root-relative file path as returned by brain_search (forward slashes)." },
                    "path": { "type": "string", "description": "Any directory inside the project (defaults to cwd). Resolved to the project automatically - prefer this over `project`." },
                    "project": { "type": "string", "description": "Explicit project id; overrides `path`." }
                },
                "required": ["file"]
            }
        },
        {
            "name": "brain_impact",
            "description": "What is affected if a symbol changes: BFS over the file-level import graph. direction=upstream (who depends on it), downstream (what it depends on), or both. Use for blast-radius questions before editing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string" },
                    "path": { "type": "string", "description": "Any directory inside the project (defaults to cwd)." },
                    "project": { "type": "string", "description": "Explicit project id; overrides `path`." },
                    "direction": { "type": "string", "enum": ["upstream", "downstream", "both"] },
                    "depth": { "type": "integer", "description": "BFS depth (default 2)." }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "brain_recent_activity",
            "description": "What was recently worked on in a project: newest-first session and files-touched events. Answers 'what was I last doing here' without reading git history.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Any directory inside the project (defaults to cwd)." },
                    "project": { "type": "string", "description": "Explicit project id; overrides `path`." },
                    "limit": { "type": "integer", "description": "Max rows (default 20)." }
                },
                "required": []
            }
        },
        {
            "name": "brain_projects",
            "description": "List indexed projects with file counts, busiest first. Call this first to resolve the `project` id the other tools take.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

/// Resolve the `project` id a tool call should act on.
///
/// The store keys on an opaque 16-hex id, but an agent only ever knows a
/// directory — so an explicit `project` wins, otherwise we derive the id from
/// `path` (default: cwd) and walk UP its ancestors, taking the DEEPEST one that
/// is actually indexed. Walking up matters because an agent's cwd is usually a
/// subdirectory of the project root; deepest-first mirrors the app's own
/// longest-prefix `resolve`, so a nested project beats its parent.
fn resolve_project(args: &Value, db: &std::path::Path) -> Result<String, String> {
    if let Some(p) = args.get("project").and_then(|v| v.as_str()) {
        if !p.trim().is_empty() {
            return Ok(p.to_string());
        }
    }
    let start = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => std::env::current_dir().map_err(|e| format!("no cwd: {e}"))?,
    };
    let known: std::collections::HashSet<String> = projects_readonly(db)
        .map_err(|e| format!("cannot list projects: {e}"))?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let mut cur: Option<&std::path::Path> = Some(start.as_path());
    while let Some(dir) = cur {
        let id = project_id_for_root(dir);
        if known.contains(&id) {
            return Ok(id);
        }
        cur = dir.parent();
    }
    Err(format!(
        "{} is not inside any indexed project. Call brain_projects to list them, or pass an explicit `project`.",
        start.display()
    ))
}

fn text_result(s: String) -> Value {
    json!({ "content": [{ "type": "text", "text": s }] })
}

fn error_result(s: String) -> Value {
    json!({ "content": [{ "type": "text", "text": s }], "isError": true })
}

fn call_tool(name: &str, args: &Value) -> Value {
    let Some(db) = db_path() else {
        return error_result("cannot resolve the Koden brain store path".into());
    };
    if !db.exists() {
        return error_result(format!(
            "no Koden brain index at {}. Open Koden once to build it, or set KODEN_BRAIN_DB.",
            db.display()
        ));
    }
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let n = |k: &str, d: usize| args.get(k).and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(d);

    match name {
        "brain_projects" => match projects_readonly(&db) {
            Ok(rows) if rows.is_empty() => text_result("no indexed projects".into()),
            Ok(rows) => text_result(
                rows.iter()
                    .map(|(p, c)| format!("{p}\t{c} files"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(e) => error_result(format!("projects failed: {e}")),
        },
        "brain_search" => {
            let q = s("query");
            if q.trim().is_empty() {
                return error_result("query is required".into());
            }
            let project = args.get("project").and_then(|v| v.as_str());
            match search_readonly(&db, project, &q, n("limit", DEFAULT_LIMIT)) {
                Ok(hits) if hits.is_empty() => text_result(format!("no hits for {q:?}")),
                Ok(hits) => text_result(
                    hits.iter()
                        .map(|h| format!("{:.3}\t{}\t{}", h.score, h.project, h.path))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Err(e) => error_result(format!("search failed: {e}")),
            }
        }
        "brain_symbol" => {
            let project = match resolve_project(args, &db) {
                Ok(p) => p,
                Err(e) => return error_result(e),
            };
            match get_symbol_readonly(&db, &project, &s("symbol")) {
                Ok(v) if v.is_empty() => {
                    text_result(format!("no definition found for {:?}", s("symbol")))
                }
                Ok(v) => text_result(
                    v.iter()
                        .map(|i| format!("{}:{}\t{}\t{}", i.path, i.start_line, i.kind, i.name))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Err(e) => error_result(format!("symbol lookup failed: {e}")),
            }
        }
        "brain_outline" => {
            // Stored paths are root-relative + forward-slash; forgive the two
            // agent slips that cost a useless round-trip: backslashes and `./`.
            let file = s("file").replace('\\', "/");
            let file = file.strip_prefix("./").unwrap_or(&file);
            if file.trim().is_empty() {
                return error_result("file is required".into());
            }
            let project = match resolve_project(args, &db) {
                Ok(p) => p,
                Err(e) => return error_result(e),
            };
            match outline_for_path_readonly(&db, &project, file) {
                Ok(v) if v.is_empty() => text_result(format!(
                    "no indexed symbols in {file:?} - either the path is not root-relative (pass it exactly as brain_search returned it) or the file has no extractable definitions"
                )),
                Ok(v) => text_result(
                    v.iter()
                        .map(|i| format!("{}\t{}\t{}", i.start_line, i.kind, i.name))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Err(e) => error_result(format!("outline failed: {e}")),
            }
        }
        "brain_impact" => {
            let dir = match s("direction").as_str() {
                "downstream" => ImpactDirection::Downstream,
                "both" => ImpactDirection::Both,
                _ => ImpactDirection::Upstream,
            };
            let project = match resolve_project(args, &db) {
                Ok(p) => p,
                Err(e) => return error_result(e),
            };
            // exclude_tests=false: an agent asking "what breaks if I change this"
            // wants the tests that cover it, not a view with them filtered out.
            match code_impact_readonly(
                &db,
                &project,
                &s("symbol"),
                n("depth", 2),
                dir,
                50,
                false,
            ) {
                Ok(v) => {
                    let body = serde_json::to_string_pretty(&v)
                        .unwrap_or_else(|_| "<unserializable>".into());
                    text_result(body)
                }
                Err(e) => error_result(format!("impact failed: {e}")),
            }
        }
        "brain_recent_activity" => {
            let project = match resolve_project(args, &db) {
                Ok(p) => p,
                Err(e) => return error_result(e),
            };
            match recent_activity_readonly(&db, &project, n("limit", DEFAULT_LIMIT)) {
                Ok(v) if v.is_empty() => text_result("no recorded activity".into()),
                Ok(v) => text_result(
                    v.iter()
                        .map(|r| format!("{}\t{}\t{}", r.ts_ms, r.kind, r.payload_redacted))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Err(e) => error_result(format!("activity failed: {e}")),
            }
        }
        other => error_result(format!("unknown tool {other}")),
    }
}

/// Handle one JSON-RPC request. `None` = a notification (no reply is sent).
fn handle(req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // Notifications carry no id and MUST NOT be answered.
    id.as_ref()?;
    let result = match method {
        "initialize" => json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "koden-brain", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({ "tools": tool_defs() }),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            call_tool(name, &args)
        }
        "ping" => json!({}),
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not found: {method}") }
            }))
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

// ---------------------------------------------------------------------------
// `koden-brain daemon` — headless indexer for always-on hosts (M2.7-A2).
//
// The GUI app's worker loop is the writer on a desktop; on a server there is
// no GUI, so this subcommand drives the SAME proven pipeline (`index_dir` →
// walk → secrets redact → FTS → reconcile, exactly what tests/brain_sandbox.rs
// exercises) on a polling loop. Polling over a filesystem watcher on purpose:
// one cycle over an already-indexed tree is cheap (blake3 skip), and a watcher
// across a whole home tree is the complex path with the same outcome.
// Single-writer by deployment: run ONE daemon per store (systemd unit);
// the MCP server stays a read-only WAL reader beside it.

/// Directories never worth descending into while looking for project roots.
const DISCOVER_SKIP: &[&str] = &[
    "node_modules", "target", "dist", "build", ".venv", "venv", "__pycache__",
    ".next", ".cache", "vendor",
];

fn is_project_root(dir: &std::path::Path) -> bool {
    dir.join(".git").exists() || dir.join(".koden-memory").is_dir() || dir.join(".memory").is_dir()
}

/// Find project roots under `root`, up to `max_depth` levels down. A project
/// root (marker: .git / .koden-memory / .memory) is collected and NOT descended
/// into — nested repos inside it are the project walk's business, not ours.
fn discover_projects(root: &std::path::Path, max_depth: usize, out: &mut Vec<PathBuf>) {
    if max_depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || DISCOVER_SKIP.contains(&name.as_ref()) {
            continue;
        }
        if is_project_root(&p) {
            out.push(p);
        } else {
            discover_projects(&p, max_depth - 1, out);
        }
    }
}

fn daemon_main(rest: &[String]) -> ! {
    let mut root: Option<PathBuf> = None;
    let mut interval_secs: u64 = 300;
    let mut once = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--root" => {
                i += 1;
                root = rest.get(i).map(PathBuf::from);
            }
            "--interval" => {
                i += 1;
                interval_secs = rest.get(i).and_then(|s| s.parse().ok()).unwrap_or(300);
            }
            "--once" => once = true,
            other => {
                eprintln!("koden-brain daemon: unknown arg {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let root = root
        .or_else(|| dirs::home_dir().map(|h| h.join("Snorlax")))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| {
            eprintln!("koden-brain daemon: pass --root <dir> (default ~/Snorlax not found)");
            std::process::exit(2);
        });
    let Some(db) = db_path() else {
        eprintln!("koden-brain daemon: cannot resolve store path");
        std::process::exit(2);
    };
    if let Some(parent) = db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let idx = match SqliteIndex::open(&db) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("koden-brain daemon: cannot open {}: {e}", db.display());
            std::process::exit(1);
        }
    };
    eprintln!(
        "koden-brain daemon: root={} store={} interval={}s",
        root.display(),
        db.display(),
        interval_secs
    );
    loop {
        let t0 = std::time::Instant::now();
        let mut projects = Vec::new();
        discover_projects(&root, 4, &mut projects);
        projects.sort();
        let (mut indexed, mut pruned, mut notes) = (0usize, 0usize, 0usize);
        for p in &projects {
            let id = project_id_for_root(p);
            let stats = index_dir(&idx, &id, p);
            indexed += stats.indexed;
            pruned += stats.pruned;
            notes += scan_project_memory(&idx, &id, p);
        }
        eprintln!(
            "cycle: {} projects, {} files indexed, {} pruned, {} memory notes, {:.1}s",
            projects.len(),
            indexed,
            pruned,
            notes,
            t0.elapsed().as_secs_f32()
        );
        if once {
            std::process::exit(0);
        }
        std::thread::sleep(std::time::Duration::from_secs(interval_secs));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("daemon") {
        daemon_main(&args[2..]);
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        // A malformed line must not kill the server: reply with a parse error and
        // keep serving, the same fail-open posture the rest of the brain takes.
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&req),
            Err(e) => Some(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            })),
        };
        if let Some(reply) = reply {
            if writeln!(out, "{reply}").is_err() || out.flush().is_err() {
                break; // client hung up
            }
        }
    }
}

#[cfg(test)]
mod daemon_tests {
    use super::discover_projects;

    fn mk(root: &std::path::Path, rel: &str, dir: bool) {
        let p = root.join(rel);
        if dir {
            std::fs::create_dir_all(p).unwrap();
        } else {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
    }

    #[test]
    fn discovery_finds_markers_prunes_projects_and_skips_junk() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        mk(r, "Products/app/.git/HEAD", false); // repo at depth 2
        mk(r, "Products/app/sub/.git/HEAD", false); // nested repo: pruned, not listed
        mk(r, "Scripts/TT/Source/prov/.memory", true); // marker at depth 4
        mk(r, "node_modules/x/.git/HEAD", false); // junk dir: skipped
        mk(r, "Deep/a/b/c/d/.git/HEAD", false); // depth 5: beyond max_depth
        mk(r, "Plain/notes.txt", false); // no marker anywhere

        let mut found = Vec::new();
        discover_projects(r, 4, &mut found);
        found.sort();
        assert_eq!(
            found,
            vec![r.join("Products/app"), r.join("Scripts/TT/Source/prov")]
        );
    }
}
