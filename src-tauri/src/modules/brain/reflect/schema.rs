//! Faithful port of Conductr's reflect output contract
//! (`Conductr/src/lib/memory/reflect-llm.ts:29-60`). The locked constants, the
//! verbatim SYSTEM_PROMPT (with the `(cap: N)` interpolation), and the loose
//! output schema carry over. The model PROPOSES only — it never writes (the core
//! invariant, `reflect-llm.ts:20-22`); validated items become human-gated
//! `MemoryProposal`s via `super::proposal`.

/// Conductr's locked constants (`reflect-llm.ts:29-31`) — carried verbatim.
pub const MAX_NOTES: usize = 60;
pub const MAX_NOTE_CHARS: usize = 200;
pub const MAX_PROPOSALS: usize = 8;

/// The reflect system prompt (`reflect-llm.ts:33-39` + one deliberate divergence).
/// The `(cap: {MAX_PROPOSALS})` is interpolated so the prompt and the cap stay
/// coupled (em-dashes are U+2014). Built at call time via [system_prompt].
///
/// DIVERGENCE from the Conductr verbatim (2026-07-10, first LIVE paid round):
/// neither Conductr's SYSTEM_PROMPT nor its user digest ever STATED the output
/// schema — "a single JSON object" with no field names. The first real round
/// (gpt-5.4-mini) returned semantically-correct proposals under invented keys
/// (`type`/`text`/`anchors`) → InvalidOutput → the fail-streak would park the
/// Librarian on EVERY provider. The schema line below closes that: exact keys,
/// exact enums, model-agnostic.
pub fn system_prompt() -> String {
    format!(
        "You are a conservative memory librarian for a developer's knowledge base. \
Given a digest of memory notes and a summary of health findings, produce a SMALL \
set of high-confidence proposals (cap: {MAX_PROPOSALS}). Only surface well-supported \
patterns or issues \u{2014} NO speculation, no low-evidence claims. Prefer fewer, \
higher-confidence items over many low-quality ones. Do NOT restate anything already \
captured in the digest or listed under \"Already proposed\" \u{2014} propose only NEW, \
materially different items; a re-wording of an existing note or pending proposal is \
noise, not a proposal. Respond ONLY with a single JSON \
object \u{2014} no prose, no code fences. The object MUST match exactly: \
{{\"proposals\":[{{\"kind\":\"insight\"|\"should_remember\"|\"stale\"|\"conflict\",\
\"title\":\"<short title>\",\"detail\":\"<one-paragraph rationale>\",\
\"scope\":\"global\"|\"project\",\"confidence\":\"low\"|\"medium\"|\"high\",\
\"target\":\"<note id>\"}}]}} \u{2014} add no keys beyond these. For kind \"stale\" or \
\"conflict\", \"target\" MUST be the id of the affected note, copied from the (id: \u{2026}) \
shown on each digest note line; omit \"target\" for \"insight\"/\"should_remember\". Use an \
empty proposals array when nothing is well-supported."
    )
}

/// `kind` enum (`reflect-llm.ts:46`). snake_case keeps `should_remember`'s
/// underscore. Unknown variants fail deserialization → fail-open to `[]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    Insight,
    ShouldRemember,
    Stale,
    Conflict,
}

/// `scope` enum (`reflect-llm.ts:49`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Global,
    Project,
}

/// Shared `low|medium|high` enum (`reflect-llm.ts:51,53-55`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Low,
    Medium,
    High,
}

/// One proposed item. Mirrors `LLM_PROPOSAL_ITEM_SCHEMA` (`reflect-llm.ts:45-56`).
/// serde ignores unknown keys by default → the `z.looseObject` (forward-compat,
/// NOT `deny_unknown_fields`) semantics are preserved. Required: kind/title/detail/
/// scope/confidence. The rest are optional.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ProposalItem {
    pub kind: ProposalKind,
    pub title: String,
    pub detail: String,
    pub scope: Scope,
    pub confidence: Level,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub evidence: Option<Vec<String>>,
    #[serde(default)]
    pub usefulness: Option<Level>,
    #[serde(default)]
    pub risk: Option<Level>,
    #[serde(default, rename = "evidenceQuality")]
    pub evidence_quality: Option<Level>,
    /// Target note id the proposal acts on. The prompt REQUIRES it for stale/conflict
    /// kinds (reflect maps those to Archive/Update, which apply against a real note).
    /// serde default None; empty/whitespace is normalized to None in `to_proposal`.
    #[serde(default)]
    pub target: Option<String>,
}

