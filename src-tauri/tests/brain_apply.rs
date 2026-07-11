//! D2 + ADR-018 — APPLY a proposal to project memory (approve or the autonomous
//! sweep) and REVERT it from its snapshot. These tests drive the real store methods
//! (`SqliteIndex::{apply_proposal, revert_proposal}` + `worker::auto_apply_pending`)
//! against temp stores + temp project dirs, and assert the end-to-end effect: the
//! `.koden-memory/*.md` FILE materializes/changes, the notes TABLE reflects it, a
//! re-scan is idempotent, a missing target soft-fails leaving the proposal pending,
//! reject writes a reject-signature and materializes nothing, every apply snapshots
//! an inverse that revert restores VERBATIM (idempotently), stacked changes on one
//! note revert NEWEST-FIRST (an older revert is gated while a newer applied change
//! touches the same note), the autonomous sweep
//! applies in `autonomous` mode and parks in `review` mode, the post-apply digest
//! pin keeps the delta gate at $0, and both status flips are journaled (survive a
//! header wipe).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use koden_lib::modules::brain::memory::proposal::{
    proposal_signature, reject_signature, MemoryProposal, ProposalAction,
};
use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::store::{
    list_memory_changes_readonly, list_notes_readonly, list_proposals_readonly, SqliteIndex,
};

const PID: &str = "p";
const NOW: &str = "2026-07-10";
const NOW_MS: i64 = 1_000;

fn write_note(root: &Path, id: &str, extra_fm: &str, body: &str) {
    let p = root.join(".koden-memory").join(format!("{id}.md"));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let content = format!("---\nid: {id}\ntype: decision\ntitle: Note {id}\nstatus: active\n{extra_fm}---\n# Note {id}\n\n{body}\n");
    std::fs::write(p, content).unwrap();
}

fn read_note(root: &Path, id: &str) -> String {
    std::fs::read_to_string(root.join(".koden-memory").join(format!("{id}.md"))).unwrap()
}

fn mk_proposal(
    action: ProposalAction,
    target_id: Option<&str>,
    title: &str,
    detail: &str,
) -> MemoryProposal {
    MemoryProposal {
        project: PID.to_string(),
        signature: proposal_signature(action, target_id, title),
        action,
        target_id: target_id.map(String::from),
        title: title.to_string(),
        detail: detail.to_string(),
        source: "reflect".to_string(),
        status: "pending".to_string(),
    }
}

/// (index, store TempDir, work TempDir, root). Keep the TempDirs alive in the caller.
fn setup() -> (SqliteIndex, tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let store = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = work.path().to_path_buf();
    let idx = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
    (idx, store, work, root)
}

fn pending_sigs(db: &Path) -> Vec<String> {
    list_proposals_readonly(db, Some(PID))
        .unwrap()
        .into_iter()
        .map(|p| p.signature)
        .collect()
}

#[test]
fn approve_create_materializes_file_and_table_and_is_idempotent() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    let p = mk_proposal(ProposalAction::Create, None, "New Insight", "Some knowledge.");
    idx.insert_proposal(PID, &p, 1).unwrap();

    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();

    // FILE materialized.
    let file = root.join(".koden-memory").join("new-insight.md");
    let raw = std::fs::read_to_string(&file).unwrap();
    assert!(raw.contains("Some knowledge."), "body written: {raw}");
    assert!(raw.contains("id: new-insight") && raw.contains("status: active"));
    // TABLE reflects it.
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    assert!(notes.iter().any(|n| n.id == "new-insight"), "note in table");
    // Proposal no longer pending.
    assert!(!pending_sigs(&db).contains(&p.signature), "proposal left pending");

    // Idempotent: a re-scan doesn't duplicate, and re-applying an already-applied
    // proposal is a no-op (no second file, no error).
    scan_project_memory(&idx, PID, &root);
    let before = std::fs::read_to_string(&file).unwrap();
    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "re-apply is a no-op");
    let count = list_notes_readonly(&db, Some(PID)).unwrap().iter().filter(|n| n.id == "new-insight").count();
    assert_eq!(count, 1, "no duplicate note");
}

