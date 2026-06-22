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
pub mod librarian;
pub mod llm;
pub mod llm_openai;
pub mod proposal;
pub mod schema;

use tauri::AppHandle;

use crate::modules::brain::freshness::hash;
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
    Unchanged,          // digest byte-identical to the last pass → delta gate skipped the call ($0)
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

/// Runtime reflect config (model + caps + provider + per-token rates). `default()`
/// is the cheap Anthropic Haiku path (rates match `librarian`'s defaults); the live
/// path builds this from the persisted [librarian::LibrarianConfig] via
/// [ReflectConfig::from_librarian].
#[derive(Clone, Debug)]
pub struct ReflectConfig {
    pub model: String,
    pub max_output_tokens: u32,
    pub max_proposals: usize,
    /// Provider id (matches the frontend `ProviderId`): `anthropic` uses the native
    /// Anthropic client; anything else uses the OpenAI-compatible client.
    pub provider: String,
    /// Explicit OpenAI-compatible base URL (incl. `/v1`); empty = the canonical
    /// per-provider URL. Ignored for the Anthropic provider.
    pub base_url: String,
    /// $/token input + output rates (already converted from the $/Mtok selection).
    /// A free local model carries 0 here, so its spend never moves the meter.
    pub in_rate: f64,
    pub out_rate: f64,
}

impl Default for ReflectConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_proposals: schema::MAX_PROPOSALS,
            provider: "anthropic".to_string(),
            base_url: String::new(),
            in_rate: 1.0 / 1_000_000.0,
            out_rate: 5.0 / 1_000_000.0,
        }
    }
}

impl ReflectConfig {
    /// Build from the persisted Librarian selection (converts $/Mtok → $/token).
    pub(crate) fn from_librarian(l: &librarian::LibrarianConfig) -> Self {
        Self {
            model: l.model.clone(),
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_proposals: schema::MAX_PROPOSALS,
            provider: l.provider.clone(),
            base_url: l.base_url.clone(),
            in_rate: l.in_rate_mtok / 1_000_000.0,
            out_rate: l.out_rate_mtok / 1_000_000.0,
        }
    }
}

/// Build the [ReflectClient] for the configured provider: the native Anthropic
/// client, or the OpenAI-compatible client (with the resolved base URL). Shared by
/// the reflect AND curation call sites so they stay in lockstep.
pub(crate) fn build_client(cfg: &ReflectConfig, key: Option<String>) -> Box<dyn ReflectClient> {
    if cfg.provider == "anthropic" {
        Box::new(llm::AnthropicClient::new(key.unwrap_or_default()))
    } else {
        let base = if cfg.base_url.trim().is_empty() {
            librarian::canonical_base_url(&cfg.provider).to_string()
        } else {
            cfg.base_url.clone()
        };
        Box::new(llm_openai::OpenAiCompatClient::new(key, base))
    }
}

/// Conservative pre-flight cost estimate: input tokens ≈ (system+user) chars/3
/// (over-count), output tokens = the full `max_output_tokens` cap. Rates come from
/// the selected model's config so the P4 reflect path AND the V2 curation path share
/// ONE pricing source of truth (a free local model estimates 0).
pub(crate) fn estimate_cost(cfg: &ReflectConfig, system: &str, user: &str) -> f64 {
    let in_tok = (system.len() + user.len()).div_ceil(EST_CHARS_PER_TOKEN) as f64;
    in_tok * cfg.in_rate + cfg.max_output_tokens as f64 * cfg.out_rate
}

