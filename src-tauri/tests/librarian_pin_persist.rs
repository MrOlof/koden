//! LIB-SPEND-01 — the librarian delta-gate pin must be DURABLE across a restart.
//!
//! The pin (the digest hash a completed round reflected on) is the "Unchanged => $0"
//! short-circuit's memory. It lives in `worker::LibrarianAuto.digest_hash`, inside the
//! `lib_state` HashMap the worker rebuilds EMPTY on every boot. Before the fix it was
//! persisted nowhere, so the first post-restart round for a project ran with
//! `prev_digest_hash=None` and made a PAID call for a byte-identical digest — a call
//! the pin would have short-circuited to Unchanged at $0.
//!
//! This test drives the REAL worker seam (`lib_entry` hydration + `librarian_round_step`
//! + `reflect_auto_with_client`'s delta gate) with a FAKE, $0 client — no paid calls —
//! and proves:
//!   - a round pins a digest and the pin survives a store reopen (a "restart");
//!   - after the restart, `lib_entry` hydrates that pin, so a DIGEST-NEUTRAL edit
//!     (a code-only change — the reflect digest is memory notes + doctor findings, not
//!     code) short-circuits to Unchanged, $0, NO network call;
//!   - NEGATIVE CONTROL A (the bug): the identical restart-then-edit WITHOUT the pin
//!     (a fresh, un-hydrated entry) re-pays the same byte-identical digest;
//!   - NEGATIVE CONTROL B (scope): a bare restart with NO edit fires no round at all, $0.
//!
//! This file is a permanent regression test (normal `tests/` layout), NOT a paid sim.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;

use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::reflect::{
    reflect_auto_with_client, ReflectClient, ReflectConfig, ReflectReason, ReflectResponse,
};
use koden_lib::modules::brain::store::SqliteIndex;
use koden_lib::modules::brain::worker::{
    lib_entry, librarian_round_step, note_content_change, LibrarianAuto, LIBRARIAN_IDLE_SETTLE_MS,
    LIBRARIAN_MIN_GAP_MS,
};

const PID: &str = "p";
const NOW_DATE: &str = "2026-07-10";
const GAP: i64 = LIBRARIAN_MIN_GAP_MS;
const SETTLE: i64 = LIBRARIAN_IDLE_SETTLE_MS;

/// Deterministic $0 fake: counts calls, returns a valid (empty-proposals) response
/// with nonzero token usage so a round both succeeds (Ok) and costs > $0. The whole
/// point is that this NEVER contacts a network — the counter is the observable.
struct Counting {
    calls: Cell<u32>,
}
impl ReflectClient for Counting {
    fn complete(&self, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<ReflectResponse, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(ReflectResponse {
            json_text: r#"{"proposals":[]}"#.to_string(),
            input_tokens: 100,
            output_tokens: 50,
        })
    }
}

