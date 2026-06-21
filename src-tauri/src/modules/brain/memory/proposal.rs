//! Memory proposals + the two distinct signature schemes (CONCEPT Flow E/G,
//! EXECUTION_PLAN §0.3). Proposals are human-gated: the Librarian PROPOSES
//! changes to user memory, never auto-applies (deletion always confirmed).
//!
//! Two signatures stay separate (§0.3):
//!  - `proposal_signature` — plain field join; the dedup key (table PK) so the
//!    same proposal isn't queued twice across doctor runs.
//!  - `reject_signature` — djb2 over `scope|action|normalized-title`, persisted
//!    on reject so a declined proposal does not reappear.

/// Apply-op space: 3 proposal variants (create/update/supersede) + `archive`
/// (the preserve-biased apply-op). §0.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalAction {
    Create,
    Update,
    Supersede,
    Archive,
}

impl ProposalAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalAction::Create => "create",
            ProposalAction::Update => "update",
            ProposalAction::Supersede => "supersede",
            ProposalAction::Archive => "archive",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        match s {
            "create" => Some(ProposalAction::Create),
            "update" => Some(ProposalAction::Update),
            "supersede" => Some(ProposalAction::Supersede),
            "archive" => Some(ProposalAction::Archive),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct MemoryProposal {
    /// Owning project id (populated on read; the writer keys by the project arg).
    pub project: String,
    pub signature: String,
    pub action: ProposalAction,
    pub target_id: Option<String>,
    pub title: String,
    pub detail: String,
    pub source: String, // "doctor" | "reflect" | "curate"
    pub status: String, // "pending" | "applied" | "rejected"
}

/// In-memory/PK dedup key — a plain field join (Conductr `proposalSignature`).
pub fn proposal_signature(action: ProposalAction, target_id: Option<&str>, title: &str) -> String {
    format!("{}|{}|{}", action.as_str(), target_id.unwrap_or(""), title)
}

fn normalize_title(t: &str) -> String {
    t.trim().to_lowercase()
}

/// Persisted reject key — djb2 over `scope|action|normalized-title` (Conductr
/// `rejectSignature`). `scope` = the target note id (or "project").
pub fn reject_signature(action: ProposalAction, target_id: Option<&str>, title: &str) -> String {
    djb2(&format!(
        "{}|{}|{}",
        target_id.unwrap_or("project"),
        action.as_str(),
        normalize_title(title)
    ))
}

/// Classic djb2 (`hash * 33 + c`), wrapping. Internal-only — Koden never imports
/// Conductr's persisted signatures, so only self-consistency matters.
fn djb2(s: &str) -> String {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(b as u64);
    }
    format!("{h:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_signature_is_stable_and_distinct() {
        let a = proposal_signature(ProposalAction::Update, Some("n1"), "Fix anchor");
        let b = proposal_signature(ProposalAction::Update, Some("n1"), "Fix anchor");
        let c = proposal_signature(ProposalAction::Supersede, Some("n1"), "Fix anchor");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn reject_signature_normalizes_title_and_is_stable() {
        let a = reject_signature(ProposalAction::Update, Some("n1"), "  Fix Anchor ");
        let b = reject_signature(ProposalAction::Update, Some("n1"), "fix anchor");
        assert_eq!(a, b, "trim + lowercase normalize");
        let d = reject_signature(ProposalAction::Update, Some("n2"), "fix anchor");
        assert_ne!(a, d, "scope changes the signature");
    }

    #[test]
    fn action_roundtrips() {
        for s in ["create", "update", "supersede", "archive"] {
            assert_eq!(ProposalAction::from_token(s).unwrap().as_str(), s);
        }
        assert!(ProposalAction::from_token("bogus").is_none());
    }
}
