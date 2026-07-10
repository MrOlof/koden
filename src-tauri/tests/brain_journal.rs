//! Canonical-tail sidecar journal — BLACK-BOX durability proof through the PUBLIC
//! recovery surface a real caller (the worker) sees. The white-box unit tests live
//! inline in `store::sqlite` and reach private seams (`journal_budget`,
//! `replay_canonical_journal`, `compact_now_for_test`, the raw `conn`); this file
//! deliberately touches ONLY the public API, so it proves the whole chain
//!   public write -> commit -> journal append -> header destruction ->
//!   `open_with_recovery` rename-aside -> replay -> restored canonical state
//! works end to end with nothing but what a caller can call.
//!
//! The contract under test (DB is the source of truth; the JSONL sidecar is the
//! backup of last resort): DB-only-canonical data — proposals, reject_signatures,
//! budget spend, librarian pins — must survive a SQLite corruption so total the
//! in-file salvage recovers nothing, because the header is destroyed. A healthy open
//! must NEVER replay. A torn final line must be tolerated. Replay must be idempotent.

use std::cell::Cell;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use koden_lib::modules::brain::memory::proposal::{
    proposal_signature, reject_signature, MemoryProposal, ProposalAction,
};
use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::reflect::{
    reflect_auto_with_client, ReflectClient, ReflectConfig, ReflectReason, ReflectResponse,
};
use koden_lib::modules::brain::store::SqliteIndex;

const PID: &str = "p";
const NOW_DATE: &str = "2026-07-10";

