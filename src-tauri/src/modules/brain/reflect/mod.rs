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
pub(crate) fn build_client(cfg: &ReflectConfig, key: Option<String>) -> Box<dyn ReflectClient + Send> {
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
#[derive(Clone, Debug)]
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

    // The one network call, then the shared reconcile → validate → enqueue tail.
    let result = client.complete(&cfg.model, &system, &user, cfg.max_output_tokens);
    finish_response(index, cfg, project_id, rid, est, result, now_ms, true)
}

/// The reconcile → validate → enqueue tail shared by the synchronous
/// [reflect_with_client] and the offloaded [reflect_finish]. Charges the ledger
/// from the provider result (or the failure classification), validates the JSON,
/// and enqueues proposals. ALL of this runs on the single writer thread — the
/// offload only moves the network call ([ReflectPending::call]) off-thread, never
/// any index access.
///
/// `enqueue=false` is the reconcile-only path for a project UNREGISTERED mid-flight
/// (RemoveProject landed while the call was running): the reservation is still
/// reconciled so the ceiling is never stranded, but proposals are NOT written — they
/// would be orphan rows for a project whose state was just pruned. [LIB-DESIGN-01 miss2]
#[allow(clippy::too_many_arguments)]
fn finish_response(
    index: &SqliteIndex,
    cfg: &ReflectConfig,
    project_id: &str,
    reservation_id: i64,
    estimate: f64,
    result: Result<ReflectResponse, String>,
    now_ms: i64,
    enqueue: bool,
) -> ReflectOutcome {
    // On an error the provider DEMONSTRABLY did not bill (a 4xx rejection), release
    // the reservation at $0; on anything ambiguous (network cut, timeout, 5xx) charge
    // the estimate — a partial/billed call may have happened (charge on uncertainty,
    // §4.1.11).
    let resp = match result {
        Ok(r) => r,
        Err(e) => {
            let charge = charge_for_failed_call(estimate, &e);
            reconcile_or_log(index, reservation_id, charge, now_ms);
            return ReflectOutcome { proposals: Vec::new(), spent_usd: charge, reason: ReflectReason::CallFailed(e) };
        }
    };

    // Charge the ACTUAL reported cost — but a 2xx with missing/garbled usage
    // deserializes to 0/0 tokens, and Anthropic still bills the input. Floor an
    // implausible 0/0 to the conservative estimate so a success never under-charges.
    let actual = actual_cost(cfg, resp.input_tokens, resp.output_tokens);
    let charge = if resp.input_tokens == 0 && resp.output_tokens == 0 { estimate } else { actual };
    reconcile_or_log(index, reservation_id, charge, now_ms);

    let items = match schema::parse_and_validate(&resp.json_text) {
        Ok(v) => v,
        Err(_) => return ReflectOutcome { proposals: Vec::new(), spent_usd: charge, reason: ReflectReason::InvalidOutput },
    };

    // Map → enqueue into the SAME P1 queue (dedup by signature; skip rejected).
    // parse_and_validate already hard-rejects > MAX_PROPOSALS; the take is a
    // defensive belt against a future config raising the cap above the parse limit.
    // Skipped entirely on the reconcile-only path (project unregistered mid-flight).
    let mut enqueued = Vec::new();
    if enqueue {
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

/// True when a call error demonstrably means the provider billed nothing: both
/// real clients surface a non-2xx as `"<provider> http <status>"` (status only,
/// never the body), and a 4xx is a request REJECTED before inference
/// (validation / auth / rate-limit — providers do not bill these). Anything else
/// (serialization, network cut, timeout, 5xx) stays ambiguous.
pub(crate) fn provider_rejected_unbilled(err: &str) -> bool {
    err.rfind(" http ")
        .and_then(|i| err[i + 6..].trim().parse::<u16>().ok())
        .is_some_and(|c| (400..500).contains(&c))
}

/// Charge for a FAILED call: $0 for a demonstrably-unbilled 4xx rejection,
/// otherwise the conservative estimate (charge-on-uncertainty — the request may
/// have been processed and billed even though no response arrived). Shared by the
/// reflect and both curation call sites. The reservation is still reconciled (at
/// the returned charge) so it never strands against the ceiling.
pub(crate) fn charge_for_failed_call(est: f64, err: &str) -> f64 {
    if provider_rejected_unbilled(err) {
        0.0
    } else {
        est
    }
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
    // Back up the new durable spend total to the canonical-tail journal — reconcile
    // runs through `conn()` outside the SqliteIndex spend methods, so this is where a
    // header-destroyed store learns it already spent (and can't re-spend the ceiling).
    index.journal_budget();
}

/// Manual-trigger reflect (the real path): always builds + sends if the budget/key
/// gates pass. Thin wrapper over [reflect_auto] with no prior digest, so a manual
/// click never short-circuits on "unchanged". Returns the digest hash too so the
/// caller can feed the autonomous delta gate — a manual round must not leave the
/// next auto round re-paying for the byte-identical digest (ADR-010 cluster 5).
pub fn reflect_once(
    app: &AppHandle,
    index: &SqliteIndex,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
) -> (ReflectOutcome, Option<String>) {
    reflect_auto(app, index, project_id, now_date, now_ms, None)
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

// ============================================================================
// Offloaded reflect (LIB-DESIGN-01: worker stall during the provider call).
//
// The autonomous round is split across the network boundary so the brain worker
// thread never blocks on `client.complete()`:
//   1. [reflect_prepare] runs on the worker — digest read + delta gate + durable
//      budget reserve (all index access stays on the single writer thread).
//   2. [ReflectPending::call] runs on a helper thread — the ONLY off-thread work;
//      it touches no index, only the network.
//   3. [reflect_finish] runs back on the worker — reconcile + validate + enqueue.
// The single-writer invariant is fully preserved (every index write is still on
// the worker); only the network wait leaves the Fs-serving thread free.
// ============================================================================

/// Metadata captured at prepare time and carried — alongside the raw provider
/// result — back to the worker to complete the round. Clone + Debug so it can ride
/// a `BrainEvent`. Its inner budget/config fields are private; the worker only
/// reads [ReflectFinish::digest_hash] and hands the whole value to [reflect_finish].
#[derive(Clone, Debug)]
pub struct ReflectFinish {
    cfg: ReflectConfig,
    reservation_id: i64,
    estimate: f64,
    /// The digest hash this round reflected on (feeds the autonomous delta gate).
    pub digest_hash: String,
}

/// A reflect round whose durable budget reservation is placed but whose provider
/// call has NOT run yet. Move it to a helper thread and call [ReflectPending::call],
/// then hand the returned parts to [reflect_finish] on the worker thread.
pub struct ReflectPending {
    client: Box<dyn ReflectClient + Send>,
    model: String,
    system: String,
    user: String,
    max_tokens: u32,
    finish: ReflectFinish,
}

impl ReflectPending {
    /// Run ONLY the provider network call (off the worker thread). Consumes self,
    /// returning the raw result plus the finish metadata for the worker. Touches no
    /// index — safe to run on any thread. A PANIC inside the client is folded into
    /// an Err result so the helper thread always produces a `LibrarianDone` — a
    /// vanished reply would wedge the project `in_flight` until restart and strand
    /// the reservation until the boot sweep. The Err path charges on uncertainty,
    /// same as any ambiguous network failure.
    pub fn call(self) -> (Result<ReflectResponse, String>, ReflectFinish) {
        let ReflectPending { client, model, system, user, max_tokens, finish } = self;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.complete(&model, &system, &user, max_tokens)
        }))
        .unwrap_or_else(|_| Err("provider client panicked".to_string()));
        (result, finish)
    }

    /// The digest hash of the round in flight (for bookkeeping / logging).
    pub fn digest_hash(&self) -> &str {
        &self.finish.digest_hash
    }
}

