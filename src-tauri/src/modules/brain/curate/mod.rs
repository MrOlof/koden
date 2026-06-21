//! V2 Flow G — stale-ADR / memory curation (CONCEPT §6 Flow G, the write-judgment
//! scenario). Free signals (P1 doctor + `superseded_present`) feed the two-stage
//! significance gate ([detect]); decisive candidates ACT (propose the preserve-
//! biased archive for $0), borderline ones ESCALATE to a Tier-2 LLM that classifies
//! and grades the action. The model only ever PROPOSES into the human-gated P1
//! queue (never editing or deleting a user file; deletion is always a human call),
//! and declined proposals stick via the existing reject-signature.
//!
//! Reuses the P4 money path: ONE budget ledger + the `ReflectClient` seam +
//! charge-on-uncertainty. $0-testable via a fake client ([curate_with_client]).

pub mod contradiction;
pub mod detect;
pub mod schema;

use tauri::AppHandle;

use crate::modules::brain::memory::proposal::{
    proposal_signature, reject_signature, MemoryProposal, ProposalAction,
};
use crate::modules::brain::reflect::schema::Level;
use crate::modules::brain::reflect::{self, budget, ReflectClient, ReflectConfig, ReflectReason};
use crate::modules::brain::store::SqliteIndex;

use detect::{Band, Candidate};
use schema::{Classification, CurationVerdict};

/// Why curation returned what it did. NOTE: `Disabled`/`OverBudget`/`NoKey` gate
/// the ESCALATION (paid) path only — the $0 ACT-band proposals are still made.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CurationReason {
    Ok,
    NoCandidates,
    Disabled,
    NoKey,
    OverBudget,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CurationOutcome {
    pub proposals: Vec<MemoryProposal>,
    pub acted: usize,     // ACT-band ($0) proposals enqueued
    pub escalated: usize, // candidates sent to the LLM
    pub spent_usd: f64,
    pub reason: CurationReason,
}

fn level_str(l: Level) -> &'static str {
    match l {
        Level::Low => "low",
        Level::Medium => "medium",
        Level::High => "high",
    }
}

fn classification_str(c: Classification) -> &'static str {
    match c {
        Classification::StillValid => "still-valid",
        Classification::KeepAsHistory => "keep-as-history",
        Classification::Obsolete => "obsolete",
    }
}

fn action_verb(a: ProposalAction) -> &'static str {
    match a {
        ProposalAction::Archive => "Archive",
        ProposalAction::Supersede => "Supersede",
        ProposalAction::Update => "Update",
        ProposalAction::Create => "Create",
    }
}

