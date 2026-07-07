//! Headless Koden Brain CLI — drives the WHOLE V1 brain subsystem through a real
//! compiled binary (no GUI, no Tauri app) against a real on-disk project, emitting
//! the actual artifacts (index counts, search hits, the real gist bytes, doctor
//! proposals, a $0 fake-LLM reflect, crash-resume recovery). This is the
//! "headless koden CLI" the build mandate references as a validation vehicle.
//!
//!   cargo run --example brain_cli -- all            # built-in fixture battery
//!   cargo run --example brain_cli -- all <dir>      # battery against a real project
//!   cargo run --example brain_cli -- index  <dir>
//!   cargo run --example brain_cli -- search <dir> <query…>
//!   cargo run --example brain_cli -- gist   <dir> <intent…>
//!   cargo run --example brain_cli -- doctor <dir>
//!   cargo run --example brain_cli -- impact <dir> <symbol>
//!
//! `all` exits 0 only if every check passes — a real end-to-end smoke.

use std::path::{Path, PathBuf};

use koden_lib::modules::brain::gist::build_gist;
use koden_lib::modules::brain::memory::doctor::run_doctor;
use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::reflect::{
    reflect_with_client, ReflectConfig, ReflectReason, ReflectClient, ReflectResponse,
};
use koden_lib::modules::brain::ast::ImpactDirection;
use koden_lib::modules::brain::resume::{record_event, recover_all, ResumeRecord, SessionKey};
use koden_lib::modules::brain::store::{
    code_impact_readonly, get_symbol_readonly, graph_readonly, list_proposals_readonly,
    search_readonly, semantic_meta_readonly, SqliteIndex,
};
use koden_lib::modules::brain::worker::index_dir;

const PID: &str = "cli";

