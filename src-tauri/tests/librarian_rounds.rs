//! Librarian fail-streak cap — integration-level coverage (ADR-010 cluster 5).
//! Drives the REAL per-project round step ([worker::librarian_round_step] — the
//! exact fn `run_librarian_rounds` delegates to, no drift copy) plus the real
//! Fs-handler fold ([worker::note_content_change]) over a simulated timeline,
//! with a counting runner standing in for `reflect_auto`. "An attempt fired" =
//! the runner was invoked at all — stronger than "no paid call", since even the
//! $0 pre-flight would require the runner to run. Proves:
//!  - consecutive CallFailed rounds increment the streak with exponential backoff;
//!  - at LIBRARIAN_MAX_CONSEC_FAILURES the round is NOT re-armed: no attempt
//!    fires no matter how many ticks pass;
//!  - a NEW content change re-arms the round; the next SUCCESSFUL round resets
//!    the streak to 0 and restores the normal min-gap cadence (the streak itself
//!    survives the content change by design, keeping the backoff gap in force
//!    until the provider actually answers well);
//!  - persistent failures (InvalidOutput: the model answered, the JSON failed
//!    validation) classify differently from transient ones (CallFailed): they
//!    pin the digest hash and NEVER re-arm — byte-identical input would fail
//!    identically, so only new content buys another attempt;
//!  - $0 pre-flight skips (Disabled) re-arm without touching the streak.

use std::cell::Cell;

use koden_lib::modules::brain::reflect::{ReflectOutcome, ReflectReason};
use koden_lib::modules::brain::worker::{
    librarian_round_step, note_content_change, LibrarianAuto, LIBRARIAN_IDLE_SETTLE_MS,
    LIBRARIAN_MAX_CONSEC_FAILURES, LIBRARIAN_MIN_GAP_MS,
};

const GAP: i64 = LIBRARIAN_MIN_GAP_MS;
const SETTLE: i64 = LIBRARIAN_IDLE_SETTLE_MS;

fn outcome(reason: ReflectReason) -> ReflectOutcome {
    ReflectOutcome { proposals: Vec::new(), spent_usd: 0.0, reason }
}

/// One Tick for one project: run the REAL round step; if it fires, the runner
/// records the attempt and returns the scripted outcome. Returns whether an
/// attempt fired this tick.
fn tick(
    st: &mut LibrarianAuto,
    now_ms: i64,
    attempts: &Cell<u32>,
    reason: ReflectReason,
    digest_hash: Option<&str>,
) -> bool {
    librarian_round_step(st, now_ms, |_prev| {
        attempts.set(attempts.get() + 1);
        (outcome(reason), digest_hash.map(str::to_string))
    })
    .is_some()
}

/// Transient (CallFailed) failures: streak increments with backoff, the cap
/// parks the project (no attempt fires at all), and a NEW content change plus
/// one successful round resets the streak and the normal cadence.
#[test]
fn fail_streak_cap_parks_and_new_content_recovers() {
    let attempts = Cell::new(0u32);
    let mut st = LibrarianAuto::default();

    // Edit lands at t=0; the project settles, and the first round fires.
    note_content_change(&mut st, 0);
    let t1 = SETTLE + GAP;
    assert!(tick(&mut st, t1, &attempts, ReflectReason::CallFailed("x".into()), None));
    assert_eq!(st.fail_streak, 1);
    assert!(st.dirty, "transient failure below the cap must re-arm");

    // Backoff: the plain min-gap no longer fires after one failure…
    assert!(!tick(&mut st, t1 + GAP, &attempts, ReflectReason::Ok, Some("never")));
    // …the doubled gap does. Walk the streak up to the cap (gap doubles each time).
    let mut t = t1;
    for expected in 2..=LIBRARIAN_MAX_CONSEC_FAILURES {
        t += GAP << (expected - 1); // the widened gap for the CURRENT streak
        assert!(
            tick(&mut st, t, &attempts, ReflectReason::CallFailed("x".into()), None),
            "retry #{expected} should fire once its backoff gap elapses"
        );
        assert_eq!(st.fail_streak, expected);
    }
    assert!(!st.dirty, "at the cap the round must NOT re-arm");
    let attempts_at_cap = attempts.get();
    assert_eq!(attempts_at_cap, LIBRARIAN_MAX_CONSEC_FAILURES);

    // Parked: no matter how far the clock runs, NO attempt fires — the runner
    // is scripted to succeed, so only the gate can be holding it back.
    let t_cap = t;
    for far in [t_cap + 8 * GAP, t_cap + 100 * GAP, t_cap + 100_000 * GAP] {
        assert!(!tick(&mut st, far, &attempts, ReflectReason::Ok, Some("never")));
    }
    assert_eq!(attempts.get(), attempts_at_cap, "parked project must not attempt (or pay) again");

    // A NEW content change re-arms — the streak survives (by design: the
    // provider hasn't proven itself yet), so the widened gap still gates…
    let t_edit = t_cap + GAP;
    note_content_change(&mut st, t_edit);
    assert!(st.dirty, "new content must re-arm the round");
    assert_eq!(st.fail_streak, LIBRARIAN_MAX_CONSEC_FAILURES, "content alone does not clear the streak");
    assert!(!tick(&mut st, t_edit + SETTLE, &attempts, ReflectReason::Ok, Some("h-new")));

    // …and once the capped-streak gap elapses, the attempt fires; success
    // resets the streak and records the digest.
    let t_ok = t_cap + (GAP << LIBRARIAN_MAX_CONSEC_FAILURES);
    assert!(tick(&mut st, t_ok, &attempts, ReflectReason::Ok, Some("h-new")));
    assert_eq!(st.fail_streak, 0, "a successful round resets the streak");
    assert_eq!(st.digest_hash.as_deref(), Some("h-new"));

    // Normal cadence restored: the very next edit fires after the plain min-gap.
    note_content_change(&mut st, t_ok);
    assert!(tick(&mut st, t_ok + GAP, &attempts, ReflectReason::Ok, Some("h-new2")));
    assert_eq!(st.digest_hash.as_deref(), Some("h-new2"));
}

