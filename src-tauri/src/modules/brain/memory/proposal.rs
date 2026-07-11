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

/// Token-set Jaccard threshold for the SEMANTIC near-dupe gate at enqueue (distinct
/// from the exact `proposal_signature` PK dedup and the `reject_signature` decline
/// memory — both of which miss re-wordings). Chosen at 0.5 by tuning on the live
/// gauntlet's own examples: run through the whole+camel+stem tokenizer over
/// title+detail, two PARAPHRASES of one fact share ~half their content tokens (e.g.
/// "Stripe webhook verifies signature before parsing" vs "Webhook signature check
/// precedes body parsing" score ≈0.58), while two genuinely DISTINCT facts share few
/// (≈0.0–0.2). 0.5 sits in that gap: high enough not to merge distinct facts, low
/// enough to collapse the re-wordings the title-signature dedup let through. See the
/// two-directional tests below. (Deliberately conservative — a missed dupe is a noisy
/// card; a false merge silently drops a real fact, the worse error.)
pub const NEAR_DUPE_THRESHOLD: f64 = 0.5;

/// The deduped token set a proposal is compared on: its title + detail as one stream.
/// Callers pass the model's RAW detail (not the reviewer-facing decorated form) so the
/// identical "scope/confidence" boilerplate never inflates similarity between two
/// otherwise-distinct proposals — see `reflect::proposal::undecorated_detail`.
pub fn proposal_dedup_set(title: &str, detail: &str) -> std::collections::HashSet<String> {
    crate::modules::brain::tokenize::token_set(&format!("{title} {detail}"))
}

/// True when `candidate` is a semantic near-duplicate of ANY set in `existing`
/// (token-set Jaccard ≥ `threshold`). Deterministic, no I/O — the pure core of the
/// enqueue-time gate, unit-tested in both directions.
pub fn is_near_duplicate(
    candidate: &std::collections::HashSet<String>,
    existing: &[std::collections::HashSet<String>],
    threshold: f64,
) -> bool {
    existing
        .iter()
        .any(|e| crate::modules::brain::tokenize::jaccard(candidate, e) >= threshold)
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
    fn near_dupe_gate_collides_paraphrases_but_not_distinct_facts() {
        // POSITIVE: two re-wordings of ONE fact (the exact class the title-signature
        // dedup let through on the live gauntlet) must collide.
        let a = proposal_dedup_set(
            "Stripe webhook verifies signature before parsing",
            "The Stripe webhook handler verifies the request signature before parsing the payload body.",
        );
        let b = proposal_dedup_set(
            "Webhook signature check precedes body parsing",
            "The webhook signature check precedes parsing of the request payload body in the handler.",
        );
        assert!(
            is_near_duplicate(&b, &[a.clone()], NEAR_DUPE_THRESHOLD),
            "paraphrase must be caught as a near-duplicate"
        );

        // NEGATIVE: a genuinely different fact must NOT be merged into either.
        let c = proposal_dedup_set(
            "Database migrations run automatically on startup",
            "Prisma schema migrations are applied during application boot via the migrate deploy command.",
        );
        assert!(
            !is_near_duplicate(&c, &[a, b], NEAR_DUPE_THRESHOLD),
            "a distinct fact must survive the gate"
        );
    }

    #[test]
    fn near_dupe_gate_empty_existing_never_collides() {
        let cand = proposal_dedup_set("anything", "at all");
        assert!(!is_near_duplicate(&cand, &[], NEAR_DUPE_THRESHOLD), "nothing to collide with");
    }

    #[test]
    fn action_roundtrips() {
        for s in ["create", "update", "supersede", "archive"] {
            assert_eq!(ProposalAction::from_token(s).unwrap().as_str(), s);
        }
        assert!(ProposalAction::from_token("bogus").is_none());
    }
}
