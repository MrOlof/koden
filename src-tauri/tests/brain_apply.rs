//! D2 — approve APPLIES the proposal to project memory. These tests drive the real
//! store method (`SqliteIndex::apply_proposal`) against temp stores + temp project
//! dirs, and assert the end-to-end effect: the `.koden-memory/*.md` FILE materializes/
//! changes, the notes TABLE reflects it, a re-scan is idempotent, a missing target
//! soft-fails leaving the proposal pending, reject writes a reject-signature and
//! materializes nothing, and the status flip is journaled (survives a header wipe).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use koden_lib::modules::brain::memory::proposal::{
    proposal_signature, reject_signature, MemoryProposal, ProposalAction,
};
use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::store::{list_notes_readonly, list_proposals_readonly, SqliteIndex};

const PID: &str = "p";
const NOW: &str = "2026-07-10";

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

    idx.apply_proposal(PID, &root, &p.signature, NOW).unwrap();

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
    idx.apply_proposal(PID, &root, &p.signature, NOW).unwrap();
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

    idx.apply_proposal(PID, &root, &p.signature, NOW).unwrap();

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

    idx.apply_proposal(PID, &root, &p.signature, NOW).unwrap();

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

    idx.apply_proposal(PID, &root, &p.signature, NOW).unwrap();

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

    let err = idx.apply_proposal(PID, &root, &p.signature, NOW).unwrap_err();
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
        idx.apply_proposal(PID, &root, &applied.signature, NOW).unwrap();
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