#[test]
fn approve_archive_flips_file_and_table() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    write_note(&root, "old", "", "Body.");
    scan_project_memory(&idx, PID, &root);
    let p = mk_proposal(ProposalAction::Archive, Some("old"), "Archive old", "stale");
    idx.insert_proposal(PID, &p, 1).unwrap();

    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();

    // FILE frontmatter flipped, body preserved.
    let raw = read_note(&root, "old");
    assert!(raw.contains("status: archived"), "file archived: {raw}");
    assert!(raw.contains("Body."), "body preserved");
    // TABLE status flipped.
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    let n = notes.iter().find(|n| n.id == "old").unwrap();
    assert_eq!(n.status.as_deref(), Some("archived"));
}

#[test]
fn approve_supersede_wires_both_sides() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    write_note(&root, "old", "", "Body.");
    scan_project_memory(&idx, PID, &root);
    let p = mk_proposal(ProposalAction::Supersede, Some("old"), "Newer Decision", "the new take");
    idx.insert_proposal(PID, &p, 1).unwrap();

    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();

    // New note carries the forward edge; it reused the target's type (decision).
    let new_raw = read_note(&root, "newer-decision");
    assert!(new_raw.contains("supersedes: old"), "forward edge: {new_raw}");
    assert!(new_raw.contains("type: decision"), "reused target type");
    // Old note carries the back edge.
    let old_raw = read_note(&root, "old");
    assert!(old_raw.contains("superseded_by: newer-decision"), "back edge: {old_raw}");
    // Table reflects both.
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    assert!(notes.iter().any(|n| n.id == "newer-decision"));
    assert!(notes.iter().any(|n| n.id == "old"));
}

#[test]
fn approve_update_appends_dated_section_without_rewrite() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    write_note(&root, "upd", "", "Original prose.");
    scan_project_memory(&idx, PID, &root);
    let before = read_note(&root, "upd");
    let p = mk_proposal(ProposalAction::Update, Some("upd"), "Refine upd", "An added observation.");
    idx.insert_proposal(PID, &p, 1).unwrap();

    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();

    let after = read_note(&root, "upd");
    assert!(after.starts_with(before.trim_end()), "original bytes kept as a prefix");
    assert!(after.contains(&format!("## Update ({NOW})")), "dated section: {after}");
    assert!(after.contains("An added observation."));
    assert!(after.contains("Original prose."), "existing prose untouched");
    // Still present in table (not left pending).
    assert!(!pending_sigs(&db).contains(&p.signature));
}

#[test]
fn missing_target_soft_fails_leaving_proposal_pending() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    let p = mk_proposal(ProposalAction::Archive, Some("nope"), "Archive missing", "x");
    idx.insert_proposal(PID, &p, 1).unwrap();

    let err = idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap_err();
    assert!(err.contains("not found"), "clear soft error: {err}");
    // Proposal stays PENDING; nothing materialized.
    assert!(pending_sigs(&db).contains(&p.signature), "proposal still pending");
    assert!(!root.join(".koden-memory").exists() || list_notes_readonly(&db, Some(PID)).unwrap().is_empty());
}

#[test]
fn reject_writes_reject_signature_and_materializes_nothing() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    let p = mk_proposal(ProposalAction::Create, None, "Rejected Idea", "no thanks");
    idx.insert_proposal(PID, &p, 1).unwrap();

    // Reject path is unchanged: resolve_proposal(reject=true).
    assert!(idx.resolve_proposal(PID, &p.signature, true).unwrap());

    assert!(!pending_sigs(&db).contains(&p.signature), "no longer pending");
    let rej = reject_signature(ProposalAction::Create, None, "Rejected Idea");
    assert!(idx.is_rejected(PID, &rej).unwrap(), "reject-signature persisted");
    // No file created.
    assert!(
        !root.join(".koden-memory").join("rejected-idea.md").exists(),
        "reject materializes nothing"
    );
}

// ===========================================================================
// ADR-018 — snapshot undo (revert) + the autonomous sweep
// ===========================================================================

use koden_lib::modules::brain::reflect::{
    pin_corpus_digest, reflect_auto_with_client, ReflectClient, ReflectConfig, ReflectReason,
    ReflectResponse,
};
use koden_lib::modules::brain::worker::auto_apply_pending;

/// One applied+reverted round-trip helper: returns the change row for `sig`.
fn change_row(db: &Path, sig: &str) -> koden_lib::modules::brain::memory::proposal::MemoryChange {
    list_memory_changes_readonly(db, Some(PID), 50)
        .unwrap()
        .into_iter()
        .find(|c| c.signature == sig)
        .expect("change row present")
}