/// Write one anchorless memory note (no anchors => no doctor findings => the digest is
/// exactly the notes section, fully deterministic across reopens).
fn note(root: &Path, id: &str, title: &str) {
    let body = format!("---\nid: {id}\ntype: insight\ntitle: {title}\nstatus: active\n---\n# {title}\n\nBody.\n");
    let p = root.join(".koden-memory").join(format!("{id}.md"));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

#[test]
fn pin_survives_restart_so_a_digest_neutral_edit_is_free() {
    let base = std::env::temp_dir().join(format!("koden-pin-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("proj");
    note(&root, "n1", "Sessions expire after a 24h TTL");

    let db = base.join("store").join("index.sqlite");
    let idx = SqliteIndex::open_with_recovery(&db).expect("open store");
    scan_project_memory(&idx, PID, &root);
    idx.set_budget_ceiling(1.0, 1).expect("set ceiling"); // arm the budget (real path)

    let cfg = ReflectConfig {
        in_rate: 1.0 / 1_000_000.0,
        out_rate: 4.0 / 1_000_000.0,
        ..ReflectConfig::default()
    };
    let client = Counting { calls: Cell::new(0) };

    assert_eq!(idx.librarian_pin(PID), None, "no pin before any round");

    // ===== SESSION 1 — first paid round pins the digest =========================
    let t0: i64 = 1_000_000;
    let pinned = {
        let mut lib1: HashMap<String, LibrarianAuto> = HashMap::new();
        let st = lib_entry(&mut lib1, &idx, PID);
        assert_eq!(st.digest_hash, None, "fresh boot: pin hydrates to None (never reflected)");
        note_content_change(st, t0);
        let out = librarian_round_step(st, t0 + SETTLE + GAP, |prev| {
            assert_eq!(prev, None, "session-1 round has no prior pin");
            reflect_auto_with_client(&idx, &client, &cfg, PID, Some(NOW_DATE), t0, prev)
        })
        .expect("session-1 round fires");
        assert!(matches!(out.reason, ReflectReason::Ok), "the paid round succeeds: {:?}", out.reason);
        // What the worker does after a round: persist the settled pin.
        let h = st.digest_hash.clone().expect("Ok pins a digest hash");
        idx.set_librarian_pin(PID, &h, t0).expect("persist pin");
        h
    };
    assert_eq!(client.calls.get(), 1, "session 1 makes exactly one call");

    // ===== RESTART — reopen the durable store; rebuild lib_state EMPTY ===========
    drop(idx);
    let idx = SqliteIndex::open_with_recovery(&db).expect("reopen store");
    assert_eq!(
        idx.librarian_pin(PID).as_deref(),
        Some(pinned.as_str()),
        "the pin survives a store reopen (the durability the fix adds)"
    );

    // ===== SESSION 2 (the repro) — restart + digest-neutral edit => Unchanged, $0 =
    let t1 = t0 + 10 * GAP;
    {
        let mut lib2: HashMap<String, LibrarianAuto> = HashMap::new();
        let st = lib_entry(&mut lib2, &idx, PID);
        assert_eq!(
            st.digest_hash.as_deref(),
            Some(pinned.as_str()),
            "lib_entry hydrates the persisted pin on first sight this boot"
        );
        note_content_change(st, t1); // a code-only Fs edge arms the round (dirty)
        let calls_before = client.calls.get();
        let out = librarian_round_step(st, t1 + SETTLE + GAP, |prev| {
            assert_eq!(prev, Some(pinned.as_str()), "the hydrated pin reaches the delta gate");
            reflect_auto_with_client(&idx, &client, &cfg, PID, Some(NOW_DATE), t1, prev)
        })
        .expect("session-2 round fires (dirty), then the delta gate short-circuits it");
        assert!(matches!(out.reason, ReflectReason::Unchanged), "byte-identical digest => Unchanged");
        assert_eq!(out.spent_usd, 0.0, "Unchanged is $0");
        assert_eq!(client.calls.get(), calls_before, "NO call for a byte-identical digest after restart");
    }

    // ===== NEGATIVE CONTROL A (the bug) — same edit, but NO persisted pin ========
    // Reproduce the pre-fix state: an un-hydrated entry (prev=None) re-pays the very
    // same byte-identical digest.
    {
        let mut st = LibrarianAuto::default(); // NOT hydrated
        assert_eq!(st.digest_hash, None);
        note_content_change(&mut st, t1);
        let calls_before = client.calls.get();
        let out = librarian_round_step(&mut st, t1 + SETTLE + GAP, |prev| {
            assert_eq!(prev, None, "pre-fix: no pin => the delta gate cannot short-circuit");
            reflect_auto_with_client(&idx, &client, &cfg, PID, Some(NOW_DATE), t1, prev)
        })
        .expect("round fires");
        assert!(matches!(out.reason, ReflectReason::Ok), "without the pin the identical digest is re-paid");
        assert_eq!(client.calls.get(), calls_before + 1, "the bug: one wasted paid call");
        assert!(out.spent_usd > 0.0, "and it costs real money");
    }

    // ===== NEGATIVE CONTROL B (scope) — bare restart, NO edit => no round, $0 =====
    {
        let mut lib3: HashMap<String, LibrarianAuto> = HashMap::new();
        let st = lib_entry(&mut lib3, &idx, PID); // hydrated, but never dirtied
        let calls_before = client.calls.get();
        let fired = librarian_round_step(st, t1 + 1000 * GAP, |prev| {
            reflect_auto_with_client(&idx, &client, &cfg, PID, Some(NOW_DATE), t1, prev)
        });
        assert!(fired.is_none(), "bare restart with no edit => due_for_round never fires");
        assert_eq!(client.calls.get(), calls_before, "and no call, $0");
    }

    let _ = std::fs::remove_dir_all(&base);
}