/// Actual cost from the API's reported usage (reconcile path). Shared with curation.
pub(crate) fn actual_cost(cfg: &ReflectConfig, input_tokens: u64, output_tokens: u64) -> f64 {
    input_tokens as f64 * cfg.in_rate + output_tokens as f64 * cfg.out_rate
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
    let system = schema::system_prompt();
    // The corpus digest (notes + structural findings), already secret-redacted —
    // see build_digest. EmptyCorpus when the project has no notes to reflect on.
    let Some(user) = build_digest(index, project_id, now_date) else {
        return ReflectOutcome::noop(ReflectReason::EmptyCorpus);
    };
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
    let actual = actual_cost(cfg, resp.input_tokens, resp.output_tokens);
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

/// Build the exact redacted user digest reflect would send for a project (memory
/// notes + structural doctor findings), or None for an empty corpus. The
/// belt-and-suspenders secret gate (§7.1) redacts the ENTIRE assembled message
/// here, immediately before it could reach the cloud. Shared by [reflect_with_client]
/// and the autonomous delta gate so the gate's hash matches what's actually sent.
pub(crate) fn build_digest(index: &SqliteIndex, project_id: &str, now_date: Option<&str>) -> Option<String> {
    let notes = store::list_notes_with_conn(index.conn(), Some(project_id)).unwrap_or_default();
    if notes.is_empty() {
        return None;
    }
    let records = index.list_note_records(project_id).unwrap_or_default();
    let indexed = index.indexed_path_set(project_id).unwrap_or_default();
    let findings = doctor::check(&records, &indexed, now_date);
    Some(crate::modules::brain::secrets::redact(&digest::build_user_message(&notes, &findings)).0)
}

/// Delta-gated reflect core (testable offline): build the digest, and SKIP the model
/// call when it's byte-identical to `prev_digest_hash` (nothing material changed →
/// Unchanged, $0). Otherwise run [reflect_with_client]. Returns the current digest
/// hash (None only on an empty corpus) so the caller can remember it for next time.
pub fn reflect_auto_with_client(
    index: &SqliteIndex,
    client: &dyn ReflectClient,
    cfg: &ReflectConfig,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
    prev_digest_hash: Option<&str>,
) -> (ReflectOutcome, Option<String>) {
    let Some(user) = build_digest(index, project_id, now_date) else {
        return (ReflectOutcome::noop(ReflectReason::EmptyCorpus), None);
    };
    let digest_hash = hash::hash_bytes(user.as_bytes());
    if prev_digest_hash == Some(digest_hash.as_str()) {
        return (ReflectOutcome::noop(ReflectReason::Unchanged), Some(digest_hash));
    }
    (
        reflect_with_client(index, client, cfg, project_id, now_date, now_ms),
        Some(digest_hash),
    )
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

/// Manual-trigger reflect (the real path): always builds + sends if the budget/key
/// gates pass. Thin wrapper over [reflect_auto] with no prior digest, so a manual
/// click never short-circuits on "unchanged".
pub fn reflect_once(
    app: &AppHandle,
    index: &SqliteIndex,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
) -> ReflectOutcome {
    reflect_auto(app, index, project_id, now_date, now_ms, None).0
}

/// Autonomous-trigger reflect: resolves the persisted Librarian model + ceiling +
/// provider key, builds the right client, then runs the delta-gated core.
/// `prev_digest_hash` is the hash of the last digest we reflected on; an unchanged
/// digest skips the paid call ($0). Returns the current digest hash so the caller
/// can remember it. Budget/key-gated exactly like [reflect_once].
pub fn reflect_auto(
    app: &AppHandle,
    index: &SqliteIndex,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
    prev_digest_hash: Option<&str>,
) -> (ReflectOutcome, Option<String>) {
    let cfg = ReflectConfig::from_librarian(&librarian::config(index.conn()));
    let ceiling_usd = budget::ceiling(index.conn());
    let account = librarian::keyring_account_for(&cfg.provider);
    let key = if account.is_empty() {
        None
    } else {
        crate::modules::secrets::read_secret(app, KEYRING_SERVICE, account)
    };
    // Keyless local providers (ollama/lmstudio/mlx/openai-compatible) satisfy the
    // key gate with no key; everything else needs the provider's keyring entry.
    let key_present = key.is_some() || librarian::is_keyless(&cfg.provider);
    if let Some(reason) = pre_flight(ceiling_usd, key_present) {
        return (ReflectOutcome::noop(reason), None);
    }
    let client = build_client(&cfg, key);
    reflect_auto_with_client(index, client.as_ref(), &cfg, project_id, now_date, now_ms, prev_digest_hash)
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
    fn estimate_scales_with_rates_and_is_zero_when_free() {
        let cheap = ReflectConfig::default();
        let pricey = ReflectConfig { in_rate: 5.0 / 1e6, out_rate: 25.0 / 1e6, ..ReflectConfig::default() };
        assert!(
            estimate_cost(&pricey, "sys", "user") > estimate_cost(&cheap, "sys", "user"),
            "higher rates ⇒ higher estimate"
        );
        // A free local model (0 rates) must estimate AND charge 0.
        let free = ReflectConfig { in_rate: 0.0, out_rate: 0.0, ..ReflectConfig::default() };
        assert_eq!(estimate_cost(&free, "sys", "user"), 0.0);
        assert_eq!(actual_cost(&free, 1000, 1000), 0.0);
    }

    #[test]
    fn build_client_picks_provider() {
        // Smoke: anthropic vs openai-compat construct without panicking; the concrete
        // type is opaque, so we just assert both branches run.
        let _a = build_client(&ReflectConfig::default(), Some("k".into()));
        let oc = ReflectConfig { provider: "openai".into(), ..ReflectConfig::default() };
        let _o = build_client(&oc, Some("k".into()));
    }
}
