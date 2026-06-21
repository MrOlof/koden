//! Flow G Tier-2 classification output (CONCEPT §6 step 2). The model reads a stale
//! ADR + its tripped signals and CLASSIFIES it, recommending ONE graded action with
//! an archive default-bias. Loose parse (tolerate unknown keys); fail-closed to Err
//! so the caller fails open (no proposal) on a bad verdict.

use crate::modules::brain::memory::proposal::ProposalAction;
use crate::modules::brain::reflect::schema::Level;

/// The model's classification of a stale note.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    StillValid,
    KeepAsHistory,
    Obsolete,
}

/// The graded action — archive is the preserve-biased default; delete is rare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradedAction {
    Archive,
    Supersede,
    Update,
    Delete,
}

impl GradedAction {
    /// Map to the human-gated proposal apply-op. Koden has no `delete` apply-op (the
    /// Librarian never proposes silent deletion of user content); `delete` is
    /// down-graded to the preserve-biased `Archive` (deletion stays a human call).
    pub fn to_proposal_action(self) -> ProposalAction {
        match self {
            GradedAction::Archive | GradedAction::Delete => ProposalAction::Archive,
            GradedAction::Supersede => ProposalAction::Supersede,
            GradedAction::Update => ProposalAction::Update,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CurationVerdict {
    pub classification: Classification,
    pub action: GradedAction,
    pub confidence: Level,
    pub reason: String,
}

/// The curation system prompt — preserve-biased, archive-default (CONCEPT §6, the
/// "old ≠ wrong" rule). U+2014 em-dashes; cap the model to a single JSON object.
pub fn system_prompt() -> String {
    "You are a conservative archivist for a developer's decision records (ADRs/notes). \
Given a note that tripped staleness signals and the current code state, CLASSIFY it as \
one of still_valid, keep_as_history, or obsolete, and recommend ONE graded action: \
archive (DEFAULT BIAS \u{2014} keep the file, mark it superseded), supersede, update, or \
delete (RARE \u{2014} only clearly worthless content). Old is not wrong; prefer preserving \
over destroying. Respond ONLY with a single JSON object \u{2014} no prose, no code fences."
        .to_string()
}

/// Parse + validate the model's verdict. Fail-closed to `Err` (non-JSON / wrong
/// shape / unknown enum) so the caller fails open (no proposal for that candidate).
pub fn parse_verdict(json_text: &str) -> Result<CurationVerdict, String> {
    serde_json::from_str(json_text.trim()).map_err(|e| format!("curation verdict: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_tolerates_unknown_keys() {
        let v = parse_verdict(
            r#"{"classification":"obsolete","action":"supersede","confidence":"high","reason":"replaced","extra":1}"#,
        )
        .unwrap();
        assert_eq!(v.classification, Classification::Obsolete);
        assert_eq!(v.action, GradedAction::Supersede);
    }

    #[test]
    fn delete_downgrades_to_archive_preserve_bias() {
        assert_eq!(GradedAction::Delete.to_proposal_action(), ProposalAction::Archive);
        assert_eq!(GradedAction::Archive.to_proposal_action(), ProposalAction::Archive);
        assert_eq!(GradedAction::Supersede.to_proposal_action(), ProposalAction::Supersede);
        assert_eq!(GradedAction::Update.to_proposal_action(), ProposalAction::Update);
    }

    #[test]
    fn rejects_unknown_enum_and_missing_field() {
        assert!(parse_verdict(r#"{"classification":"bogus","action":"archive","confidence":"low","reason":"x"}"#).is_err());
        assert!(parse_verdict(r#"{"classification":"obsolete","action":"archive","confidence":"low"}"#).is_err());
    }

    #[test]
    fn system_prompt_is_preserve_biased() {
        let p = system_prompt();
        assert!(p.contains("DEFAULT BIAS") && p.contains("Old is not wrong"));
        assert!(p.contains('\u{2014}'));
    }
}