#[test]
fn revert_create_removes_minted_note_and_writes_reject_signature() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    let p = mk_proposal(ProposalAction::Create, None, "Fresh Insight", "Body of it.");
    idx.insert_proposal(PID, &p, 1).unwrap();
    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, true).unwrap();

    let file = root.join(".koden-memory").join("fresh-insight.md");
    assert!(file.exists(), "apply materialized the note");
    let ch = change_row(&db, &p.signature);
    assert_eq!(ch.status, "applied");
    assert!(ch.auto_applied, "auto flag recorded");
    assert!(ch.revertible, "create apply recorded its minted id");
    assert_eq!(ch.applied_ms, Some(NOW_MS));

    assert!(idx.revert_proposal(PID, &root, &p.signature, 2_000).unwrap(), "revert applied");
    assert!(!file.exists(), "revert deleted the minted note");
    assert!(
        !list_notes_readonly(&db, Some(PID)).unwrap().iter().any(|n| n.id == "fresh-insight"),
        "notes table re-synced"
    );
    let ch = change_row(&db, &p.signature);
    assert_eq!(ch.status, "reverted");
    assert_eq!(ch.reverted_ms, Some(2_000));
    assert!(!ch.revertible, "a reverted row is not revertible again");
    // The undo persists a reject-signature so the Librarian can't re-propose +
    // re-auto-apply the same change (the undo-fights-the-librarian loop).
    let rej = reject_signature(ProposalAction::Create, None, "Fresh Insight");
    assert!(idx.is_rejected(PID, &rej).unwrap(), "revert persisted the reject-signature");
}

#[test]
fn revert_archive_restores_prior_bytes_even_without_a_status_key() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    // Deliberately NO `status:` key — archive INSERTS one, so only a full-bytes
    // snapshot can restore the original exactly (there is no remove-key edit).
    let prior = "---\nid: bare\ntype: decision\ntitle: Bare note\n---\n# Bare note\n\nBody.\n";
    let file = root.join(".koden-memory").join("bare.md");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, prior).unwrap();
    scan_project_memory(&idx, PID, &root);

    let p = mk_proposal(ProposalAction::Archive, Some("bare"), "Archive bare", "stale");
    idx.insert_proposal(PID, &p, 1).unwrap();
    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().contains("status: archived"));

    assert!(idx.revert_proposal(PID, &root, &p.signature, 2_000).unwrap());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), prior, "prior bytes restored VERBATIM");
    let n = list_notes_readonly(&db, Some(PID)).unwrap();
    assert_eq!(
        n.iter().find(|n| n.id == "bare").unwrap().status,
        None,
        "table re-synced to the (status-less) original"
    );
}

#[test]
fn revert_update_restores_prior_bytes() {
    let (idx, store, _work, root) = setup();
    let _db = store.path().join("i.sqlite");
    write_note(&root, "upd", "", "Original prose.");
    scan_project_memory(&idx, PID, &root);
    let before = read_note(&root, "upd");
    let p = mk_proposal(ProposalAction::Update, Some("upd"), "Refine upd", "Appended fact.");
    idx.insert_proposal(PID, &p, 1).unwrap();
    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();
    assert!(read_note(&root, "upd").contains("Appended fact."));

    assert!(idx.revert_proposal(PID, &root, &p.signature, 2_000).unwrap());
    assert_eq!(read_note(&root, "upd"), before, "append undone byte-for-byte");
}

#[test]
fn revert_supersede_removes_new_note_and_restores_target() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    write_note(&root, "old", "", "Body.");
    scan_project_memory(&idx, PID, &root);
    let before = read_note(&root, "old");
    let p = mk_proposal(ProposalAction::Supersede, Some("old"), "Newer Decision", "the new take");
    idx.insert_proposal(PID, &p, 1).unwrap();
    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();
    assert!(root.join(".koden-memory").join("newer-decision.md").exists());
    assert!(read_note(&root, "old").contains("superseded_by: newer-decision"));

    assert!(idx.revert_proposal(PID, &root, &p.signature, 2_000).unwrap());
    assert!(
        !root.join(".koden-memory").join("newer-decision.md").exists(),
        "superseding note deleted"
    );
    assert_eq!(read_note(&root, "old"), before, "back-edge undone byte-for-byte");
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    assert!(!notes.iter().any(|n| n.id == "newer-decision"));
}