/// Persistent failures (InvalidOutput) classify differently from transient
/// ones: PAID, streak counted, digest hash pinned, and NEVER re-armed — the
/// same digest must never be re-paid; only new content buys another attempt,
/// and the pinned hash reaches the next round's delta gate.
#[test]
fn invalid_output_parks_immediately_and_pins_digest() {
    let attempts = Cell::new(0u32);
    let mut st = LibrarianAuto::default();

    note_content_change(&mut st, 0);
    let t1 = SETTLE + GAP;
    let fired = librarian_round_step(&mut st, t1, |prev| {
        attempts.set(attempts.get() + 1);
        assert_eq!(prev, None, "first round has no previous digest");
        (outcome(ReflectReason::InvalidOutput), Some("d1".into()))
    });
    assert!(fired.is_some());
    assert_eq!(st.fail_streak, 1);
    assert!(!st.dirty, "InvalidOutput must NOT re-arm even below the cap");
    assert_eq!(st.digest_hash.as_deref(), Some("d1"), "failing digest pinned");

    // Unlike CallFailed at streak 1 (which retries after 2×gap), no tick ever
    // fires again on the same content.
    for far in [t1 + 2 * GAP, t1 + 100 * GAP] {
        assert!(!tick(&mut st, far, &attempts, ReflectReason::Ok, Some("never")));
    }
    assert_eq!(attempts.get(), 1, "the same digest must never be re-paid");

    // New content re-arms; the retry (past the streak-1 backoff gap) hands the
    // PINNED hash to the delta gate, and success resets the streak.
    let t_edit = t1 + 2 * GAP;
    note_content_change(&mut st, t_edit);
    let t2 = t_edit + SETTLE + 2 * GAP;
    let fired = librarian_round_step(&mut st, t2, |prev| {
        attempts.set(attempts.get() + 1);
        assert_eq!(prev, Some("d1"), "pinned digest hash must reach the delta gate");
        (outcome(ReflectReason::Ok), Some("d2".into()))
    });
    assert!(fired.is_some());
    assert_eq!(st.fail_streak, 0);
    assert_eq!(st.digest_hash.as_deref(), Some("d2"));
}

/// $0 pre-flight skips (Disabled/NoKey/OverBudget/EmptyCorpus) are not failures:
/// they re-arm without touching the streak, so the next eligible tick retries at
/// the plain min-gap — no backoff, no park.
#[test]
fn zero_cost_skips_rearm_without_streak() {
    let attempts = Cell::new(0u32);
    let mut st = LibrarianAuto::default();

    note_content_change(&mut st, 0);
    let t1 = SETTLE + GAP;
    assert!(tick(&mut st, t1, &attempts, ReflectReason::Disabled, None));
    assert_eq!(st.fail_streak, 0, "a free skip is not a failure");
    assert!(st.dirty, "a free skip must re-arm (recover once a budget/key is set)");

    // Plain min-gap suffices — no backoff was accrued.
    assert!(tick(&mut st, t1 + GAP, &attempts, ReflectReason::Disabled, None));
    assert_eq!(attempts.get(), 2);
    assert_eq!(st.fail_streak, 0);
}
