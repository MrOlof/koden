//! P4 — budgeted LLM reflect: the ONLY token-spending path in Koden Brain.
//! Opt-in, default-$0, hard pre-flight budget gate, single-flight, fail-open.
//! The model PROPOSES memory cleanups; a human approves them via the P1 queue —
//! reflect NEVER writes user memory and NEVER spends without a durable reservation
//! passing the ceiling check first.
//!
//! Testability: the Anthropic call sits behind [ReflectClient] so the whole
//! pipeline (digest → budget → map → enqueue) runs offline/$0/deterministically
//! against a fake client (`reflect_with_client`). [reflect_once] is the thin real
//! wrapper that resolves the key + builds the real client ([llm::AnthropicClient]).

pub mod budget;
pub mod digest;
pub mod llm;
pub mod proposal;
pub mod schema;

use tauri::AppHandle;

use crate::modules::brain::memory::doctor;
use crate::modules::brain::memory::proposal::reject_signature;
use crate::modules::brain::store::{self, SqliteIndex};

/// Keyring location of the daemon's Anthropic key (mirrors the frontend
/// `ai/config.ts`: service `koden-ai`, account `anthropic-api-key`).
pub const KEYRING_SERVICE: &str = "koden-ai";
pub const KEYRING_ACCOUNT: &str = "anthropic-api-key";

/// Default cheap reflect model (Haiku). Opus is opt-in via config. The pinned id
/// (`claude-haiku-4-5-20251001`) resolves from this alias against the live API;
/// the offline sandbox uses a fake client so the id only matters for the real smoke.
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5";
const MAX_OUTPUT_TOKENS: u32 = 2048;
/// Conservative token estimate for the pre-flight RESERVE: chars/3 over-counts vs
/// the ~chars/4 rule, so the ceiling errs toward blocking early / over-charging a
/// crash, never under-reserving (EXECUTION_PLAN appendix "conservative divisor").
const EST_CHARS_PER_TOKEN: usize = 3;

/// Why reflect returned what it returned (EXECUTION_PLAN §4.1.2).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectReason {
    Ok,
    Disabled,           // ceiling == 0.0 (default off)
    NoKey,              // keyring koden-ai empty
    OverBudget,         // pre-flight reserve would exceed ceiling
    EmptyCorpus,        // nothing to reflect on
    CallFailed(String), // network/HTTP/timeout — fail-open to []
    InvalidOutput,      // model JSON rejected by schema validation
}

/// The result of a reflect attempt.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReflectOutcome {
    pub proposals: Vec<crate::modules::brain::memory::proposal::MemoryProposal>,
    pub spent_usd: f64,
    pub reason: ReflectReason,
}

impl ReflectOutcome {
    fn noop(reason: ReflectReason) -> Self {
        Self { proposals: Vec::new(), spent_usd: 0.0, reason }
    }
}

/// Runtime reflect config (model + caps). `default()` is the cheap Haiku path.
#[derive(Clone, Debug)]
pub struct ReflectConfig {
    pub model: String,
    pub max_output_tokens: u32,
    pub max_proposals: usize,
}

impl Default for ReflectConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_proposals: schema::MAX_PROPOSALS,
        }
    }
}

/// One model's ($/token in, $/token out). Unknown models price at Opus (the
/// expensive tier) so an unrecognized id never UNDER-estimates the budget.
fn pricing(model: &str) -> (f64, f64) {
    if model.starts_with("claude-haiku-4-5") {
        (1.0 / 1_000_000.0, 5.0 / 1_000_000.0)
    } else {
        // Opus 4.8 AND any unrecognized id price at the expensive tier, so an
        // unknown model can never UNDER-estimate the budget.
        (5.0 / 1_000_000.0, 25.0 / 1_000_000.0)
    }
}

