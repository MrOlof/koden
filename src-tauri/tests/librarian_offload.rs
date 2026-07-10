//! LIB-DESIGN-01 regression: the librarian round's provider call must NOT block the
//! brain worker thread's incremental indexing.
//!
//! Drives the REAL offload seam end to end — [reflect_prepare_with_client] (the
//! worker-side prepare + budget reserve) → [ReflectPending::call] (off-thread network)
//! → [reflect_finish] (worker-side reconcile + enqueue) — the exact split
//! `worker.rs::run_librarian_rounds` uses. It proves that while a round's provider call
//! is in flight, the worker thread still indexes a freshly-changed file to
//! searchability (via the real `index_changed`) — the freshness the OLD inline
//! dispatch stalled for the full call duration.
//!
//! Negative control: the SAME gated client used SYNCHRONOUSLY through
//! [reflect_with_client] (the old worker shape) keeps its caller thread occupied for
//! the entire call — demonstrating the client genuinely blocks and that offloading the
//! call is precisely what frees the worker.
//!
//! $0 real spend (a free-rate fake client). Fully deterministic: the client blocks on
//! an explicit channel handshake — no sleeps, no timing races decide the assertions.
//!
//! Run:  cargo test --test librarian_offload -- --nocapture --test-threads=1

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::reflect::{
    reflect_finish, reflect_prepare_with_client, reflect_with_client, ReflectClient,
    ReflectConfig, ReflectDispatch, ReflectReason, ReflectResponse,
};
use koden_lib::modules::brain::store::{budget_state_readonly, search_readonly, SqliteIndex};
use koden_lib::modules::brain::worker::{
    index_changed, index_dir, librarian_round_begin, LibrarianAuto,
};

const PID: &str = "proj";

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn tmp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("koden-liboffload-{}-{}", tag, std::process::id()))
}

/// Free-rate config: exercises the REAL reserve/reconcile ledger path at $0 real spend.
fn free_cfg() -> ReflectConfig {
    ReflectConfig { in_rate: 0.0, out_rate: 0.0, ..ReflectConfig::default() }
}

/// A tiny project with one indexed file plus a memory note (so the reflect digest is
/// non-empty — EmptyCorpus would short-circuit the round before any call). Indexed
/// into a fresh store with a fake budget ceiling and free rates.
fn setup(tag: &str) -> (PathBuf, PathBuf, SqliteIndex) {
    let root = tmp(&format!("{tag}-fix"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".koden-memory")).unwrap();
    std::fs::write(root.join("main.ts"), "export function login() { return 1; }\n").unwrap();
    std::fs::write(
        root.join(".koden-memory").join("adr.md"),
        "---\nid: adr\ntype: decision\ntitle: Auth centralized in login\n\
         status: active\nanchors: [\"main.ts\"]\n---\nLogin handling is centralized.\n",
    )
    .unwrap();

    let store = tmp(&format!("{tag}-store"));
    let _ = std::fs::remove_dir_all(&store);
    std::fs::create_dir_all(&store).unwrap();
    let db = store.join("index.sqlite");
    let idx = SqliteIndex::open_with_recovery(&db).expect("open store");
    index_dir(&idx, PID, &root);
    scan_project_memory(&idx, PID, &root);
    idx.set_budget_ceiling(1.0, now_ms()).expect("set ceiling"); // fake ceiling; free rates => $0
    (root, db, idx)
}