#[test]
fn revert_is_idempotent_and_noops_on_unapplied_rows() {
    let (idx, store, _work, root) = setup();
    let _db = store.path().join("i.sqlite");
    let p = mk_proposal(ProposalAction::Create, None, "Twice", "x");
    idx.insert_proposal(PID, &p, 1).unwrap();

    // Reverting a PENDING proposal is a no-op (nothing was applied).
    assert!(!idx.revert_proposal(PID, &root, &p.signature, 10).unwrap());
    // And a missing signature too.
    assert!(!idx.revert_proposal(PID, &root, "no|such|sig", 10).unwrap());

    idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, false).unwrap();
    assert!(idx.revert_proposal(PID, &root, &p.signature, 20).unwrap(), "first revert acts");
    assert!(
        !idx.revert_proposal(PID, &root, &p.signature, 30).unwrap(),
        "second revert is a no-op (idempotent)"
    );
    assert!(!root.join(".koden-memory").join("twice.md").exists());
}

#[test]
fn pre_adr018_applied_row_without_snapshot_soft_fails_and_stays_applied() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    let p = mk_proposal(ProposalAction::Create, None, "Legacy", "applied before undo existed");
    idx.insert_proposal(PID, &p, 1).unwrap();
    // Simulate a pre-ADR-018 apply: `resolve_proposal(reject=false)` is the legacy
    // status-only flip — it records NO undo snapshot, exactly the old row shape.
    assert!(idx.resolve_proposal(PID, &p.signature, false).unwrap());
    let err = idx.revert_proposal(PID, &root, &p.signature, 10).unwrap_err();
    assert!(err.contains("before undo snapshots"), "clear soft error: {err}");
    let ch = change_row(&db, &p.signature);
    assert_eq!(ch.status, "applied", "row untouched by the failed revert");
    assert!(!ch.revertible, "listed as not revertible");
}

/// Stacked applied changes on ONE note must unwind newest-first: snapshots are
/// full prior files, so restoring an OLDER snapshot would silently wipe every
/// newer applied change (and reverting the newer one afterwards would resurrect
/// the content just undone). Both the feed's `revertible` flag and
/// `revert_proposal` itself enforce the gate; the cascade B-then-A converges to
/// the original bytes.
#[test]
fn stacked_changes_on_one_note_revert_newest_first_only() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    write_note(&root, "hot", "", "Original prose.");
    scan_project_memory(&idx, PID, &root);
    let original = read_note(&root, "hot");

    // A then B on the SAME note: update (snapshot S0), then archive (snapshot S0+A).
    let a = mk_proposal(ProposalAction::Update, Some("hot"), "Refine hot", "Fact A.");
    idx.insert_proposal(PID, &a, 1).unwrap();
    idx.apply_proposal(PID, &root, &a.signature, NOW, 1_000, true).unwrap();
    let after_a = read_note(&root, "hot");
    let b = mk_proposal(ProposalAction::Archive, Some("hot"), "Archive hot", "stale");
    idx.insert_proposal(PID, &b, 2).unwrap();
    idx.apply_proposal(PID, &root, &b.signature, NOW, 2_000, true).unwrap();
    let after_b = read_note(&root, "hot");
    assert!(after_b.contains("Fact A.") && after_b.contains("status: archived"));

    // The feed gates the OLDER change; only the newest applied row offers Revert.
    let ch_a = change_row(&db, &a.signature);
    assert!(!ch_a.revertible, "older stacked change is gated");
    assert!(ch_a.blocked_by_newer, "and marked as gated by a newer sibling");
    let ch_b = change_row(&db, &b.signature);
    assert!(ch_b.revertible && !ch_b.blocked_by_newer, "newest change revertible");

    // Reverting the OLDER change is refused: its snapshot predates B's archive.
    let err = idx.revert_proposal(PID, &root, &a.signature, 3_000).unwrap_err();
    assert!(err.contains("newest-first"), "clear soft error: {err}");
    assert_eq!(read_note(&root, "hot"), after_b, "file untouched by the refused revert");
    assert_eq!(change_row(&db, &a.signature).status, "applied", "row untouched");

    // Newest-first cascade: revert B (A's update survives), then A (original bytes).
    assert!(idx.revert_proposal(PID, &root, &b.signature, 4_000).unwrap());
    assert_eq!(read_note(&root, "hot"), after_a, "B undone, A's content intact");
    let ch_a = change_row(&db, &a.signature);
    assert!(ch_a.revertible && !ch_a.blocked_by_newer, "A re-exposed once B is gone");
    assert!(idx.revert_proposal(PID, &root, &a.signature, 5_000).unwrap());
    assert_eq!(read_note(&root, "hot"), original, "stack fully unwound");
}

