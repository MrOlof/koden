//! V2.4 — contradiction detection (CONCEPT Flow G step 1, the LLM-only signal
//! deferred in V2.1). Two memory notes anchored to the SAME code are likely to be
//! about the same decision; if they conflict, one is stale. This is inherently a
//! judgment call (no free heuristic decides it), so it is a paid Tier-2 escalate
//! path — bounded by only comparing CO-ANCHORED pairs (not all O(n²) pairs).
//!
//! Reuses the P4 money path (one budget ledger + ReflectClient + charge-on-
//! uncertainty) and the redact-before-send gate. Secret-safe: the pair digest draws
//! only from already-scan-redacted note metadata (titles + types + anchors); no raw
//! note body is re-read (a redacted body-excerpt comparison is a documented refinement).

use crate::modules::brain::memory::proposal::{proposal_signature, reject_signature, MemoryProposal, ProposalAction};
use crate::modules::brain::memory::NoteSummary;
use crate::modules::brain::reflect::{self, budget, ReflectClient, ReflectConfig, ReflectReason};
use crate::modules::brain::store::{self, SqliteIndex};

use super::{CurationOutcome, CurationReason};

/// Hard cap on judged pairs per pass — bounds spend + latency on a large note set.
const MAX_PAIRS: usize = 24;

/// The Tier-2 contradiction verdict. Loose-parsed; fail-closed to Err → no proposal.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ContradictionVerdict {
    pub contradicts: bool,
    /// Which note id is the STALE one to flag (the model picks; may be absent).
    #[serde(default)]
    pub stale_id: Option<String>,
    #[serde(default)]
    pub reason: String,
}

pub fn system_prompt() -> String {
    "You are a conservative reviewer of a developer's decision records. You are given \
TWO memory notes that are anchored to the SAME code. Decide whether they CONTRADICT \
each other (assert incompatible decisions/conventions about that code). If they do, \
say which note id is the STALE one (prefer the older / superseded framing; keep the \
newer). Be conservative \u{2014} overlap or elaboration is NOT contradiction. Respond \
ONLY with a single JSON object: {\"contradicts\": bool, \"stale_id\": string|null, \
\"reason\": string}."
        .to_string()
}

pub fn parse_verdict(json_text: &str) -> Result<ContradictionVerdict, String> {
    serde_json::from_str(json_text.trim()).map_err(|e| format!("contradiction verdict: {e}"))
}