/// Build + enqueue a `curate`-sourced proposal (dedup by signature; skip if a prior
/// reject-signature is persisted). Returns the proposal iff newly queued.
fn enqueue(
    index: &SqliteIndex,
    project: &str,
    action: ProposalAction,
    note_id: &str,
    title: String,
    detail: String,
    now_ms: i64,
) -> Option<MemoryProposal> {
    let rej = reject_signature(action, Some(note_id), &title);
    if index.is_rejected(project, &rej).unwrap_or(false) {
        return None; // declined before — preserve the human's "no"
    }
    let p = MemoryProposal {
        project: project.to_string(),
        signature: proposal_signature(action, Some(note_id), &title),
        action,
        target_id: Some(note_id.to_string()),
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

/// ACT band: decisive signals → propose the preserve-biased archive for $0 (no LLM).
fn run_act_band(index: &SqliteIndex, project: &str, candidates: &[Candidate], now_ms: i64) -> Vec<MemoryProposal> {
    let mut out = Vec::new();
    for c in candidates.iter().filter(|c| c.band == Band::Act) {
        let title = format!("Archive stale note '{}'", c.note_id);
        let sup = c.superseded_by.as_deref().map(|sb| format!(" Superseded by '{sb}'.")).unwrap_or_default();
        let detail = format!(
            "Signals: {}.{sup} Preserve-biased: keep the file, mark it superseded — old \u{2260} wrong.",
            c.signals.join(", ")
        );
        if let Some(p) = enqueue(index, project, ProposalAction::Archive, &c.note_id, title, detail, now_ms) {
            out.push(p);
        }
    }
    out
}

/// One stale-ADR's bounded digest for the Tier-2 judge (identifiers only).
fn candidate_digest(c: &Candidate) -> String {
    let sup = c.superseded_by.as_deref().map(|sb| format!("superseded_by: {sb}\n")).unwrap_or_default();
    format!(
        "## Stale note\nid: {}\nsignals: {}\n{sup}Classify (still_valid / keep_as_history / obsolete) and recommend a graded action.",
        c.note_id,
        c.signals.join(", ")
    )
}

/// The testable curation core: detect → ACT-band ($0) → ESCALATE-band (budget-gated
/// Tier-2). Same charge-on-uncertainty semantics as reflect.
pub fn curate_with_client(
    index: &SqliteIndex,
    client: &dyn ReflectClient,
    cfg: &ReflectConfig,
    project: &str,
    now_date: Option<&str>,
    now_ms: i64,
) -> CurationOutcome {
    let records = index.list_note_records(project).unwrap_or_default();
    let indexed = index.indexed_path_set(project).unwrap_or_default();
    let candidates = detect::detect_candidates(&records, &indexed, now_date);
    if candidates.is_empty() {
        return CurationOutcome { proposals: Vec::new(), acted: 0, escalated: 0, spent_usd: 0.0, reason: CurationReason::NoCandidates };
    }

    let mut proposals = run_act_band(index, project, &candidates, now_ms);
    let acted = proposals.len();
    let mut escalated = 0usize;
    let mut spent = 0.0f64;
    let mut reason = CurationReason::Ok;

    let system = schema::system_prompt();
    for c in candidates.iter().filter(|c| c.band == Band::Escalate) {
        let user = crate::modules::brain::secrets::redact(&candidate_digest(c)).0;
        let est = reflect::estimate_cost(cfg, &system, &user);
        let rid = match budget::check_and_reserve(index.conn(), &cfg.model, est, now_ms) {
            Ok(id) => id,
            Err(ReflectReason::Disabled) => {
                reason = CurationReason::Disabled;
                break; // ceiling off → stop escalating (ACT-band proposals stand)
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
                // charge the estimate on uncertainty (a partial/billed call may have happened).
                reflect::reconcile_or_log(index, rid, est, now_ms);
                spent += est;
                continue;
            }
        };
        let actual = reflect::actual_cost(cfg, resp.input_tokens, resp.output_tokens);
        let charge = if resp.input_tokens == 0 && resp.output_tokens == 0 { est } else { actual };
        reflect::reconcile_or_log(index, rid, charge, now_ms);
        spent += charge;

        let Ok(v) = schema::parse_verdict(&resp.json_text) else { continue }; // fail-open
        if v.classification == Classification::StillValid {
            continue; // judged still good — no proposal
        }
        if let Some(p) = enqueue_graded(index, project, c, &v, now_ms) {
            proposals.push(p);
        }
    }

    CurationOutcome { proposals, acted, escalated, spent_usd: spent, reason }
}

fn enqueue_graded(
    index: &SqliteIndex,
    project: &str,
    c: &Candidate,
    v: &CurationVerdict,
    now_ms: i64,
) -> Option<MemoryProposal> {
    let action = v.action.to_proposal_action();
    let title = format!("{} stale note '{}'", action_verb(action), c.note_id);
    let detail = format!(
        "Classified {} (confidence {}): {}. Signals: {}.",
        classification_str(v.classification),
        level_str(v.confidence),
        v.reason.trim(),
        c.signals.join(", ")
    );
    enqueue(index, project, action, &c.note_id, title, detail, now_ms)
}

/// ACT-band only — no LLM (no key / escalation unavailable). Detection + the $0
/// preserve-biased archive proposals still run.
pub fn curate_act_only(index: &SqliteIndex, project: &str, now_date: Option<&str>, now_ms: i64) -> CurationOutcome {
    let records = index.list_note_records(project).unwrap_or_default();
    let indexed = index.indexed_path_set(project).unwrap_or_default();
    let candidates = detect::detect_candidates(&records, &indexed, now_date);
    if candidates.is_empty() {
        return CurationOutcome { proposals: Vec::new(), acted: 0, escalated: 0, spent_usd: 0.0, reason: CurationReason::NoCandidates };
    }
    let proposals = run_act_band(index, project, &candidates, now_ms);
    let acted = proposals.len();
    // Escalate candidates exist but there is no key/client to judge them.
    let had_escalate = candidates.iter().any(|c| c.band == Band::Escalate);
    CurationOutcome {
        proposals,
        acted,
        escalated: 0,
        spent_usd: 0.0,
        reason: if had_escalate { CurationReason::NoKey } else { CurationReason::Ok },
    }
}

/// Manual-trigger curation (the real path). Shares reflect's front-door gate: when
/// the paid escalation is disabled (ceiling 0) or has no key, it still runs
/// detection + the $0 ACT band (no client built), stamping the precise reason; only
/// with key + ceiling does it run the full detect→act→escalate flow.
pub fn curate_once(app: &AppHandle, index: &SqliteIndex, project: &str, now_date: Option<&str>, now_ms: i64) -> CurationOutcome {
    let cfg = ReflectConfig::from_librarian(&reflect::librarian::config(index.conn()));
    let ceiling = budget::ceiling(index.conn());
    let account = reflect::librarian::keyring_account_for(&cfg.provider);
    let key = if account.is_empty() {
        None
    } else {
        crate::modules::secrets::read_secret(app, reflect::KEYRING_SERVICE, account)
    };
    let key_present = key.is_some() || reflect::librarian::is_keyless(&cfg.provider);
    if let Some(reason) = reflect::pre_flight(ceiling, key_present) {
        // escalation gated — run the $0 ACT band, then stamp the front-door reason
        // (unless there was nothing to do at all).
        let mut out = curate_act_only(index, project, now_date, now_ms);
        if !matches!(out.reason, CurationReason::NoCandidates) {
            out.reason = match reason {
                ReflectReason::Disabled => CurationReason::Disabled,
                _ => CurationReason::NoKey,
            };
        }
        return out;
    }
    // pre_flight returned None ⇒ ceiling > 0 and the provider's key gate is satisfied.
    let client = reflect::build_client(&cfg, key);
    curate_with_client(index, client.as_ref(), &cfg, project, now_date, now_ms)
}