/// The gate also spans MINTED notes: a create's undo deletes the file, so a newer
/// applied change TARGETING that minted note (target_id == the create's
/// undo_created_id) blocks the create's revert. Unrelated notes never gate each
/// other.
#[test]
fn revert_of_create_is_blocked_while_a_newer_change_touches_the_minted_note() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    let c = mk_proposal(ProposalAction::Create, None, "Minted Note", "Body.");
    idx.insert_proposal(PID, &c, 1).unwrap();
    idx.apply_proposal(PID, &root, &c.signature, NOW, 1_000, true).unwrap();
    let u = mk_proposal(ProposalAction::Update, Some("minted-note"), "Refine minted", "More.");
    idx.insert_proposal(PID, &u, 2).unwrap();
    idx.apply_proposal(PID, &root, &u.signature, NOW, 2_000, true).unwrap();

    let ch_c = change_row(&db, &c.signature);
    assert!(!ch_c.revertible && ch_c.blocked_by_newer, "create gated behind the update");
    let err = idx.revert_proposal(PID, &root, &c.signature, 3_000).unwrap_err();
    assert!(err.contains("newest-first"), "clear soft error: {err}");
    assert!(
        root.join(".koden-memory").join("minted-note.md").exists(),
        "minted note NOT deleted out from under the applied update"
    );

    // A newer change on a DIFFERENT note gates nothing.
    let other = mk_proposal(ProposalAction::Create, None, "Unrelated", "x");
    idx.insert_proposal(PID, &other, 3).unwrap();
    idx.apply_proposal(PID, &root, &other.signature, NOW, 4_000, true).unwrap();
    assert!(change_row(&db, &u.signature).revertible, "unrelated newer change doesn't gate");

    // Cascade unwinds: revert the update, then the (now unblocked) create.
    assert!(idx.revert_proposal(PID, &root, &u.signature, 5_000).unwrap());
    assert!(idx.revert_proposal(PID, &root, &c.signature, 6_000).unwrap(), "unblocked after cascade");
    assert!(!root.join(".koden-memory").join("minted-note.md").exists());
}

#[test]
fn autonomous_sweep_applies_and_review_mode_parks() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    let a = mk_proposal(ProposalAction::Create, None, "Auto One", "first fact");
    idx.insert_proposal(PID, &a, 1).unwrap();

    // Default mode is AUTONOMOUS (ADR-018): the sweep applies everything pending.
    assert_eq!(idx.curation_mode(), "autonomous", "ADR-018 default");
    assert_eq!(auto_apply_pending(&idx, PID, &root, NOW_MS), 1);
    assert!(root.join(".koden-memory").join("auto-one.md").exists());
    assert!(pending_sigs(&db).is_empty(), "nothing left pending");
    let ch = change_row(&db, &a.signature);
    assert!(ch.auto_applied && ch.revertible, "sweep records auto + undo");

    // Review mode: behavior unchanged from pre-ADR-018 — proposals park pending.
    idx.set_curation_mode("review", 2).unwrap();
    let b = mk_proposal(ProposalAction::Create, None, "Parked Two", "second fact");
    idx.insert_proposal(PID, &b, 3).unwrap();
    assert_eq!(auto_apply_pending(&idx, PID, &root, NOW_MS), 0, "review mode never applies");
    assert!(pending_sigs(&db).contains(&b.signature), "proposal waits in the inbox");
    assert!(!root.join(".koden-memory").join("parked-two.md").exists());

    // A sweep failure class: soft-fail (missing target) leaves THAT one pending.
    idx.set_curation_mode("autonomous", 4).unwrap();
    let ghost = mk_proposal(ProposalAction::Archive, Some("ghost"), "Archive ghost", "x");
    idx.insert_proposal(PID, &ghost, 5).unwrap();
    assert_eq!(auto_apply_pending(&idx, PID, &root, NOW_MS), 1, "b applies; ghost soft-fails");
    let pend = pending_sigs(&db);
    assert!(pend.contains(&ghost.signature), "unactionable proposal stays visible");
    assert!(!pend.contains(&b.signature));
}

