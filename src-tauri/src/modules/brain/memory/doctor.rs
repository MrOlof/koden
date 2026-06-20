//! Deterministic memory doctor (CONCEPT Flow G, ADR-006 P1). Reads the structured
//! notes + the index and emits Findings, each of which becomes a human-gated
//! `MemoryProposal`. Pure check logic is separated from persistence for
//! deterministic testing (a controlled `now_date`, no wall clock — §13.21).
//!
//! P1 ships a meaningful, code-grounded SUBSET of checks. The full 18-check port
//! and `TYPED_CHECK_MAP` from Conductr's `doctor.ts` (§0.3) are tracked follow-up
//! work; AST-based anchor validation lands with P2 (for now only path-shaped
//! anchors are validated against the file index).

use std::collections::HashSet;

use crate::modules::brain::memory::proposal::{
    proposal_signature, reject_signature, MemoryProposal, ProposalAction,
};
use crate::modules::brain::store::SqliteIndex;

/// A note as the doctor needs it (richer than `NoteSummary`).
#[derive(Clone, Debug)]
pub struct NoteRecord {
    pub id: String,
    pub note_type: Option<String>,
    pub revalidate_after: Option<String>,
    pub superseded_by: Option<String>,
    pub anchors: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Finding {
    pub check: &'static str,
    pub severity: &'static str,
    pub note_id: Option<String>,
    pub title: String,
    pub detail: String,
    pub action: ProposalAction,
}

fn looks_like_path(anchor: &str) -> bool {
    // Path-shaped anchors only (symbol anchors like `mod::fn` are validated by
    // the AST graph in P2). A '/' is the unambiguous path marker.
    anchor.contains('/')
}

/// Pure check pass. `now_date` is an ISO `YYYY-MM-DD` string (ISO sorts
/// chronologically, so lexical `<` is a correct date comparison); `None` disables
/// the date-dependent staleness check.
pub fn check(
    notes: &[NoteRecord],
    indexed_paths: &HashSet<String>,
    now_date: Option<&str>,
) -> Vec<Finding> {
    let ids: HashSet<&str> = notes.iter().map(|n| n.id.as_str()).collect();
    let mut out = Vec::new();
    for n in notes {
        if n.note_type.is_none() {
            out.push(Finding {
                check: "missing_type",
                severity: "low",
                note_id: Some(n.id.clone()),
                title: format!("Note '{}' has no type", n.id),
                detail: "Add a `type:` (decision/convention/glossary/incident/reference) to the frontmatter.".into(),
                action: ProposalAction::Update,
            });
        }
        if let Some(sb) = &n.superseded_by {
            if !ids.contains(sb.as_str()) {
                out.push(Finding {
                    check: "broken_supersession",
                    severity: "medium",
                    note_id: Some(n.id.clone()),
                    title: format!("Note '{}' superseded_by missing note '{}'", n.id, sb),
                    detail: format!("`superseded_by: {sb}` does not resolve to an existing note."),
                    action: ProposalAction::Update,
                });
            }
        }
        if let (Some(rv), Some(now)) = (&n.revalidate_after, now_date) {
            if rv.as_str() < now {
                out.push(Finding {
                    check: "stale_revalidate",
                    severity: "medium",
                    note_id: Some(n.id.clone()),
                    title: format!("Note '{}' is due for revalidation", n.id),
                    detail: format!("`revalidate_after: {rv}` has passed (today {now})."),
                    action: ProposalAction::Supersede,
                });
            }
        }
        for a in &n.anchors {
            if looks_like_path(a) && !indexed_paths.contains(a) {
                out.push(Finding {
                    check: "broken_anchor",
                    severity: "medium",
                    note_id: Some(n.id.clone()),
                    title: format!("Note '{}' anchor not found: {}", n.id, a),
                    detail: format!("Anchor `{a}` points to a path not in the index (moved or deleted?)."),
                    action: ProposalAction::Update,
                });
            }
        }
    }
    out
}

/// Run the doctor for a project: compute findings, then queue each as a proposal
/// (skipping any whose reject-signature is persisted). Writes via the worker's
/// writer connection. Returns the number of NEW proposals queued.
pub fn run_doctor(
    index: &SqliteIndex,
    project_id: &str,
    now_date: Option<&str>,
    created_ms: i64,
) -> usize {
    let notes = index.list_note_records(project_id).unwrap_or_default();
    let indexed = index.indexed_path_set(project_id).unwrap_or_default();
    let mut created = 0usize;
    for f in check(&notes, &indexed, now_date) {
        let rej = reject_signature(f.action, f.note_id.as_deref(), &f.title);
        if index.is_rejected(project_id, &rej).unwrap_or(false) {
            continue; // declined before — don't resurrect it
        }
        let proposal = MemoryProposal {
            project: project_id.to_string(),
            signature: proposal_signature(f.action, f.note_id.as_deref(), &f.title),
            action: f.action,
            target_id: f.note_id,
            title: f.title,
            detail: f.detail,
            source: "doctor".into(),
            status: "pending".into(),
        };
        if index.insert_proposal(project_id, &proposal, created_ms).unwrap_or(false) {
            created += 1;
        }
    }
    created
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str) -> NoteRecord {
        NoteRecord {
            id: id.into(),
            note_type: Some("decision".into()),
            revalidate_after: None,
            superseded_by: None,
            anchors: vec![],
        }
    }

    #[test]
    fn flags_missing_type() {
        let mut n = note("a");
        n.note_type = None;
        let f = check(&[n], &HashSet::new(), None);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].check, "missing_type");
    }

    #[test]
    fn flags_broken_supersession_and_stale_and_anchor() {
        let mut n = note("a");
        n.superseded_by = Some("ghost".into());
        n.revalidate_after = Some("2020-01-01".into());
        n.anchors = vec!["src/gone.rs".into(), "mod::ok".into()];
        let mut indexed = HashSet::new();
        indexed.insert("src/here.rs".to_string());
        let f = check(&[n], &indexed, Some("2026-06-20"));
        let checks: Vec<&str> = f.iter().map(|x| x.check).collect();
        assert!(checks.contains(&"broken_supersession"));
        assert!(checks.contains(&"stale_revalidate"));
        assert!(checks.contains(&"broken_anchor"));
        // the `mod::ok` symbol anchor is NOT path-shaped → not flagged in P1
        assert_eq!(f.iter().filter(|x| x.check == "broken_anchor").count(), 1);
    }

    #[test]
    fn clean_note_yields_nothing() {
        let f = check(&[note("a")], &HashSet::new(), Some("2026-06-20"));
        assert!(f.is_empty());
    }

    #[test]
    fn stale_disabled_without_now_date() {
        let mut n = note("a");
        n.revalidate_after = Some("2000-01-01".into());
        assert!(check(&[n], &HashSet::new(), None)
            .iter()
            .all(|x| x.check != "stale_revalidate"));
    }
}