/// A ReflectClient whose `complete()` announces entry on `entered`, then BLOCKS until
/// the test sends on `release` — a deterministic stand-in for provider latency. On
/// release it returns one insight proposal. `Send` (Sender/Receiver/String are all
/// Send), so it can be boxed as `dyn ReflectClient + Send` and moved to a thread.
struct GatedClient {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    title: String,
}
impl ReflectClient for GatedClient {
    fn complete(&self, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<ReflectResponse, String> {
        let _ = self.entered.send(()); // announce: the call is now in flight
        let _ = self.release.recv(); // block until the test releases it
        let j = format!(
            r#"{{"proposals":[{{"kind":"insight","title":"{}","detail":"regr","scope":"project","confidence":"high"}}]}}"#,
            self.title
        );
        Ok(ReflectResponse { json_text: j, input_tokens: 10, output_tokens: 5 })
    }
}

/// Count budget-ledger rows by status via a read-only connection (reserved-orphan +
/// spent-row invariants, exactly as the load gauntlet checks them).
fn ledger_counts(db: &Path) -> (i64, i64) {
    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .expect("open ledger ro");
    conn.query_row(
        "SELECT SUM(status='reserved'), SUM(status='spent') FROM brain_budget_ledger",
        [],
        |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
    )
    .unwrap_or((-1, -1))
}

/// THE FIX: with the provider call offloaded, the worker thread indexes a
/// freshly-changed file to searchability WHILE the round is still in flight.
#[test]
fn offloaded_round_does_not_stall_worker_indexing() {
    let (root, db, idx) = setup("fix");
    let cfg = free_cfg();

    // Begin a round on the "worker" (this thread) through the REAL gate.
    let mut st = LibrarianAuto { dirty: true, boundary: true, ..LibrarianAuto::default() };
    let prev = librarian_round_begin(&mut st, now_ms()).expect("a dirty+boundary project is due");
    assert_eq!(prev, None, "first round has no prior digest");

    // Prepare on the worker thread: this places the durable budget reservation
    // (front half runs here — index access stays on the single writer thread).
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let client: Box<dyn ReflectClient + Send> =
        Box::new(GatedClient { entered: entered_tx, release: release_rx, title: "regr-insight".into() });
    let pending = match reflect_prepare_with_client(&idx, client, &cfg, PID, None, now_ms(), prev.as_deref()) {
        ReflectDispatch::Pending(p) => p,
        ReflectDispatch::Ready(o, _) => panic!("expected a Pending dispatch, got Ready({:?})", o.reason),
    };
    st.in_flight = true; // as run_librarian_rounds does after a Pending dispatch
    let (reserved_before, spent_rows_before) = ledger_counts(&db);
    assert_eq!(reserved_before, 1, "prepare must place the reservation on the worker thread");
    assert_eq!(spent_rows_before, 0, "nothing reconciled before the call returns");

    // Offload the provider call to a helper thread (the only off-thread work).
    let (done_tx, done_rx) = mpsc::channel();
    let net = std::thread::spawn(move || {
        let (result, finish) = pending.call();
        done_tx.send((result, finish)).unwrap();
    });

    // Wait until the call is genuinely in flight (the client entered complete()).
    entered_rx.recv_timeout(Duration::from_secs(10)).expect("round entered the provider call");

    // ---- REGRESSION ASSERTION -------------------------------------------------
    // The worker thread (this thread) is free: index a brand-new file to
    // searchability while the provider call is STILL blocked.
    let needle = "zephyrquokka";
    std::fs::write(root.join("fresh.ts"), format!("const {needle} = 1;\n")).unwrap();
    let t0 = Instant::now();
    let stats = index_changed(&idx, PID, &root, &[root.join("fresh.ts")]);
    let index_ms = t0.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(stats.indexed, 1, "worker indexed the new file WHILE the round was in flight");
    let hits = search_readonly(&db, Some(PID), needle, 3).unwrap_or_default();
    assert!(!hits.is_empty(), "fresh file searchable while the provider call is still blocked");
    // And the call must still be blocked — proving indexing did not wait on it.
    assert!(done_rx.try_recv().is_err(), "provider call returned too early; it must still be gated");
    eprintln!(
        "REGR offloaded worker indexed fresh file in {index_ms:.1}ms with the provider call in flight"
    );

    // ---- Release + finish on the worker (single writer) -----------------------
    release_tx.send(()).unwrap();
    let (result, finish) = done_rx.recv_timeout(Duration::from_secs(10)).expect("provider call completed");
    net.join().unwrap();
    let expected_dh = finish.digest_hash.clone();
    let (outcome, digest_hash) = reflect_finish(&idx, PID, finish, result, now_ms());
    st.in_flight = false;
    assert!(matches!(outcome.reason, ReflectReason::Ok), "round Ok, got {:?}", outcome.reason);
    assert_eq!(outcome.proposals.len(), 1, "the insight proposal is enqueued on finish");
    assert_eq!(digest_hash.as_deref(), Some(expected_dh.as_str()), "digest hash pinned for the delta gate");

    // Budget ledger clean: reservation reconciled (no orphan), $0 at free rates.
    drop(idx);
    let (ceiling, spent) = budget_state_readonly(&db).expect("read budget");
    let (reserved_after, spent_rows_after) = ledger_counts(&db);
    assert_eq!(reserved_after, 0, "no orphaned reservation after finish");
    assert_eq!(spent_rows_after, 1, "the reservation reconciled to a spent row");
    assert!(spent <= ceiling + 1e-9, "spent within ceiling");
    assert_eq!(spent, 0.0, "a free-rate round spends $0");

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// NEGATIVE CONTROL: the SAME gated client, used SYNCHRONOUSLY through the old inline
/// path ([reflect_with_client]), occupies its caller thread for the WHOLE call — the
/// exact stall the offload removes. If this were the worker thread it could not index
/// anything until the call returned.
#[test]
fn inline_round_occupies_its_thread_for_the_whole_call() {
    let (root, _db, idx) = setup("ctrl");
    let cfg = free_cfg();

    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let client: Box<dyn ReflectClient + Send> =
        Box::new(GatedClient { entered: entered_tx, release: release_rx, title: "ctrl".into() });

    let returned = Arc::new(AtomicBool::new(false));
    let returned_c = returned.clone();
    // Move the store into the thread (SqliteIndex is Send; a single owner at a time).
    let inline = std::thread::spawn(move || {
        // The OLD worker shape: the call runs inline, blocking THIS thread end to end.
        let out = reflect_with_client(&idx, client.as_ref(), &cfg, PID, None, now_ms());
        returned_c.store(true, Ordering::SeqCst);
        out.reason
    });

    // The inline call has entered the network but has NOT returned — its thread is
    // wholly occupied. On the real worker this is precisely when Fs would stall.
    entered_rx.recv_timeout(Duration::from_secs(10)).expect("inline call entered the provider call");
    assert!(
        !returned.load(Ordering::SeqCst),
        "inline dispatch occupies its thread for the entire call (the stall the offload removes)"
    );

    // Release and confirm it only completes AFTER the call returns.
    release_tx.send(()).unwrap();
    let reason = inline.join().unwrap();
    assert!(returned.load(Ordering::SeqCst));
    assert!(matches!(reason, ReflectReason::Ok), "control round Ok, got {reason:?}");

    let _ = std::fs::remove_dir_all(&root);
}