/// Conservative pre-flight cost estimate: input tokens ≈ (system+user) chars/3
/// (over-count), output tokens = the full `max_output_tokens` cap. `pub(crate)` so
/// the P4 reflect path AND the V2 curation path share ONE pricing source of truth.
pub(crate) fn estimate_cost(cfg: &ReflectConfig, system: &str, user: &str) -> f64 {
    let (in_rate, out_rate) = pricing(&cfg.model);
    let in_tok = (system.len() + user.len()).div_ceil(EST_CHARS_PER_TOKEN) as f64;
    in_tok * in_rate + cfg.max_output_tokens as f64 * out_rate
}

/// Actual cost from the API's reported usage (reconcile path). Shared with curation.
pub(crate) fn actual_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (in_rate, out_rate) = pricing(model);
    input_tokens as f64 * in_rate + output_tokens as f64 * out_rate
}

/// Pure pre-flight gate (testable without an `AppHandle`/keyring): `Disabled` when
/// the ceiling is off, `NoKey` when the key is absent, else proceed. Shared with
/// the curation path (the same daily ceiling gates both token-spending flows).
pub(crate) fn pre_flight(ceiling_usd: f64, key_present: bool) -> Option<ReflectReason> {
    if ceiling_usd <= 0.0 {
        Some(ReflectReason::Disabled)
    } else if !key_present {
        Some(ReflectReason::NoKey)
    } else {
        None
    }
}