/// A unique scratch dir per test (pid + a label) so parallel `cargo test` runs and
/// leftover files from a prior run never collide.
fn scratch(label: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("koden-brain-journal-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    base
}

fn db_path(base: &Path) -> PathBuf {
    base.join("store").join("index.sqlite")
}

/// The sidecar journal path the store derives (`<stem>.canonical.jsonl`).
fn journal_path(db: &Path) -> PathBuf {
    db.with_file_name("index.canonical.jsonl")
}

/// Destroy the SQLite header so the next open classifies the file as NotADatabase —
/// the rename-aside recovery branch fires and the in-file ATTACH salvage can read
/// nothing (the exact gap the sidecar journal closes). Drops any WAL/SHM first so
/// recovery sees only the wrecked main file.
fn destroy_header(db: &Path) {
    for suffix in ["-wal", "-shm"] {
        let name = format!("{}{suffix}", db.file_name().unwrap().to_string_lossy());
        let _ = std::fs::remove_file(db.with_file_name(name));
    }
    let mut f = std::fs::OpenOptions::new().write(true).open(db).unwrap();
    f.write_all(&[0xFFu8; 100]).unwrap();
    f.flush().unwrap();
}

/// One anchorless memory note on disk → a non-empty, fully deterministic reflect
/// digest (notes only, no doctor findings), so a round actually runs and spends.
fn write_note(root: &Path, id: &str, title: &str) {
    let body =
        format!("---\nid: {id}\ntype: insight\ntitle: {title}\nstatus: active\n---\n# {title}\n\nBody.\n");
    let p = root.join(".koden-memory").join(format!("{id}.md"));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

/// $0 fake provider: never touches a network, returns a valid empty-proposals reply
/// with nonzero token usage so a round both SUCCEEDS and costs > $0 (the whole point
/// is a real, journaled spend with no paid call).
struct FakeProvider {
    calls: Cell<u32>,
}
impl ReflectClient for FakeProvider {
    fn complete(&self, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<ReflectResponse, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(ReflectResponse {
            json_text: r#"{"proposals":[]}"#.to_string(),
            input_tokens: 1000,
            output_tokens: 500,
        })
    }
}

fn spend_cfg() -> ReflectConfig {
    ReflectConfig {
        in_rate: 1.0 / 1_000_000.0,
        out_rate: 4.0 / 1_000_000.0,
        ..ReflectConfig::default()
    }
}

/// Run ONE real reflect round through the public path (reserve → fake call →
/// reconcile → budget journal). Returns the number of provider calls made.
fn spend_one_round(idx: &SqliteIndex, client: &FakeProvider, now: i64, prev: Option<&str>) -> ReflectReason {
    let (out, _hash) =
        reflect_auto_with_client(idx, client, &spend_cfg(), PID, Some(NOW_DATE), now, prev);
    out.reason
}

fn sample_proposal() -> MemoryProposal {
    MemoryProposal {
        project: PID.into(),
        signature: proposal_signature(ProposalAction::Archive, Some("n1"), "Stale note"),
        action: ProposalAction::Archive,
        target_id: Some("n1".into()),
        title: "Stale note".into(),
        detail: "the body".into(),
        source: "curate".into(),
        status: "pending".into(),
    }
}

/// (a)+(b) The load-bearing test: every DB-only-canonical write class is journaled,
/// and after a header-destroying corruption `open_with_recovery` replays the sidecar
/// to restore ALL of it — proposal status, reject history, budget spend, pin — such
/// that the Librarian cannot spend the ceiling again.
#[test]
fn canonical_state_survives_header_destruction_via_public_api() {
    let base = scratch("survives");
    let root = base.join("proj");
    write_note(&root, "n1", "Sessions expire after a 24h TTL");
    let db = db_path(&base);

    let sig = proposal_signature(ProposalAction::Archive, Some("n1"), "Stale note");
    let reject_sig = reject_signature(ProposalAction::Archive, Some("n1"), "Stale note");

    let spent = {
        let idx = SqliteIndex::open_with_recovery(&db).expect("open");
        scan_project_memory(&idx, PID, &root);

        // Arm the budget generously, spend one real round, then TIGHTEN the ceiling to
        // exactly the durable spend. ceiling == spent makes the "cannot re-spend" proof
        // deterministic (any est > 0 strictly exceeds), independent of the estimator.
        idx.set_budget_ceiling(1.0, 1).unwrap();
        let client = FakeProvider { calls: Cell::new(0) };
        let reason = spend_one_round(&idx, &client, 1_000, None);
        assert!(matches!(reason, ReflectReason::Ok), "the seeded round must run+spend: {reason:?}");
        assert_eq!(client.calls.get(), 1, "exactly one (fake) provider call");
        let (_ceiling, spent) = idx.budget_state();
        assert!(spent > 0.0, "the round moved the spend meter: {spent}");
        idx.set_budget_ceiling(spent, 2).unwrap(); // ceiling := spent (journals the full row)

        // The other canonical classes: a proposal, its rejection (→ reject-signature),
        // and a librarian pin.
        idx.insert_proposal(PID, &sample_proposal(), 1).unwrap();
        idx.resolve_proposal(PID, &sig, true).unwrap();
        idx.set_librarian_pin(PID, "digest-xyz", 3).unwrap();

        // Every canonical write appended a line; a DERIVED write (the note scan / files)
        // did NOT. We can't count exact lines without knowing the reflect internals, but
        // the sidecar must exist and be non-trivial.
        let jlines = std::fs::read_to_string(journal_path(&db)).expect("journal written");
        assert!(jlines.lines().count() >= 4, "canonical writes were journaled: {jlines}");
        assert!(jlines.contains("\"table\":\"brain_budget\""));
        assert!(jlines.contains("\"table\":\"proposals\""));
        assert!(jlines.contains("\"table\":\"reject_signatures\""));
        assert!(jlines.contains("\"table\":\"brain_librarian_pin\""));

        idx.checkpoint();
        spent
    };

    // --- Corruption so total the in-file salvage recovers nothing -------------------
    destroy_header(&db);
    let idx = SqliteIndex::open_with_recovery(&db).expect("recovery open");

    // Budget spend restored bit-for-bit → ceiling == spent → the meter is pinned.
    let (ceiling, spent2) = idx.budget_state();
    assert_eq!(spent2, spent, "durable spend survived the rebuild");
    assert_eq!(ceiling, spent, "ceiling survived the rebuild");

    // Behavioral proof: the Librarian cannot spend the ceiling again. Re-scan the note
    // (DERIVED, dropped on rebuild) so the digest is non-empty, then a fresh round must
    // be refused OverBudget rather than making a call.
    scan_project_memory(&idx, PID, &root);
    let client = FakeProvider { calls: Cell::new(0) };
    let reason = spend_one_round(&idx, &client, 2_000, None);
    assert!(matches!(reason, ReflectReason::OverBudget), "restored ceiling blocks a re-spend: {reason:?}");
    assert_eq!(client.calls.get(), 0, "no provider call when over budget");
    assert_eq!(idx.budget_state().1, spent, "spend meter did not move");

    // Human decisions restored: the proposal is present (rejected) and its reject
    // signature blocks the finding from reappearing; the pin is back.
    assert!(idx.proposal_exists(PID, &sig).unwrap(), "proposal restored");
    assert!(idx.is_rejected(PID, &reject_sig).unwrap(), "reject history restored");
    assert_eq!(idx.librarian_pin(PID), Some("digest-xyz".to_string()), "pin restored");

    // The corrupt original was preserved aside for forensics, never deleted.
    let aside = std::fs::read_dir(db.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
        .count();
    assert_eq!(aside, 1, "corrupt store moved aside");

    let _ = std::fs::remove_dir_all(&base);
}

/// (c) A HEALTHY open must never replay the journal — replay is scoped to the
/// corrupt-cache rebuild branch. A phantom line for a project the DB never had would
/// leak in if a healthy open replayed.
#[test]
fn healthy_open_never_replays() {
    let base = scratch("healthy");
    let db = db_path(&base);
    {
        let idx = SqliteIndex::open_with_recovery(&db).expect("open");
        idx.set_librarian_pin(PID, "real", 1).unwrap();
    }
    // Inject a phantom pin for a "ghost" project directly into the sidecar.
    {
        let mut f = std::fs::OpenOptions::new().append(true).open(journal_path(&db)).unwrap();
        writeln!(
            f,
            "{{\"seq\":999,\"table\":\"brain_librarian_pin\",\"op\":\"upsert\",\"data\":{{\"project_id\":\"ghost\",\"digest_hash\":\"phantom\",\"updated_at\":0}},\"ts\":0}}"
        )
        .unwrap();
    }
    let idx = SqliteIndex::open_with_recovery(&db).expect("healthy reopen");
    assert_eq!(idx.librarian_pin("ghost"), None, "healthy open must not replay the sidecar");
    assert_eq!(idx.librarian_pin(PID), Some("real".to_string()), "the real row is intact");

    let _ = std::fs::remove_dir_all(&base);
}

/// (d) A torn final line (a crash mid-append) is skipped; the earlier good lines
/// still replay.
#[test]
fn truncated_final_line_is_tolerated() {
    let base = scratch("truncated");
    let db = db_path(&base);
    {
        let idx = SqliteIndex::open_with_recovery(&db).expect("open");
        idx.set_librarian_pin(PID, "good-digest", 1).unwrap();
        idx.checkpoint();
    }
    // Append a half-written JSON fragment with no trailing newline.
    {
        let mut f = std::fs::OpenOptions::new().append(true).open(journal_path(&db)).unwrap();
        write!(f, "{{\"seq\":2,\"table\":\"brain_librarian_pin\",\"op\":\"ups").unwrap();
    }
    destroy_header(&db);
    let idx = SqliteIndex::open_with_recovery(&db).expect("recovery open");
    assert_eq!(
        idx.librarian_pin(PID),
        Some("good-digest".to_string()),
        "the good line replayed despite the torn tail"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// (e) Replay is idempotent, observed black-box: a SECOND full corrupt→recover cycle
/// (a fresh DB whose high-water starts at 0, replaying the same lines again) yields
/// exactly the same canonical state — no duplicated proposal, no doubled spend.
#[test]
fn replay_is_idempotent_across_cycles() {
    let base = scratch("idempotent");
    let root = base.join("proj");
    write_note(&root, "n1", "Sessions expire after a 24h TTL");
    let db = db_path(&base);
    let sig = proposal_signature(ProposalAction::Archive, Some("n1"), "Stale note");

    let spent = {
        let idx = SqliteIndex::open_with_recovery(&db).expect("open");
        scan_project_memory(&idx, PID, &root);
        idx.set_budget_ceiling(1.0, 1).unwrap();
        let client = FakeProvider { calls: Cell::new(0) };
        assert!(matches!(spend_one_round(&idx, &client, 1_000, None), ReflectReason::Ok));
        let (_c, spent) = idx.budget_state();
        idx.insert_proposal(PID, &sample_proposal(), 1).unwrap();
        idx.resolve_proposal(PID, &sig, true).unwrap();
        idx.set_librarian_pin(PID, "pin-1", 2).unwrap();
        idx.checkpoint();
        spent
    };

    // Cycle 1: corrupt → recover (replay #1).
    destroy_header(&db);
    let after1 = {
        let idx = SqliteIndex::open_with_recovery(&db).expect("recover #1");
        idx.checkpoint();
        (idx.budget_state(), idx.proposal_exists(PID, &sig).unwrap(), idx.librarian_pin(PID))
    };

    // Cycle 2: corrupt the REBUILT db → recover again (a fresh DB, high-water 0, so the
    // SAME sidecar lines replay a second time from scratch).
    destroy_header(&db);
    let after2 = {
        let idx = SqliteIndex::open_with_recovery(&db).expect("recover #2");
        (idx.budget_state(), idx.proposal_exists(PID, &sig).unwrap(), idx.librarian_pin(PID))
    };

    assert_eq!(after1, after2, "a second replay cycle changed the canonical state");
    assert_eq!(after2.0 .1, spent, "spend was not double-applied across replays");
    assert!(after2.1, "proposal present exactly once (INSERT OR REPLACE by PK)");
    assert_eq!(after2.2, Some("pin-1".to_string()), "pin stable across replays");

    let _ = std::fs::remove_dir_all(&base);
}
