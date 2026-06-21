//! Map a validated LLM `ProposalItem` → a human-gated `MemoryProposal`
//! (`reflect-llm.ts:120-160` mapToProposals, adapted). Reflect output flows into
//! the SAME P1 proposal queue + dedup signature as the doctor — the model only
//! ever PROPOSES; a human approves before any user file changes.

use crate::modules::brain::memory::proposal::{proposal_signature, MemoryProposal, ProposalAction};

use super::schema::{Level, ProposalItem, ProposalKind, Scope};

/// Reflect's kind → the proposal apply-op. `stale` archives (preserve-biased),
/// `conflict` updates, `insight`/`should_remember` create.
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

/// Map one validated item to a pending `reflect`-sourced proposal for `project_id`.
/// `target_id` is `None`: the model does not reliably reference an existing note id,
/// so reflect proposes against the project and the human resolves the target.
pub fn to_proposal(project_id: &str, item: &ProposalItem) -> MemoryProposal {
    let action = action_for(item.kind);
    let title = item.title.trim().to_string();
    MemoryProposal {
        project: project_id.to_string(),
        signature: proposal_signature(action, None, &title),
        action,
        target_id: None,
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