/// The outcome of [reflect_prepare] / [reflect_prepare_with_client].
pub enum ReflectDispatch {
    /// No provider call is needed — the outcome is already final (EmptyCorpus,
    /// delta-gate Unchanged, a $0 pre-flight skip, or a reserve failure). Carries
    /// the digest hash (None only for EmptyCorpus) for the caller's delta gate.
    Ready(ReflectOutcome, Option<String>),
    /// A provider call is required; run [ReflectPending::call] off-thread and then
    /// [reflect_finish] on the worker.
    Pending(ReflectPending),
}

/// The offload twin of [reflect_auto_with_client] (testable offline): build the
/// digest, apply the delta gate, and — when a call is warranted — estimate + place
/// the durable budget reservation, all WITHOUT the provider call. Returns
/// [ReflectDispatch::Pending] with the reservation already held when a call must
/// run; the gating is byte-for-byte the same as [reflect_auto_with_client].
pub fn reflect_prepare_with_client(
    index: &SqliteIndex,
    client: Box<dyn ReflectClient + Send>,
    cfg: &ReflectConfig,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
    prev_digest_hash: Option<&str>,
) -> ReflectDispatch {
    let Some(user) = build_digest(index, project_id, now_date) else {
        return ReflectDispatch::Ready(ReflectOutcome::noop(ReflectReason::EmptyCorpus), None);
    };
    let digest_hash = hash::hash_bytes(user.as_bytes());
    if prev_digest_hash == Some(digest_hash.as_str()) {
        return ReflectDispatch::Ready(ReflectOutcome::noop(ReflectReason::Unchanged), Some(digest_hash));
    }
    let system = schema::system_prompt();
    let est = estimate_cost(cfg, &system, &user);
    // Pre-flight reserve on the worker thread — the durable, atomic ceiling gate.
    let rid = match budget::check_and_reserve(index.conn(), &cfg.model, est, now_ms) {
        Ok(id) => id,
        Err(reason) => return ReflectDispatch::Ready(ReflectOutcome::noop(reason), Some(digest_hash)),
    };
    ReflectDispatch::Pending(ReflectPending {
        client,
        model: cfg.model.clone(),
        system,
        user,
        max_tokens: cfg.max_output_tokens,
        finish: ReflectFinish { cfg: cfg.clone(), reservation_id: rid, estimate: est, digest_hash },
    })
}