/// The self-feeding-loop guard end to end: auto-applied writes change the corpus,
/// but re-pinning the post-apply digest keeps the next delta-gated round at
/// Unchanged/$0 — no paid call on the Librarian's own writes.
#[test]
fn post_apply_digest_pin_keeps_the_delta_gate_at_zero() {
    struct CountingFake(std::cell::Cell<u32>);
    impl ReflectClient for CountingFake {
        fn complete(&self, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<ReflectResponse, String> {
            self.0.set(self.0.get() + 1);
            Ok(ReflectResponse {
                json_text: r#"{"proposals":[]}"#.into(),
                input_tokens: 10,
                output_tokens: 5,
            })
        }
    }

    let (idx, store, _work, root) = setup();
    let _db = store.path().join("i.sqlite");
    write_note(&root, "seed", "", "A seed body.");
    scan_project_memory(&idx, PID, &root);
    idx.set_budget_ceiling(1.0, 1).unwrap();

    let p = mk_proposal(ProposalAction::Create, None, "Grew From Reflect", "the new fact");
    idx.insert_proposal(PID, &p, 1).unwrap();
    assert_eq!(auto_apply_pending(&idx, PID, &root, NOW_MS), 1, "sweep applied");
    let pin = pin_corpus_digest(&idx, PID, Some(NOW), NOW_MS).expect("non-empty corpus pins");

    // With the post-apply pin, the next autonomous round is Unchanged at $0.
    let fake = CountingFake(std::cell::Cell::new(0));
    let (out, h) = reflect_auto_with_client(
        &idx, &fake, &ReflectConfig::default(), PID, Some(NOW), 2_000, Some(&pin),
    );
    assert!(matches!(out.reason, ReflectReason::Unchanged), "{:?}", out.reason);
    assert_eq!(out.spent_usd, 0.0);
    assert_eq!(fake.0.get(), 0, "no provider call on the brain's own writes");
    assert_eq!(h.as_deref(), Some(pin.as_str()));

    // Control: WITHOUT the pin the same corpus would have paid a call — the pin is
    // load-bearing, not incidental.
    let fake2 = CountingFake(std::cell::Cell::new(0));
    let (out2, _) = reflect_auto_with_client(
        &idx, &fake2, &ReflectConfig::default(), PID, Some(NOW), 3_000, None,
    );
    assert!(matches!(out2.reason, ReflectReason::Ok), "{:?}", out2.reason);
    assert_eq!(fake2.0.get(), 1, "unpinned round pays");
}

#[test]
fn journal_records_the_reverted_flip_and_reject_signature() {
    let store = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = work.path().to_path_buf();
    let db = store.path().join("i.sqlite");

    let p = mk_proposal(ProposalAction::Create, None, "Undone Insight", "here today");
    let rej = reject_signature(ProposalAction::Create, None, "Undone Insight");
    {
        let idx = SqliteIndex::open(&db).unwrap();
        idx.insert_proposal(PID, &p, 1).unwrap();
        idx.apply_proposal(PID, &root, &p.signature, NOW, NOW_MS, true).unwrap();
        assert!(idx.revert_proposal(PID, &root, &p.signature, 2_000).unwrap());
        idx.checkpoint();
    }

    destroy_header(&db);
    let idx = SqliteIndex::open_with_recovery(&db).unwrap();

    // The reverted flip + its reject-signature survived the header wipe via the
    // sidecar journal (same lockstep as the applied flip).
    let ch = change_row(&db, &p.signature);
    assert_eq!(ch.status, "reverted", "reverted flip journaled");
    assert!(idx.is_rejected(PID, &rej).unwrap(), "revert reject-signature journaled");
    assert!(pending_sigs(&db).is_empty(), "nothing resurrected as pending");
}

/// Destroy the SQLite header so the next open rebuilds fresh and the ATTACH salvage
/// reads nothing — only the journal can restore the canonical proposal state.
fn destroy_header(db: &Path) {
    for suffix in ["-wal", "-shm"] {
        let name = format!("{}{suffix}", db.file_name().unwrap().to_string_lossy());
        let _ = std::fs::remove_file(db.with_file_name(name));
    }
    let mut f = std::fs::OpenOptions::new().write(true).open(db).unwrap();
    f.write_all(&[0xFFu8; 100]).unwrap();
    f.flush().unwrap();
}

#[test]
fn journal_records_the_applied_status_flip() {
    let store = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = work.path().to_path_buf();
    let db = store.path().join("i.sqlite");

    let applied = mk_proposal(ProposalAction::Create, None, "Kept Insight", "keep me");
    let still_pending = mk_proposal(ProposalAction::Create, None, "Untouched", "later");
    {
        let idx = SqliteIndex::open(&db).unwrap();
        idx.insert_proposal(PID, &applied, 1).unwrap();
        idx.insert_proposal(PID, &still_pending, 2).unwrap();
        idx.apply_proposal(PID, &root, &applied.signature, NOW, NOW_MS, false).unwrap();
        idx.checkpoint();
    }

    destroy_header(&db);
    let _recovered = SqliteIndex::open_with_recovery(&db).unwrap();

    // After recovery the applied flip was journaled → the approved proposal is NOT
    // pending, while the untouched one replays as pending. (If the flip weren't
    // journaled, replay would restore the approved one as pending and it would appear.)
    let pend = pending_sigs(&db);
    assert!(!pend.contains(&applied.signature), "applied flip survived the wipe");
    assert!(pend.contains(&still_pending.signature), "pending proposal replayed");
}

use koden_lib::modules::brain::gist::artifact::{self, EmitOutcome};
use koden_lib::modules::brain::worker::index_dir;

/// ADR-019 end to end over the real apply pipeline: the gist hook artifact
/// refreshes when the autonomous sweep materializes a memory change (the
/// worker's `auto_apply_sweep` re-emits right after `auto_apply_pending`), an
/// unchanged corpus re-emits as a byte-stable NO-write (the prompt-cache
/// contract), and the injection toggle round-trips on the store singleton.
#[test]
fn gist_artifact_refreshes_on_autonomous_apply_and_stays_byte_stable() {
    let (idx, store, _work, root) = setup();
    let db = store.path().join("i.sqlite");
    std::fs::write(root.join("main.rs"), "fn main() { auth_flow(); }").unwrap();
    write_note(&root, "seed", "", "Seed body.");
    index_dir(&idx, PID, &root);
    scan_project_memory(&idx, PID, &root);

    // First emission; the document is the COMPLETE UserPromptSubmit stdout JSON.
    assert_eq!(artifact::emit(&db, PID, "proj", &root).unwrap(), EmitOutcome::Written);
    let path = artifact::hook_artifact_path(&root);
    let first = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&first).expect("artifact is valid JSON");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
    let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(ctx.contains("Note seed"), "memory layer lists the seeded note: {ctx}");

    // Unchanged memory → byte-stable no-write.
    assert_eq!(artifact::emit(&db, PID, "proj", &root).unwrap(), EmitOutcome::Unchanged);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

    // The autonomous sweep applies a create proposal → the re-emit reflects it.
    let p = mk_proposal(ProposalAction::Create, None, "Fresh Decision", "because.");
    idx.insert_proposal(PID, &p, 1).unwrap();
    assert_eq!(auto_apply_pending(&idx, PID, &root, NOW_MS), 1, "sweep applied");
    // Watcher parity, driven manually: the materialized note FILE is indexed and
    // the notes table re-scanned before the worker's re-emit fires.
    index_dir(&idx, PID, &root);
    scan_project_memory(&idx, PID, &root);
    assert_eq!(artifact::emit(&db, PID, "proj", &root).unwrap(), EmitOutcome::Written);
    let second = std::fs::read_to_string(&path).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert!(
        v2["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("Fresh Decision"),
        "artifact content reflects the applied note"
    );

    // Self-feed guard through the REAL pipeline: a full re-index + re-scan with
    // the artifact on disk indexes/notes nothing new, so the emit stays a no-write
    // and the notes table holds exactly the real notes.
    index_dir(&idx, PID, &root);
    scan_project_memory(&idx, PID, &root);
    assert_eq!(artifact::emit(&db, PID, "proj", &root).unwrap(), EmitOutcome::Unchanged);
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    let mut ids: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["fresh-decision", "seed"], "artifact never becomes a note");

    // The ADR-019 toggle rides the brain_librarian singleton (default ON).
    assert!(idx.inject_gist(), "default ON");
    idx.set_inject_gist(false, 2).unwrap();
    assert!(!idx.inject_gist());
    // OFF: the worker deletes the artifact (worker glue calls this same remove).
    assert!(artifact::remove(&root));
    assert!(!path.exists(), "toggle-off leaves no stale artifact for the hook");
    idx.set_inject_gist(true, 3).unwrap();
    assert!(idx.inject_gist());
}
