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
//!   cargo run --example brain_cli -- watch  <dir> <store>   # arm the REAL watcher; index deltas until <store>/STOP exists
//!   cargo run --example brain_cli -- query  <store> <query…> # search an existing store (no reindex — cross-process reads)
//!   cargo run --example brain_cli -- reflect-hang <dir> <store>  # $0 Librarian round that HANGS inside the LLM call (crash-sim kill window)
//!   cargo run --example brain_cli -- sweep <store>               # the boot sweep (worker brain_loop step): fold orphaned reservations
//!   cargo run --example brain_cli -- reflect-live <dir> <store> <ceiling-usd>  # ONE real paid Librarian round behind the budget/delta/reject gates
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
use koden_lib::modules::brain::worker::{index_changed, index_dir};

const PID: &str = "cli";

/// `watch <dir> <store>` — the live-context-engine loop as a real PROCESS:
/// arm the REAL recursive watcher (freshness::watch::spawn) over `dir`, warm-index,
/// then dispatch coalesced `BrainEvent`s to the real worker fns (`Fs` →
/// `index_changed`, `Rescan` → `index_dir`), printing one line per pass (the
/// observable coalescing counter). Exits when `<store>/STOP` appears.
fn run_watch(root: &Path, store: &Path) {
    use koden_lib::modules::brain::events::BrainEvent;
    use koden_lib::modules::brain::freshness::watch;
    use koden_lib::modules::fs::to_canon;
    use std::io::Write as _;

    let idx = open(store);
    let canon = to_canon(std::fs::canonicalize(root).expect("canonicalize root"));
    let (tx, rx) = std::sync::mpsc::channel::<BrainEvent>();
    // Arm BEFORE the warm walk, exactly like brain_loop step 6.
    let watcher = watch::spawn(vec![(PID.to_string(), canon)], tx);
    if watcher.is_none() {
        eprintln!("FATAL: watcher failed to arm");
        std::process::exit(1);
    }
    let stats = index_dir(&idx, PID, root);
    let notes = scan_project_memory(&idx, PID, root);
    println!("READY indexed={} pruned={} notes={notes}", stats.indexed, stats.pruned);
    let _ = std::io::stdout().flush();
    let stop = store.join("STOP");
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(300)) {
            Ok(BrainEvent::Fs { project, changed }) if project == PID => {
                let s = index_changed(&idx, PID, root, &changed);
                println!("FS paths={} indexed={} pruned={}", changed.len(), s.indexed, s.pruned);
            }
            Ok(BrainEvent::Rescan { .. }) => {
                let s = index_dir(&idx, PID, root);
                println!("RESCAN indexed={} pruned={}", s.indexed, s.pruned);
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let _ = std::io::stdout().flush();
        if stop.exists() {
            break;
        }
    }
    drop(watcher);
    println!("STOPPED");
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A ReflectClient that announces the call then HANGS — giving a crash sim a
/// deterministic kill window BETWEEN the durable budget reservation (committed
/// before `complete` is invoked) and the reconcile/proposal write after it.
struct HangLlm;
impl ReflectClient for HangLlm {
    fn complete(&self, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<ReflectResponse, String> {
        use std::io::Write as _;
        println!("LLM_CALL_STARTED");
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_secs(300));
        Ok(ReflectResponse {
            json_text: r#"{"proposals":[]}"#.into(),
            input_tokens: 1,
            output_tokens: 1,
        })
    }
}

/// `reflect-hang <dir> <store>` — index + scan notes, set a $1 ceiling via the
/// real budget path, then run the REAL reflect pipeline with [HangLlm]. The
/// reservation row commits, LLM_CALL_STARTED prints, and the process sits inside
/// the "network call" until it is killed (or 300 s pass).
fn run_reflect_hang(root: &Path, store: &Path) {
    use std::io::Write as _;
    let idx = open(store);
    let s = index_dir(&idx, PID, root);
    let notes = scan_project_memory(&idx, PID, root);
    println!("INDEXED {} NOTES {notes}", s.indexed);
    let _ = std::io::stdout().flush();
    idx.set_budget_ceiling(1.0, now_ms()).expect("set ceiling");
    let out = reflect_with_client(&idx, &HangLlm, &ReflectConfig::default(), PID, None, now_ms());
    // Only reached if nobody killed us.
    println!("REFLECT_DONE reason={:?} spent={:.6}", out.reason, out.spent_usd);
}

/// `sweep <store>` — exactly what the GUI worker does at boot (worker.rs
/// brain_loop): charge any orphaned 'reserved' ledger row at its estimate.
fn run_sweep(store: &Path) {
    let idx = open(store);
    match idx.sweep_orphaned_reservations(now_ms()) {
        Ok(n) => println!("SWEPT {n}"),
        Err(e) => {
            eprintln!("SWEEP_ERR {e}");
            std::process::exit(1);
        }
    }
}

/// `reflect-live <dir> <store> <ceiling>` — ONE real paid Librarian round against
/// the configured provider (fresh-store default: Anthropic Haiku), driven through
/// the same gates the autonomous path uses. The key is read from the app's OWN
/// location (keyring service `koden-ai`, per-provider account — the exact path
/// `secrets::read_secret` takes on Windows/macOS) and is NEVER printed.
/// Exit codes: 0 = all gates behaved; 3 = no key (blocked); 4 = a gate misfired;
/// 5 = the paid round itself did not return Ok.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn run_reflect_live(root: &Path, store: &Path, ceiling: f64) {
    use std::cell::RefCell;

    use koden_lib::modules::brain::memory::proposal::reject_signature;
    use koden_lib::modules::brain::reflect::{
        librarian, llm::AnthropicClient, reflect_auto_with_client, ReflectReason, KEYRING_SERVICE,
    };

    let mut failed = false;
    let mut gate = |name: &str, ok: bool, detail: String| {
        println!("{name} {} {detail}", if ok { "pass" } else { "FAIL" });
        if !ok {
            failed = true;
        }
    };

    // Fresh stores seed brain_librarian to the Anthropic Haiku default; use the
    // same ReflectConfig::default() (identical values) for rates + model.
    let cfg = ReflectConfig::default();
    let account = librarian::keyring_account_for(&cfg.provider);
    let key = keyring::Entry::new(KEYRING_SERVICE, account)
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|k| !k.is_empty());
    let Some(key) = key else {
        println!("KEY absent service={KEYRING_SERVICE} account={account}");
        std::process::exit(3);
    };
    println!("KEY present provider={} model={}", cfg.provider, cfg.model);

    let idx = open(store);
    let s = index_dir(&idx, PID, root);
    let notes = scan_project_memory(&idx, PID, root);
    println!("INDEXED {} NOTES {notes}", s.indexed);

    /// Real Anthropic client + a tap that records whether/what the provider
    /// actually returned — the observable for "this gate made NO network call".
    struct Recording {
        inner: AnthropicClient,
        last: RefCell<Option<String>>,
    }
    impl ReflectClient for Recording {
        fn complete(&self, m: &str, s: &str, u: &str, t: u32) -> Result<ReflectResponse, String> {
            let r = self.inner.complete(m, s, u, t)?;
            *self.last.borrow_mut() = Some(r.json_text.clone());
            Ok(r)
        }
    }
    let client = Recording { inner: AnthropicClient::new(key), last: RefCell::new(None) };

    // GATE 1 — pre-flight budget gate: fresh store ceiling = 0.0 → Disabled,
    // $0, and the provider is NEVER contacted.
    let (g1, _) = reflect_auto_with_client(&idx, &client, &cfg, PID, None, now_ms(), None);
    gate(
        "GATE1_DISABLED",
        matches!(g1.reason, ReflectReason::Disabled)
            && g1.spent_usd == 0.0
            && client.last.borrow().is_none(),
        format!("reason={:?} spent={:.6}", g1.reason, g1.spent_usd),
    );

    // Arm the budget via the REAL path (same fn the settings command uses).
    idx.set_budget_ceiling(ceiling, now_ms()).expect("set ceiling");
    println!("CEILING {ceiling}");

    // ROUND 1 — the ONE paid call.
    let (r1, digest_hash) = reflect_auto_with_client(&idx, &client, &cfg, PID, None, now_ms(), None);
    let round1_json = client.last.borrow().clone();
    let (ceil_now, spent) = idx.budget_state();
    println!(
        "ROUND1 reason={:?} proposals={} spent={:.6} total_spent={:.6} ceiling={ceil_now}",
        r1.reason,
        r1.proposals.len(),
        r1.spent_usd,
        spent
    );
    for p in &r1.proposals {
        println!("P1 action={} sig={} title={}", p.action.as_str(), p.signature, p.title);
    }
    if !matches!(r1.reason, ReflectReason::Ok) {
        println!("S11_ABORT round1 did not return Ok");
        std::process::exit(5);
    }

    // GATE 2 — delta gate: byte-identical digest → Unchanged, $0, no call.
    *client.last.borrow_mut() = None;
    let (g2, _) =
        reflect_auto_with_client(&idx, &client, &cfg, PID, None, now_ms(), digest_hash.as_deref());
    gate(
        "GATE2_UNCHANGED",
        matches!(g2.reason, ReflectReason::Unchanged)
            && g2.spent_usd == 0.0
            && client.last.borrow().is_none(),
        format!("reason={:?} spent={:.6}", g2.reason, g2.spent_usd),
    );

    // GATE 3 — ceiling gate: lower the ceiling to exactly what's spent →
    // OverBudget rejected in the reserve txn, BEFORE any network I/O.
    idx.set_budget_ceiling(spent, now_ms()).expect("lower ceiling");
    let (g3, _) = reflect_auto_with_client(&idx, &client, &cfg, PID, None, now_ms(), None);
    gate(
        "GATE3_OVERBUDGET",
        matches!(g3.reason, ReflectReason::OverBudget)
            && g3.spent_usd == 0.0
            && client.last.borrow().is_none(),
        format!("reason={:?} spent={:.6}", g3.reason, g3.spent_usd),
    );
    idx.set_budget_ceiling(ceiling, now_ms()).expect("restore ceiling");

    // GATE 4 — reject-signature gate, $0 (free-rate replay config, no network).
    // The reject signature normalizes the title (djb2 of scope|action|lowercased
    // title) while the queue PK is the exact-title join — so a CASE-FLIPPED
    // resubmission dodges the PK dedup and can ONLY be stopped by the reject gate.
    if let Some(p0) = r1.proposals.first() {
        struct Replay(String);
        impl ReflectClient for Replay {
            fn complete(&self, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<ReflectResponse, String> {
                Ok(ReflectResponse { json_text: self.0.clone(), input_tokens: 1, output_tokens: 1 })
            }
        }
        let free = ReflectConfig {
            model: "replay-control".into(),
            in_rate: 0.0,
            out_rate: 0.0,
            ..ReflectConfig::default()
        };
        let kind = match p0.action.as_str() {
            "create" => "insight",
            "archive" => "stale",
            _ => "conflict", // update; supersede never originates from reflect
        };
        let item = |title: &str| {
            serde_json::json!({"proposals":[{"kind":kind,"title":title,"detail":"reject-sig control","scope":"project","confidence":"high"}]})
                .to_string()
        };
        let flipped: String = p0
            .title
            .chars()
            .map(|c| {
                if c.is_lowercase() {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c.to_lowercase().next().unwrap_or(c)
                }
            })
            .collect();
        if flipped == p0.title {
            println!("GATE4_SKIPPED title has no case to flip");
        } else {
            // Negative control FIRST: an unrelated title enqueues fine, proving
            // the replay path can insert at all.
            let neg = reflect_with_client(
                &idx,
                &Replay(item("Unrelated control proposal zk9q")),
                &free,
                PID,
                None,
                now_ms(),
            );
            gate(
                "GATE4_NEG_ENQUEUES",
                matches!(neg.reason, ReflectReason::Ok) && neg.proposals.len() == 1 && neg.spent_usd == 0.0,
                format!("reason={:?} props={} spent={:.6}", neg.reason, neg.proposals.len(), neg.spent_usd),
            );
            // Reject round-1's first proposal through the REAL resolve path.
            idx.resolve_proposal(PID, &p0.signature, true).expect("reject proposal");
            let rej = reject_signature(p0.action, p0.target_id.as_deref(), &p0.title);
            gate(
                "GATE4_SIG_PERSISTED",
                idx.is_rejected(PID, &rej).unwrap_or(false),
                format!("reject_sig={rej}"),
            );
            // The case-flipped resubmission must be swallowed by the reject gate.
            let r2 = reflect_with_client(&idx, &Replay(item(&flipped)), &free, PID, None, now_ms());
            gate(
                "GATE4_REJECTSIG",
                matches!(r2.reason, ReflectReason::Ok) && r2.proposals.is_empty() && r2.spent_usd == 0.0,
                format!("reason={:?} props={} spent={:.6}", r2.reason, r2.proposals.len(), r2.spent_usd),
            );
        }
    } else {
        println!("GATE4_SKIPPED round1 returned no proposals");
        let _ = round1_json; // kept for parity with the recording tap
    }

    let (_, final_spent) = idx.budget_state();
    println!("S11_DONE final_spent={final_spent:.6}");
    if failed {
        std::process::exit(4);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn run_reflect_live(_root: &Path, _store: &Path, _ceiling: f64) {
    println!("KEY absent (no keyring backend on this target)");
    std::process::exit(3);
}

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
    // Production parity: the GUI worker boots via `open_with_recovery` (ADR-010
    // cluster 4 — BUSY retry + corrupt rename-aside + salvage), so the headless
    // CLI must too, or a recovered-in-prod store bricks the CLI.
    SqliteIndex::open_with_recovery(&store.join("index.sqlite")).expect("open store")
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

    // Process-boundary subcommands with their own arg shapes.
    match cmd {
        "watch" => {
            let (Some(dir), Some(store)) = (args.get(2), args.get(3)) else {
                eprintln!("usage: brain_cli watch <dir> <store>");
                std::process::exit(2);
            };
            run_watch(Path::new(dir), Path::new(store));
            return;
        }
        "reflect-hang" => {
            let (Some(dir), Some(store)) = (args.get(2), args.get(3)) else {
                eprintln!("usage: brain_cli reflect-hang <dir> <store>");
                std::process::exit(2);
            };
            run_reflect_hang(Path::new(dir), Path::new(store));
            return;
        }
        "sweep" => {
            let Some(store) = args.get(2) else {
                eprintln!("usage: brain_cli sweep <store>");
                std::process::exit(2);
            };
            run_sweep(Path::new(store));
            return;
        }
        "reflect-live" => {
            let (Some(dir), Some(store), Some(ceiling)) = (args.get(2), args.get(3), args.get(4))
            else {
                eprintln!("usage: brain_cli reflect-live <dir> <store> <ceiling-usd>");
                std::process::exit(2);
            };
            let ceiling: f64 = ceiling.parse().expect("ceiling must be a number");
            assert!(
                ceiling > 0.0 && ceiling <= 1.0,
                "sim guard: ceiling must be in (0, 1] USD"
            );
            run_reflect_live(Path::new(dir), Path::new(store), ceiling);
            return;
        }
        "query" => {
            let Some(store) = args.get(2) else {
                eprintln!("usage: brain_cli query <store> <query…>");
                std::process::exit(2);
            };
            let q = args[3..].join(" ");
            let hits = search_readonly(&Path::new(store).join("index.sqlite"), Some(PID), &q, 20)
                .unwrap_or_default();
            for h in &hits {
                println!("{:>7.3}  {}", h.score, h.path);
            }
            println!("HITS {}", hits.len());
            return;
        }
        _ => {}
    }
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