/// Autonomous offloaded reflect (the real worker path): resolve config/ceiling/key
/// exactly like [reflect_auto], then PREPARE (reserve) without making the call. The
/// worker runs the returned [ReflectPending] on a helper thread and completes it via
/// [reflect_finish], so incremental indexing is never blocked by the provider call.
pub fn reflect_prepare(
    app: &AppHandle,
    index: &SqliteIndex,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
    prev_digest_hash: Option<&str>,
) -> ReflectDispatch {
    let cfg = ReflectConfig::from_librarian(&librarian::config(index.conn()));
    let ceiling_usd = budget::ceiling(index.conn());
    let account = librarian::keyring_account_for(&cfg.provider);
    let key = if account.is_empty() {
        None
    } else {
        crate::modules::secrets::read_secret(app, KEYRING_SERVICE, account)
    };
    let key_present = key.is_some() || librarian::is_keyless(&cfg.provider);
    if let Some(reason) = pre_flight(ceiling_usd, key_present) {
        return ReflectDispatch::Ready(ReflectOutcome::noop(reason), None);
    }
    let client = build_client(&cfg, key);
    reflect_prepare_with_client(index, client, &cfg, project_id, now_date, now_ms, prev_digest_hash)
}

/// Complete an offloaded round on the worker thread from the provider result:
/// reconcile the reservation, validate, and enqueue proposals (the same tail as
/// [reflect_with_client]). Returns the outcome plus the digest hash the round
/// reflected on, so the caller can pin it for the delta gate.
pub fn reflect_finish(
    index: &SqliteIndex,
    project_id: &str,
    finish: ReflectFinish,
    result: Result<ReflectResponse, String>,
    now_ms: i64,
) -> (ReflectOutcome, Option<String>) {
    let ReflectFinish { cfg, reservation_id, estimate, digest_hash } = finish;
    let outcome = finish_response(index, &cfg, project_id, reservation_id, estimate, result, now_ms, true);
    (outcome, Some(digest_hash))
}

/// Complete an offloaded round for a project that was UNREGISTERED mid-flight — a
/// `BrainEvent::RemoveProject` landed (pruning the project's rows AND its delta-gate
/// pin) while this round's provider call was still running. Reconciles the budget
/// reservation from the result so the ceiling is never stranded, but does NOT enqueue
/// proposals (they would be orphan rows for a pruned project) and returns NO digest
/// hash — so the worker skips re-persisting the pin `remove_project` deliberately
/// deleted (which would otherwise resurrect it and wedge a re-added identical corpus
/// at Unchanged/$0 forever). [LIB-DESIGN-01 miss2 / LIB-SPEND-01 pin-delete invariant]
pub fn reflect_reconcile_only(
    index: &SqliteIndex,
    project_id: &str,
    finish: ReflectFinish,
    result: Result<ReflectResponse, String>,
    now_ms: i64,
) -> ReflectOutcome {
    let ReflectFinish { cfg, reservation_id, estimate, digest_hash: _ } = finish;
    finish_response(index, &cfg, project_id, reservation_id, estimate, result, now_ms, false)
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
    fn failed_call_charge_classifies_unbilled_4xx() {
        // Both real clients' 4xx shape → demonstrably unbilled → $0.
        assert_eq!(charge_for_failed_call(0.01, "anthropic http 400"), 0.0);
        assert_eq!(charge_for_failed_call(0.01, "openai-compat http 429"), 0.0);
        // Ambiguous (network / 5xx / parse) → charge-on-uncertainty (the estimate).
        for e in [
            "anthropic http 500",
            "connection reset",
            "error sending request for url (https://api.openai.com/v1)",
            "openai-compat resp parse: EOF",
        ] {
            assert_eq!(charge_for_failed_call(0.01, e), 0.01, "ambiguous must charge: {e}");
        }
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