/// The top-level object (`LLM_PROPOSALS_SCHEMA`, `reflect-llm.ts:58-60`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ProposalsOutput {
    pub proposals: Vec<ProposalItem>,
}

/// Parse + validate the model's raw JSON text. Fail-closed to `Err`:
///  - non-JSON / wrong shape / unknown enum value → parse error,
///  - `proposals.len() > MAX_PROPOSALS` → over-cap rejection.
///
/// Over-cap is a Koden hardening over Conductr (which silently slices to the cap,
/// `reflect-llm.ts:104-108`): a model that ignores the cap it was told is treated
/// as untrustworthy and the whole response is dropped (caller → `InvalidOutput`,
/// fail-open to the deterministic doctor path).
pub fn parse_and_validate(json_text: &str) -> Result<Vec<ProposalItem>, String> {
    let parsed: ProposalsOutput =
        serde_json::from_str(json_text.trim()).map_err(|e| format!("reflect json: {e}"))?;
    if parsed.proposals.len() > MAX_PROPOSALS {
        return Err(format!(
            "reflect returned {} proposals > cap {MAX_PROPOSALS}",
            parsed.proposals.len()
        ));
    }
    Ok(parsed.proposals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_couples_cap_and_uses_em_dash() {
        let p = system_prompt();
        assert!(p.contains(&format!("(cap: {MAX_PROPOSALS})")), "cap interpolated: {p}");
        assert!(p.contains('\u{2014}'), "em-dash U+2014 preserved");
        assert!(p.contains("single JSON object"));
    }

    #[test]
    fn system_prompt_states_target_for_stale_conflict() {
        let p = system_prompt();
        assert!(p.contains("\"target\""), "target key in schema statement: {p}");
        assert!(p.contains("stale") && p.contains("conflict"), "names the target-requiring kinds");
    }

    #[test]
    fn parses_optional_target_present_and_absent() {
        let with = r#"{"proposals":[{"kind":"stale","title":"t","detail":"d","scope":"project","confidence":"high","target":"n7"}]}"#;
        assert_eq!(parse_and_validate(with).unwrap()[0].target.as_deref(), Some("n7"));
        let without = r#"{"proposals":[{"kind":"insight","title":"t","detail":"d","scope":"project","confidence":"high"}]}"#;
        assert_eq!(parse_and_validate(without).unwrap()[0].target, None, "target defaults to None");
    }

    #[test]
    fn parses_required_and_tolerates_unknown_keys() {
        let raw = r#"{"proposals":[
          {"kind":"should_remember","title":"t","detail":"d","scope":"project","confidence":"high","futureField":42}
        ]}"#;
        let items = parse_and_validate(raw).expect("valid");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ProposalKind::ShouldRemember);
        assert_eq!(items[0].scope, Scope::Project);
    }

    #[test]
    fn rejects_missing_required_field() {
        // no `scope`
        let raw = r#"{"proposals":[{"kind":"insight","title":"t","detail":"d","confidence":"low"}]}"#;
        assert!(parse_and_validate(raw).is_err());
    }

    #[test]
    fn rejects_unknown_enum_value() {
        let raw = r#"{"proposals":[{"kind":"bogus","title":"t","detail":"d","scope":"project","confidence":"low"}]}"#;
        assert!(parse_and_validate(raw).is_err());
    }

    #[test]
    fn rejects_over_cap_batch() {
        let one = r#"{"kind":"insight","title":"t","detail":"d","scope":"global","confidence":"low"}"#;
        let raw = format!("{{\"proposals\":[{}]}}", [one; MAX_PROPOSALS + 1].join(","));
        assert!(parse_and_validate(&raw).is_err(), "over-cap rejected");
        let ok = format!("{{\"proposals\":[{}]}}", [one; MAX_PROPOSALS].join(","));
        assert_eq!(parse_and_validate(&ok).unwrap().len(), MAX_PROPOSALS, "at-cap accepted");
    }

    #[test]
    fn empty_proposals_is_valid() {
        assert_eq!(parse_and_validate(r#"{"proposals":[]}"#).unwrap().len(), 0);
    }
}