/// One model response: the raw JSON text + reported token usage (for reconcile).
#[derive(Clone)]
pub struct ReflectResponse {
    pub json_text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The seam the budget/digest/map pipeline calls. The real impl is
/// [llm::AnthropicClient]; tests inject a deterministic fake (§13.22).
pub trait ReflectClient {
    fn complete(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ReflectResponse, String>;
}

/// The testable reflect core: digest → reserve → call → reconcile → map → enqueue.
/// All token spend goes through [budget]; the writer index is the single writer.
pub fn reflect_with_client(
    index: &SqliteIndex,
    client: &dyn ReflectClient,
    cfg: &ReflectConfig,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
) -> ReflectOutcome {
    let notes = store::list_notes_with_conn(index.conn(), Some(project_id)).unwrap_or_default();
    if notes.is_empty() {
        return ReflectOutcome::noop(ReflectReason::EmptyCorpus);
    }
    let records = index.list_note_records(project_id).unwrap_or_default();
    let indexed = index.indexed_path_set(project_id).unwrap_or_default();
    let findings = doctor::check(&records, &indexed, now_date);

    let system = schema::system_prompt();
    // Belt-and-suspenders secret gate (§7.1): redact the ENTIRE assembled message
    // immediately before it can reach the cloud, so no single un-redacted field
    // (anchors, a finding detail interpolating raw frontmatter, etc.) can leak —
    // even though anchors/titles are already redacted at scan.
    let user = crate::modules::brain::secrets::redact(&digest::build_user_message(&notes, &findings)).0;
    let est = estimate_cost(cfg, &system, &user);

    // Pre-flight reserve — the durable, atomic ceiling gate (no call if it fails).
    let rid = match budget::check_and_reserve(index.conn(), &cfg.model, est, now_ms) {
        Ok(id) => id,
        Err(reason) => return ReflectOutcome::noop(reason),
    };

    // The one network call. On ANY error, charge the estimate (a partial/billed
    // call may have happened — default to charging on uncertainty, §4.1.11).
    let resp = match client.complete(&cfg.model, &system, &user, cfg.max_output_tokens) {
        Ok(r) => r,
        Err(e) => {
            reconcile_or_log(index, rid, est, now_ms);
            return ReflectOutcome { proposals: Vec::new(), spent_usd: est, reason: ReflectReason::CallFailed(e) };
        }
    };

    // Charge the ACTUAL reported cost — but a 2xx with missing/garbled usage
    // deserializes to 0/0 tokens, and Anthropic still bills the input. Floor an
    // implausible 0/0 to the conservative estimate so a success never under-charges.
    let actual = actual_cost(&cfg.model, resp.input_tokens, resp.output_tokens);
    let charge = if resp.input_tokens == 0 && resp.output_tokens == 0 { est } else { actual };
    reconcile_or_log(index, rid, charge, now_ms);

    let items = match schema::parse_and_validate(&resp.json_text) {
        Ok(v) => v,
        Err(_) => return ReflectOutcome { proposals: Vec::new(), spent_usd: charge, reason: ReflectReason::InvalidOutput },
    };

    // Map → enqueue into the SAME P1 queue (dedup by signature; skip rejected).
    // parse_and_validate already hard-rejects > MAX_PROPOSALS; the take is a
    // defensive belt against a future config raising the cap above the parse limit.
    let mut enqueued = Vec::new();
    for item in items.iter().take(cfg.max_proposals) {
        let p = proposal::to_proposal(project_id, item);
        let rej = reject_signature(p.action, p.target_id.as_deref(), &p.title);
        if index.is_rejected(project_id, &rej).unwrap_or(false) {
            continue; // declined before — don't resurrect it
        }
        if index.insert_proposal(project_id, &p, now_ms).unwrap_or(false) {
            enqueued.push(p);
        }
    }
    ReflectOutcome { proposals: enqueued, spent_usd: charge, reason: ReflectReason::Ok }
}

/// Reconcile a reservation, logging (not swallowing) a failure. A stranded
/// 'reserved' row is still counted against the ceiling by the next
/// [budget::check_and_reserve] and folded by the boot sweep, so a failed reconcile
/// can never under-enforce the ceiling — but it must be visible in the log. Shared
/// with the curation path so both token-spending flows reconcile identically.
pub(crate) fn reconcile_or_log(index: &SqliteIndex, reservation_id: i64, charge_usd: f64, now_ms: i64) {
    if let Err(e) = budget::reconcile(index.conn(), reservation_id, charge_usd, now_ms) {
        log::warn!("brain: reflect budget reconcile failed ({e}); reservation {reservation_id} left for the boot sweep");
    }
}

/// Manual-trigger reflect (the real path). Resolves the ceiling + key, builds the
/// real Anthropic client, and runs [reflect_with_client]. Never on a timer.
pub fn reflect_once(
    app: &AppHandle,
    index: &SqliteIndex,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
) -> ReflectOutcome {
    let cfg = ReflectConfig::default();
    let ceiling_usd = budget::ceiling(index.conn());
    let key = crate::modules::secrets::read_secret(app, KEYRING_SERVICE, KEYRING_ACCOUNT);
    if let Some(reason) = pre_flight(ceiling_usd, key.is_some()) {
        return ReflectOutcome::noop(reason);
    }
    // pre_flight returned None ⇒ key is Some; let-else over expect() so a future
    // reorder can never panic the worker here.
    let Some(api_key) = key else {
        return ReflectOutcome::noop(ReflectReason::NoKey);
    };
    let client = llm::AnthropicClient::new(api_key);
    reflect_with_client(index, &client, &cfg, project_id, now_date, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_flight_gates_disabled_then_nokey() {
        assert!(matches!(pre_flight(0.0, true), Some(ReflectReason::Disabled)));
        assert!(matches!(pre_flight(1.0, false), Some(ReflectReason::NoKey)));
        assert!(pre_flight(1.0, true).is_none());
    }

    #[test]
    fn estimate_is_conservative_and_unknown_model_prices_high() {
        let cfg = ReflectConfig::default();
        let haiku = estimate_cost(&cfg, "sys", "user");
        let opus_cfg = ReflectConfig { model: "claude-opus-4-8".into(), ..cfg.clone() };
        let opus = estimate_cost(&opus_cfg, "sys", "user");
        assert!(opus > haiku, "opus pricier");
        let unknown_cfg = ReflectConfig { model: "mystery".into(), ..cfg };
        // unknown prices at opus, never cheaper.
        assert!((estimate_cost(&unknown_cfg, "sys", "user") - opus).abs() < 1e-12);
    }
}