/// Co-anchored note pairs (share ≥1 anchor), deterministic + capped. Returns index
/// pairs `(i, j)` with `i < j` into `notes` (sorted by id upstream) + their shared anchors.
pub fn contradiction_pairs(notes: &[NoteSummary]) -> Vec<(usize, usize, Vec<String>)> {
    let mut pairs = Vec::new();
    for i in 0..notes.len() {
        // Hoisted: build note i's anchor set ONCE, not once per inner j.
        let ai: std::collections::BTreeSet<&str> = notes[i].anchors.iter().map(String::as_str).collect();
        if ai.is_empty() {
            continue;
        }
        for (j, nj) in notes.iter().enumerate().skip(i + 1) {
            let shared: Vec<String> =
                nj.anchors.iter().filter(|a| ai.contains(a.as_str())).cloned().collect();
            if !shared.is_empty() {
                pairs.push((i, j, shared));
            }
        }
    }
    // Judge the MOST co-anchored pairs first: more shared anchors ⇒ likelier a real
    // contradiction, and a pair sharing only one common/hub anchor (e.g. package.json)
    // sinks below pairs sharing several specific ones. Tie-break by (i, j) so the cap
    // is deterministic for identical input.
    pairs.sort_by(|a, b| b.2.len().cmp(&a.2.len()).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    if pairs.len() > MAX_PAIRS {
        log::warn!(
            "contradiction: {} co-anchored pairs; judging top {MAX_PAIRS} by shared-anchor count (rest skipped this pass)",
            pairs.len()
        );
        pairs.truncate(MAX_PAIRS);
    }
    pairs
    // ponytail: relevance ranking deprioritizes hub-anchor pairs but doesn't drop them;
    // add an anchor-frequency filter (skip anchors shared by >K notes) if hub anchors
    // measurably waste budget.
}

/// Bounded, redacted digest of one co-anchored pair (metadata only).
fn pair_digest(a: &NoteSummary, b: &NoteSummary, shared: &[String]) -> String {
    let line = |n: &NoteSummary| {
        format!("id={} type={} title={}", n.id, n.note_type.as_deref().unwrap_or("note"), n.title)
    };
    format!(
        "## Note A\n{}\n## Note B\n{}\n## Shared anchors\n{}\nDo A and B contradict? If so, which id is stale?",
        line(a),
        line(b),
        shared.join(", ")
    )
}

/// The testable core: co-anchored pairs → budget-gated Tier-2 contradiction judgment
/// → an Update proposal flagging the stale note (human-gated). Mirrors curate's
/// charge-on-uncertainty + reject-signature discipline.
pub fn curate_contradictions_with_client(
    index: &SqliteIndex,
    client: &dyn ReflectClient,
    cfg: &ReflectConfig,
    project: &str,
    now_ms: i64,
) -> CurationOutcome {
    let notes = store::list_notes_with_conn(index.conn(), Some(project)).unwrap_or_default();
    let pairs = contradiction_pairs(&notes);
    if pairs.is_empty() {
        return CurationOutcome { proposals: Vec::new(), acted: 0, escalated: 0, spent_usd: 0.0, reason: CurationReason::NoCandidates };
    }

    let system = system_prompt();
    let mut proposals = Vec::new();
    let mut escalated = 0usize;
    let mut spent = 0.0f64;
    let mut reason = CurationReason::Ok;

    for (i, j, shared) in pairs {
        let user = crate::modules::brain::secrets::redact(&pair_digest(&notes[i], &notes[j], &shared)).0;
        let est = reflect::estimate_cost(cfg, &system, &user);
        let rid = match budget::check_and_reserve(index.conn(), &cfg.model, est, now_ms) {
            Ok(id) => id,
            Err(ReflectReason::Disabled) => {
                reason = CurationReason::Disabled;
                break;
            }
            Err(ReflectReason::OverBudget) => {
                reason = CurationReason::OverBudget;
                break;
            }
            Err(_) => break,
        };
        escalated += 1;
        let resp = match client.complete(&cfg.model, &system, &user, cfg.max_output_tokens) {
            Ok(r) => r,
            Err(_) => {
                reflect::reconcile_or_log(index, rid, est, now_ms);
                spent += est;
                continue;
            }
        };
        let actual = reflect::actual_cost(&cfg.model, resp.input_tokens, resp.output_tokens);
        let charge = if resp.input_tokens == 0 && resp.output_tokens == 0 { est } else { actual };
        reflect::reconcile_or_log(index, rid, charge, now_ms);
        spent += charge;

        let Ok(v) = parse_verdict(&resp.json_text) else { continue };
        if !v.contradicts {
            continue;
        }
        // FAIL CLOSED: only enqueue when the model names WHICH note is stale AND it's
        // one of this pair. NoteSummary drops `created`, so we can't pick the older
        // note ourselves — guessing (the old "note B by convention") risks flagging
        // the CORRECT note. No valid pick ⇒ no proposal.
        let Some(stale) = v
            .stale_id
            .as_deref()
            .filter(|s| *s == notes[i].id.as_str() || *s == notes[j].id.as_str())
        else {
            continue;
        };
        let other = if stale == notes[i].id.as_str() { &notes[j].id } else { &notes[i].id };
        if let Some(p) = enqueue_contradiction(index, project, stale, other, v.reason.trim(), now_ms) {
            proposals.push(p);
        }
    }

    CurationOutcome { proposals, acted: 0, escalated, spent_usd: spent, reason }
}

fn enqueue_contradiction(
    index: &SqliteIndex,
    project: &str,
    stale_id: &str,
    other_id: &str,
    reason: &str,
    now_ms: i64,
) -> Option<MemoryProposal> {
    let action = ProposalAction::Update; // flag for human resolution; never auto-edit
    let title = format!("Resolve contradiction in note '{stale_id}'");
    let detail = format!("Note '{stale_id}' appears to contradict co-anchored note '{other_id}': {reason}");
    let rej = reject_signature(action, Some(stale_id), &title);
    if index.is_rejected(project, &rej).unwrap_or(false) {
        return None;
    }
    let p = MemoryProposal {
        project: project.to_string(),
        signature: proposal_signature(action, Some(stale_id), &title),
        action,
        target_id: Some(stale_id.to_string()),
        title,
        detail,
        source: "curate".into(),
        status: "pending".into(),
    };
    if index.insert_proposal(project, &p, now_ms).unwrap_or(false) {
        Some(p)
    } else {
        None
    }
}

/// Real wrapper: shares the front-door gate (no client built when the ceiling is off
/// or no key — contradiction detection is purely paid, so it returns a noop reason).
pub fn curate_contradictions_once(app: &tauri::AppHandle, index: &SqliteIndex, project: &str, now_ms: i64) -> CurationOutcome {
    let cfg = ReflectConfig::default();
    let ceiling = budget::ceiling(index.conn());
    let key = crate::modules::secrets::read_secret(app, reflect::KEYRING_SERVICE, reflect::KEYRING_ACCOUNT);
    if let Some(reason) = reflect::pre_flight(ceiling, key.is_some()) {
        let r = match reason {
            ReflectReason::Disabled => CurationReason::Disabled,
            _ => CurationReason::NoKey,
        };
        return CurationOutcome { proposals: Vec::new(), acted: 0, escalated: 0, spent_usd: 0.0, reason: r };
    }
    let Some(k) = key else {
        return CurationOutcome { proposals: Vec::new(), acted: 0, escalated: 0, spent_usd: 0.0, reason: CurationReason::NoKey };
    };
    let client = reflect::llm::AnthropicClient::new(k);
    curate_contradictions_with_client(index, &client, &cfg, project, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, anchors: &[&str]) -> NoteSummary {
        NoteSummary {
            id: id.into(),
            title: format!("Title {id}"),
            note_type: Some("decision".into()),
            status: None,
            path: format!(".koden-memory/{id}.md"),
            anchors: anchors.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn pairs_only_co_anchored_deterministic() {
        let notes = vec![
            note("a", &["src/x.rs"]),
            note("b", &["src/x.rs", "src/y.rs"]), // shares src/x.rs with a
            note("c", &["src/z.rs"]),             // shares nothing
        ];
        let pairs = contradiction_pairs(&notes);
        assert_eq!(pairs.len(), 1, "only a↔b are co-anchored");
        assert_eq!((pairs[0].0, pairs[0].1), (0, 1));
        assert_eq!(pairs[0].2, vec!["src/x.rs".to_string()]);
    }

    #[test]
    fn no_anchors_no_pairs() {
        let notes = vec![note("a", &[]), note("b", &[])];
        assert!(contradiction_pairs(&notes).is_empty());
    }

    #[test]
    fn verdict_parse_tolerant_and_failclosed() {
        let v = parse_verdict(r#"{"contradicts":true,"stale_id":"a","reason":"x","extra":1}"#).unwrap();
        assert!(v.contradicts && v.stale_id.as_deref() == Some("a"));
        assert!(parse_verdict("not json").is_err());
    }
}
