//! Map a validated LLM `ProposalItem` → a `MemoryProposal`
//! (`reflect-llm.ts:120-160` mapToProposals, adapted). Reflect output flows into
//! the SAME P1 proposal queue + dedup signature as the doctor — the model only
//! ever PROPOSES here; the queue is then applied by the autonomous worker sweep
//! (default mode, snapshot-undo recorded) or by a human approval in 'review'
//! mode (ADR-018).

use crate::modules::brain::memory::proposal::{proposal_signature, MemoryProposal, ProposalAction};

use super::schema::{Level, ProposalItem, ProposalKind, Scope};

/// Reflect's kind → the proposal apply-op. DELIBERATE divergence from Conductr's
/// mapToProposals (`reflect-llm.ts:148-156`, which maps conflict→supersede,
/// stale→update): Koden has no `manual` apply-op and is preserve-biased, so `stale`
/// → Archive (the preserve-biased op, not a content rewrite) and `conflict` →
/// Update (flag the conflict, not an automatic supersede). Every apply-op is
/// revertible from its snapshot (ADR-018), so the mapping stays preserve-biased
/// even under autonomous application.
fn action_for(kind: ProposalKind) -> ProposalAction {
    match kind {
        ProposalKind::Insight | ProposalKind::ShouldRemember => ProposalAction::Create,
        ProposalKind::Stale => ProposalAction::Archive,
        ProposalKind::Conflict => ProposalAction::Update,
    }
}

fn level_str(l: Level) -> &'static str {
    match l {
        Level::Low => "low",
        Level::Medium => "medium",
        Level::High => "high",
    }
}

fn scope_str(s: Scope) -> &'static str {
    match s {
        Scope::Global => "global",
        Scope::Project => "project",
    }
}

/// Build the human-facing detail, carrying the model's reasoning (confidence,
/// scope, evidence) so the reviewer can judge it. Deterministic.
fn detail_for(item: &ProposalItem) -> String {
    let mut s = item.detail.trim().to_string();
    s.push_str(&format!(
        "\n\n(reflect · scope: {} · confidence: {})",
        scope_str(item.scope),
        level_str(item.confidence)
    ));
    if let Some(ev) = &item.evidence {
        if !ev.is_empty() {
            s.push_str("\nEvidence:");
            for e in ev {
                s.push_str(&format!("\n- {e}"));
            }
        }
    }
    s
}

/// The marker [detail_for] appends before the "scope · confidence" decoration.
/// Splitting a STORED detail on it recovers the model's raw rationale for semantic
/// dedup, so the identical decoration (and evidence lines, which follow it) never
/// inflates Jaccard similarity between two distinct proposals.
const DECORATION_MARKER: &str = "\n\n(reflect \u{b7} scope:";

/// The model's raw detail with any reviewer-facing decoration stripped. A no-op for
/// doctor-sourced proposals (the marker is absent), so both proposal sources compare
/// on the same footing.
pub fn undecorated_detail(stored_detail: &str) -> &str {
    stored_detail.split(DECORATION_MARKER).next().unwrap_or(stored_detail)
}

/// Map one validated item to a pending `reflect`-sourced proposal for `project_id`.
/// `target_id` comes from the model's `target` (the note id it named from the digest),
/// trimmed, empty→None. It is REQUIRED by the prompt for stale/conflict (→ Archive/
/// Update, which apply against a note); an Archive/Update proposal that still lacks a
/// valid target is dropped at enqueue (`reflect::finish_response`) rather than stranding
/// as a reject-only card. The target is folded into the signature so two proposals
/// against different notes don't dedup-collide.
pub fn to_proposal(project_id: &str, item: &ProposalItem) -> MemoryProposal {
    let action = action_for(item.kind);
    let title = item.title.trim().to_string();
    let target_id = item
        .target
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    MemoryProposal {
        project: project_id.to_string(),
        signature: proposal_signature(action, target_id.as_deref(), &title),
        action,
        target_id,
        title,
        detail: detail_for(item),
        source: "reflect".into(),
        status: "pending".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: ProposalKind, title: &str) -> ProposalItem {
        ProposalItem {
            kind,
            title: title.into(),
            detail: "body".into(),
            scope: Scope::Project,
            confidence: Level::High,
            project: None,
            evidence: Some(vec!["a.rs".into(), "b.rs".into()]),
            usefulness: None,
            risk: None,
            evidence_quality: None,
            target: None,
        }
    }

    #[test]
    fn kind_maps_to_expected_action() {
        assert_eq!(to_proposal("p", &item(ProposalKind::Insight, "t")).action, ProposalAction::Create);
        assert_eq!(to_proposal("p", &item(ProposalKind::ShouldRemember, "t")).action, ProposalAction::Create);
        assert_eq!(to_proposal("p", &item(ProposalKind::Stale, "t")).action, ProposalAction::Archive);
        assert_eq!(to_proposal("p", &item(ProposalKind::Conflict, "t")).action, ProposalAction::Update);
    }

    #[test]
    fn target_maps_to_target_id_and_folds_into_signature() {
        let mut stale = item(ProposalKind::Stale, "Old note");
        stale.target = Some("  n7  ".into());
        let p = to_proposal("proj", &stale);
        assert_eq!(p.action, ProposalAction::Archive);
        assert_eq!(p.target_id.as_deref(), Some("n7"), "trimmed");
        assert_eq!(
            p.signature,
            proposal_signature(ProposalAction::Archive, Some("n7"), "Old note"),
            "target folded into the dedup signature"
        );
        // Whitespace-only target normalizes to None.
        let mut blank = item(ProposalKind::Insight, "x");
        blank.target = Some("   ".into());
        assert_eq!(to_proposal("proj", &blank).target_id, None);
    }

    #[test]
    fn undecorated_detail_recovers_raw_model_text() {
        // Guards against marker drift: whatever detail_for appends, undecorated_detail
        // must strip it back to the model's raw rationale.
        let p = to_proposal("proj", &item(ProposalKind::Insight, "t"));
        assert!(p.detail.contains("scope:"), "decorated as stored: {}", p.detail);
        assert_eq!(undecorated_detail(&p.detail), "body", "decoration + evidence stripped");
        // A plain (doctor-style) detail with no marker is returned untouched.
        assert_eq!(undecorated_detail("just a plain finding detail"), "just a plain finding detail");
    }

    #[test]
    fn proposal_is_reflect_sourced_pending_with_evidence_in_detail() {
        let p = to_proposal("proj", &item(ProposalKind::Insight, "  Keep it  "));
        assert_eq!(p.source, "reflect");
        assert_eq!(p.status, "pending");
        assert_eq!(p.title, "Keep it", "trimmed");
        assert!(p.detail.contains("confidence: high") && p.detail.contains("- a.rs"), "{}", p.detail);
        assert!(p.target_id.is_none());
        // signature is the standard plain-join (dedup PK).
        assert_eq!(p.signature, proposal_signature(ProposalAction::Create, None, "Keep it"));
    }
}