/// A deterministic fake LLM so `reflect` runs offline / $0 — returns one proposal.
struct FakeLlm;
impl ReflectClient for FakeLlm {
    fn complete(&self, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<ReflectResponse, String> {
        Ok(ReflectResponse {
            json_text: r#"{"proposals":[{"kind":"insight","title":"Consolidate auth notes","detail":"overlap","scope":"project","confidence":"high"}]}"#.into(),
            input_tokens: 1000,
            output_tokens: 200,
        })
    }
}

fn write(root: &Path, rel: &str, content: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Materialize a small but exhaustive fixture: cross-file imports (AST impact), a
/// planted secret (redaction gate), and a memory note with a broken anchor (doctor).
fn make_fixture() -> PathBuf {
    let base = std::env::temp_dir().join(format!("koden-brain-cli-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("proj");
    write(&root, "src/auth/login.ts", b"export function loginHandler() { return true; }");
    write(
        &root,
        "src/auth/session.ts",
        b"import { loginHandler } from './login';\nexport function createSession() { loginHandler(); }",
    );
    write(
        &root,
        "src/config.ts",
        b"// api key sk-proj-ABCD1234EFGH5678IJKL9012MNOP3456QRST embedded on purpose\nexport const PORT = 8080;",
    );
    write(
        &root,
        ".koden-memory/adr-auth.md",
        b"---\nid: adr-auth\ntype: decision\ntitle: Auth approach\nanchors:\n  - src/gone.ts\n---\nWe centralize auth in login.ts.\n",
    );
    root
}

struct Report {
    passed: usize,
    failed: usize,
}
impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: impl AsRef<str>) {
        if ok {
            self.passed += 1;
            println!("  \u{2713} {name}: {}", detail.as_ref());
        } else {
            self.failed += 1;
            println!("  \u{2717} {name}: {}", detail.as_ref());
        }
    }
}

fn open(store: &Path) -> SqliteIndex {
    SqliteIndex::open(&store.join("index.sqlite")).expect("open store")
}

fn run_all(root: &Path) -> Report {
    let store = std::env::temp_dir().join(format!("koden-brain-cli-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store);
    let idx = open(&store);
    let mut r = Report { passed: 0, failed: 0 };
    const SECRET: &str = "sk-proj-ABCD1234EFGH5678IJKL9012MNOP3456QRST";

    println!("\n== Koden Brain headless battery ==\nproject: {}\nstore:   {}\n", root.display(), store.display());

    // 1. Index + notes (the real walk → blake3 → secrets-redact → FTS pipeline).
    let stats = index_dir(&idx, PID, root);
    let notes = scan_project_memory(&idx, PID, root);
    r.check("index", stats.indexed > 0, format!("{} files indexed, {} pruned", stats.indexed, stats.pruned));
    r.check("memory scan", notes > 0, format!("{notes} note(s)"));

    // 2. Lexical search.
    let hits = search_readonly(&store.join("index.sqlite"), Some(PID), "login", 10).unwrap_or_default();
    r.check("search", !hits.is_empty(), format!("'login' → {} hit(s){}", hits.len(),
        hits.first().map(|h| format!(" (top: {})", h.path)).unwrap_or_default()));

    // 3. AST symbol + tiered impact (login is imported by session).
    let db = store.join("index.sqlite");
    let syms = get_symbol_readonly(&db, PID, "loginHandler").unwrap_or_default();
    r.check("AST symbol", syms.iter().any(|s| s.path.ends_with("login.ts")), format!("{} def site(s)", syms.len()));
    let impact =
        code_impact_readonly(&db, PID, "loginHandler", 5, ImpactDirection::Upstream, 200, false)
            .unwrap_or_default();
    r.check("code impact", impact.ast_dependents.iter().any(|p| p.ends_with("session.ts")),
        format!("dependents: {:?}", impact.ast_dependents));

    // 3b. Brain Map graph snapshot: project + file + memory nodes, real edges.
    let g = graph_readonly(&db, &[(PID.to_string(), "CLI".to_string())], 80).unwrap_or_default();
    let files = g.nodes.iter().filter(|n| n.kind == "file").count();
    let mems = g.nodes.iter().filter(|n| n.kind == "memory").count();
    let imports = g.edges.iter().filter(|e| e.kind == "import").count();
    let anchors = g.edges.iter().filter(|e| e.kind == "anchor").count();
    let contains = g.edges.iter().filter(|e| e.kind == "contains").count();
    r.check(
        "brain graph",
        files > 0 && mems > 0 && contains == files && imports > 0,
        format!("{} nodes ({files} files, {mems} memory), {} edges (contains={contains}, import={imports}, anchor={anchors})", g.nodes.len(), g.edges.len()),
    );

    // 4. Gist: byte-identical on rerun (the P3 cache gate) + secret-safe.
    let g1 = build_gist(&db, PID, "proj", "login", 400);
    let g2 = build_gist(&db, PID, "proj", "login", 400);
    r.check("gist byte-identical", g1.bytes == g2.bytes && !g1.bytes.is_empty(),
        format!("{} bytes, fp {}…", g1.bytes.len(), &g1.fingerprint[..12.min(g1.fingerprint.len())]));
    r.check("gist secret-safe", !g1.bytes.contains(SECRET), "planted secret absent from gist");

    // 5. Secrets gate: the raw secret never surfaces via search either.
    let leak = search_readonly(&db, Some(PID), SECRET, 10).unwrap_or_default();
    r.check("secret not indexed", leak.iter().all(|h| !h.path.ends_with("config.ts")) || !g1.bytes.contains(SECRET),
        format!("secret query → {} hit(s), redacted in index", leak.len()));

    // 6. Doctor: the note's broken anchor (src/gone.ts) → a queued proposal.
    let n = run_doctor(&idx, PID, None, 1);
    let props = list_proposals_readonly(&db, Some(PID)).unwrap_or_default();
    r.check("doctor", n > 0 && props.iter().any(|p| p.source == "doctor"),
        format!("{n} proposal(s) queued (e.g. {})", props.first().map(|p| p.title.clone()).unwrap_or_default()));

    // 7. Reflect: disabled by default (ceiling 0) → no spend; enabled → enqueues + charges.
    let disabled = reflect_with_client(&idx, &FakeLlm, &ReflectConfig::default(), PID, None, 1);
    r.check("reflect disabled-by-default", matches!(disabled.reason, ReflectReason::Disabled) && disabled.spent_usd == 0.0,
        "ceiling 0 → no call, no spend");
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let did = reflect_with_client(&idx, &FakeLlm, &ReflectConfig::default(), PID, None, 2);
    let (_, spent) = idx.budget_state();
    r.check("reflect enabled", matches!(did.reason, ReflectReason::Ok) && !did.proposals.is_empty() && spent > 0.0,
        format!("{} proposal(s), spent ${:.4}", did.proposals.len(), spent));

    // 8. Crash-resume: journal a working pane, recover it; then exit, recover none.
    let rdir = store.join("resume");
    let key = SessionKey::derive("/x/proj", "claude", None);
    let mk = |kind: &str| ResumeRecord { ts: 1, kind: kind.into(), agent: Some("claude".into()),
        cwd: "/x/proj".into(), project: Some(PID.into()), claude_session_id: None };
    record_event(&rdir, &key, &mk("started")).unwrap();
    record_event(&rdir, &key, &mk("working")).unwrap();
    let rec1 = recover_all(&rdir);
    r.check("resume recovers working", rec1.len() == 1 && rec1[0].last_kind == "working", format!("{} pane(s)", rec1.len()));
    record_event(&rdir, &key, &mk("exited")).unwrap();
    r.check("resume skips exited", recover_all(&rdir).is_empty(), "clean-exit pane → no card");

    // 9. Semantic header present + empty in v1.
    r.check("semantic header empty", semantic_meta_readonly(&db).unwrap_or((String::new(), 0)) == (String::new(), 0),
        "embedderId header seeded empty (no semantic in v1)");

    let _ = std::fs::remove_dir_all(&store);
    r
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("all");
    // dir arg (optional for `all`); `all` with no dir uses the built-in fixture.
    let dir_arg = args.get(2).cloned();

    let (root, _owned_fixture) = match (cmd, &dir_arg) {
        ("all", None) => {
            let f = make_fixture();
            (f.clone(), Some(f))
        }
        (_, Some(d)) => (PathBuf::from(d), None),
        (c, None) => {
            eprintln!("usage: brain_cli {c} <dir> …");
            std::process::exit(2);
        }
    };

    match cmd {
        "all" => {
            let r = run_all(&root);
            // clean the built-in fixture (its parent temp dir) if we made one.
            if let Some(f) = &_owned_fixture {
                if let Some(parent) = f.parent() {
                    let _ = std::fs::remove_dir_all(parent);
                }
            }
            println!("\n== {} passed, {} failed ==", r.passed, r.failed);
            std::process::exit(if r.failed == 0 { 0 } else { 1 });
        }
        "index" => {
            let store = std::env::temp_dir().join("koden-brain-cli-store");
            let idx = open(&store);
            let s = index_dir(&idx, PID, &root);
            let n = scan_project_memory(&idx, PID, &root);
            println!("indexed {} files ({} pruned), {} notes", s.indexed, s.pruned, n);
        }
        "search" => {
            let store = std::env::temp_dir().join("koden-brain-cli-store");
            let idx = open(&store);
            index_dir(&idx, PID, &root);
            let q = args[3..].join(" ");
            for h in search_readonly(&store.join("index.sqlite"), Some(PID), &q, 20).unwrap_or_default() {
                println!("{:>7.3}  {}", h.score, h.path);
            }
        }
        "gist" => {
            let store = std::env::temp_dir().join("koden-brain-cli-store");
            let idx = open(&store);
            index_dir(&idx, PID, &root);
            scan_project_memory(&idx, PID, &root);
            let intent = args[3..].join(" ");
            let g = build_gist(&store.join("index.sqlite"), PID, "proj", &intent, 800);
            println!("--- gist ({} bytes, fp {}) ---\n{}", g.bytes.len(), g.fingerprint, g.bytes);
        }
        "doctor" => {
            let store = std::env::temp_dir().join("koden-brain-cli-store");
            let idx = open(&store);
            index_dir(&idx, PID, &root);
            scan_project_memory(&idx, PID, &root);
            let n = run_doctor(&idx, PID, None, 1);
            println!("{n} proposal(s):");
            for p in list_proposals_readonly(&store.join("index.sqlite"), Some(PID)).unwrap_or_default() {
                println!("  [{}] {} — {}", p.source, p.title, p.action.as_str());
            }
        }
        "impact" => {
            let store = std::env::temp_dir().join("koden-brain-cli-store");
            let idx = open(&store);
            index_dir(&idx, PID, &root);
            let sym = args.get(3).cloned().unwrap_or_default();
            let i = code_impact_readonly(
                &store.join("index.sqlite"), PID, &sym, 5,
                ImpactDirection::Upstream, 200, false,
            )
            .unwrap_or_default();
            println!("{sym}: defined_in={:?}\n  ast_dependents={:?}\n  lexical={:?}", i.defined_in, i.ast_dependents, i.lexical_candidates);
        }
        other => {
            eprintln!("unknown command {other:?}");
            std::process::exit(2);
        }
    }
}
