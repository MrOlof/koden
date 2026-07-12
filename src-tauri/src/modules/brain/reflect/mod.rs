//! P4 — budgeted LLM reflect: the ONLY token-spending path in Koden Brain.
//! Opt-in, default-$0, hard pre-flight budget gate, single-flight, fail-open.
//! The model PROPOSES memory cleanups into the P1 queue — reflect itself NEVER
//! writes user memory and NEVER spends without a durable reservation passing the
//! ceiling check first. Who then APPLIES the queue is ADR-018's curation mode:
//! the worker sweep in the default AUTONOMOUS mode (snapshot-undo recorded,
//! everything revertible), or a human approval in 'review' mode.
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
use crate::modules::brain::memory::proposal::{
    is_near_duplicate, proposal_dedup_set, reject_signature, ProposalAction, NEAR_DUPE_THRESHOLD,
};
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
/// How many recently-RESOLVED (applied|rejected) proposals feed the near-dupe gate
/// and the "already proposed" digest section, on top of ALL pending. Bounds both the
/// comparison cost and the prompt size; 50 covers many reflect rounds of history
/// (canonical writes are low-frequency) without unbounded growth.
const RESOLVED_DEDUP_LOOKBACK: usize = 50;
/// Cap on titles listed in the "already proposed" digest section, so a large inbox
/// can't blow up the user message (and the token estimate). Pending are listed first.
const ALREADY_PROPOSED_MAX: usize = 40;
/// ADR-020: newest activity rows appended to the SENT message (never the hash —
/// the append_already_proposed split) + the per-line turn-snippet char cap.
const ACTIVITY_SEND_MAX: usize = 12;
const ACTIVITY_SNIPPET_CHARS: usize = 120;
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
    let Some(corpus) = build_digest(index, project_id, now_date) else {
        return ReflectOutcome::noop(ReflectReason::EmptyCorpus);
    };
    // What we SEND additionally carries the "already proposed" advisory + the
    // recent-activity context (ADR-020). NEITHER is part of the delta-gate hash
    // (that is `build_digest` alone) — see append_already_proposed /
    // append_recent_activity — so enqueuing proposals can't self-re-fire a round
    // and turn ingest never buys a paid round by itself.
    let user = append_recent_activity(index, project_id, append_already_proposed(index, project_id, corpus));
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

    // Map → enqueue into the SAME P1 queue. THREE dedup layers, cheapest first:
    //  1. reject-signature — a previously DECLINED item (exact djb2 on title) never
    //     resurfaces (unchanged).
    //  2. semantic near-dupe gate — the title-signature PK missed re-wordings, so a
    //     token-set Jaccard over title+detail suppresses paraphrases of anything
    //     already pending, recently resolved, OR enqueued earlier in THIS response
    //     (3 re-wordings in one reply collapse to 1). Suppressions are counted, not
    //     enqueued — no journal interaction, no signature written.
    //  3. insert_proposal's exact-signature PK — the final belt.
    // parse_and_validate already hard-rejects > MAX_PROPOSALS; the take is a defensive
    // belt. Skipped entirely on the reconcile-only path (project unregistered).
    let mut enqueued = Vec::new();
    let mut suppressed = 0usize;
    if enqueue {
        let mut seen: Vec<std::collections::HashSet<String>> = index
            .proposal_dedup_texts(project_id, RESOLVED_DEDUP_LOOKBACK)
            .iter()
            .map(|(t, d)| proposal_dedup_set(t, proposal::undecorated_detail(d)))
            .collect();
        // The set of real note ids for this project, for the D2 actionability gate below.
        let note_ids: std::collections::HashSet<String> =
            index.existing_note_ids(project_id).unwrap_or_default().into_iter().collect();
        for item in items.iter().take(cfg.max_proposals) {
            let p = proposal::to_proposal(project_id, item);
            // D2 actionability gate: Archive/Update (reflect's stale/conflict) apply
            // against a target note. A proposal whose target_id is missing or names no
            // known note would strand as a reject-only card (Approve → "no target note")
            // — drop it instead. Create/Supersede carry no target and are unaffected.
            if matches!(p.action, ProposalAction::Archive | ProposalAction::Update)
                && p.target_id.as_deref().is_none_or(|t| !note_ids.contains(t))
            {
                log::info!(
                    "brain: reflect dropped an unactionable {} proposal '{}' (target {:?} is not a known note; project {project_id})",
                    p.action.as_str(),
                    p.title,
                    p.target_id
                );
                continue;
            }
            let rej = reject_signature(p.action, p.target_id.as_deref(), &p.title);
            if index.is_rejected(project_id, &rej).unwrap_or(false) {
                continue; // declined before — don't resurrect it
            }
            // Compare on the model's RAW rationale (undecorated) so the "scope ·
            // confidence" boilerplate shared by every reflect proposal never inflates
            // similarity between two distinct facts.
            let cand = proposal_dedup_set(&p.title, proposal::undecorated_detail(&p.detail));
            if is_near_duplicate(&cand, &seen, NEAR_DUPE_THRESHOLD) {
                log::debug!(
                    "brain: reflect suppressed a near-duplicate proposal '{}' (project {project_id})",
                    p.title
                );
                suppressed += 1;
                continue;
            }
            if index.insert_proposal(project_id, &p, now_ms).unwrap_or(false) {
                seen.push(cand);
                enqueued.push(p);
            }
        }
        if suppressed > 0 {
            // Redundancy signal for the live gauntlet — surfaced via the log (NOT a
            // ReflectOutcome field: an outcome-shape change would break the sim
            // gauntlet's own ReflectOutcome literals).
            log::info!(
                "brain: reflect enqueued {} proposal(s), suppressed {suppressed} near-duplicate(s) (project {project_id})",
                enqueued.len()
            );
        }
    }
    ReflectOutcome { proposals: enqueued, spent_usd: charge, reason: ReflectReason::Ok }
}

/// The STABLE corpus digest (memory notes + structural doctor findings), redacted,
/// or None for an empty corpus. This — and ONLY this — is what the autonomous delta
/// gate hashes: it is a pure function of the note/finding corpus, so it does NOT move
/// when proposals are enqueued. The message actually SENT to the model appends the
/// volatile "already proposed" advisory on top (see [append_already_proposed]); if
/// that advisory were folded in here, every enqueue would change the digest and
/// self-re-fire a paid round in a loop — the invariant the split protects.
///
/// The belt-and-suspenders secret gate (§7.1) redacts the ENTIRE assembled corpus
/// here, immediately before it could reach the cloud.
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

/// Append the bounded "## Already proposed (do not re-propose)" advisory to the corpus
/// for the SEND ONLY (never the delta-gate hash — see [build_digest]). Lists the
/// titles of pending + recently-resolved proposals so the model doesn't restate what's
/// already in the inbox or was just decided. The advisory is redacted through the same
/// [secrets::redact] path as the corpus (defense in depth on titles). Returns `corpus`
/// unchanged when there is nothing proposed yet.
fn append_already_proposed(index: &SqliteIndex, project_id: &str, corpus: String) -> String {
    let texts = index.proposal_dedup_texts(project_id, RESOLVED_DEDUP_LOOKBACK);
    if texts.is_empty() {
        return corpus;
    }
    let lines: Vec<String> = texts
        .iter()
        .take(ALREADY_PROPOSED_MAX)
        .map(|(title, _)| format!("- {}", title.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect();
    let section = format!("## Already proposed (do not re-propose)\n\n{}", lines.join("\n"));
    let redacted = crate::modules::brain::secrets::redact(&section).0;
    format!("{corpus}\n\n{redacted}")
}

/// ADR-020: append the recent session-activity trail to the SENT message ONLY —
/// the identical split as [append_already_proposed] (never folded into
/// [build_digest], whose hash is the delta gate; a hashed trail would make every
/// stored turn buy a paid round). Rows were redacted at ingest; the assembled
/// section passes [secrets::redact] again (defense in depth, same as the
/// advisory). Returns `corpus` unchanged when the trail is empty.
fn append_recent_activity(index: &SqliteIndex, project_id: &str, corpus: String) -> String {
    let rows = index.recent_activity(project_id, ACTIVITY_SEND_MAX).unwrap_or_default();
    if rows.is_empty() {
        return corpus;
    }
    let snip = |s: &str| -> String {
        let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if one_line.chars().count() <= ACTIVITY_SNIPPET_CHARS {
            one_line
        } else {
            let cut: String = one_line.chars().take(ACTIVITY_SNIPPET_CHARS).collect();
            format!("{}…", cut.trim_end())
        }
    };
    let lines: Vec<String> = rows
        .iter()
        .map(|r| match r.kind.as_str() {
            "turn" => format!("- turn: {}", snip(&r.payload_redacted)),
            "files" => {
                let files: Vec<String> =
                    serde_json::from_str(&r.payload_redacted).unwrap_or_default();
                let extra = files.len().saturating_sub(6);
                let head = files.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
                if extra > 0 {
                    format!("- files touched: {head} (+{extra})")
                } else {
                    format!("- files touched: {head}")
                }
            }
            "start" => format!("- session started: {}", snip(&r.payload_redacted)),
            "end" => format!("- session ended: {}", snip(&r.payload_redacted)),
            other => format!("- {other}"),
        })
        .collect();
    let section = format!(
        "## Recent session activity (context only — not memory, do not re-propose)\n\n{}",
        lines.join("\n")
    );
    let redacted = crate::modules::brain::secrets::redact(&section).0;
    format!("{corpus}\n\n{redacted}")
}

/// Pin the CURRENT corpus digest as a project's delta-gate pin — the ADR-018
/// self-feeding-loop guard. `build_digest` is a pure function of the note corpus,
/// so an ENQUEUE never moves it (the invariant it documents) — but an APPLY does:
/// the autonomous worker writing `.koden-memory` files (an auto-apply batch, a
/// revert) changes the very corpus the next round would hash, and without re-pinning
/// every round would chain a paid call on the Librarian's own writes until
/// quiescent. Called by the worker right AFTER such brain-originated writes; user
/// edits still unpin naturally (different bytes ⇒ different hash). Returns the
/// pinned hash (None for an empty corpus) so the caller can fold it into the
/// in-memory `LibrarianAuto.digest_hash`; the durable write is best-effort (a
/// failure only risks one re-paid round, logged).
pub fn pin_corpus_digest(
    index: &SqliteIndex,
    project_id: &str,
    now_date: Option<&str>,
    now_ms: i64,
) -> Option<String> {
    let corpus = build_digest(index, project_id, now_date)?;
    let h = hash::hash_bytes(corpus.as_bytes());
    if let Err(e) = index.set_librarian_pin(project_id, &h, now_ms) {
        log::warn!("brain: pin post-apply digest for '{project_id}' failed ({e})");
    }
    Some(h)
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
    // Gate on the STABLE corpus hash only; reflect_with_client appends the volatile
    // "already proposed" advisory for the actual send (so enqueues can't self-re-fire).
    let Some(corpus) = build_digest(index, project_id, now_date) else {
        return (ReflectOutcome::noop(ReflectReason::EmptyCorpus), None);
    };
    let digest_hash = hash::hash_bytes(corpus.as_bytes());
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
    // Hash the STABLE corpus only (the delta gate), then append the volatile "already
    // proposed" advisory + recent-activity context (ADR-020) for the SEND — same split
    // as [reflect_with_client], so an offloaded round that enqueues proposals doesn't
    // self-re-fire either.
    let Some(corpus) = build_digest(index, project_id, now_date) else {
        return ReflectDispatch::Ready(ReflectOutcome::noop(ReflectReason::EmptyCorpus), None);
    };
    let digest_hash = hash::hash_bytes(corpus.as_bytes());
    if prev_digest_hash == Some(digest_hash.as_str()) {
        return ReflectDispatch::Ready(ReflectOutcome::noop(ReflectReason::Unchanged), Some(digest_hash));
    }
    let system = schema::system_prompt();
    let user = append_recent_activity(index, project_id, append_already_proposed(index, project_id, corpus));
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

    // ---- Proposal-stream quality: near-dupe gate + pending-aware digest ----------

    use crate::modules::brain::memory::scan_project_memory;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

    /// A $0 fake provider that records call count + the LAST user message it received
    /// and replays a fixed proposals JSON. Never touches a network.
    struct CapturingFake {
        calls: Cell<u32>,
        last_user: RefCell<String>,
        json: String,
    }
    impl ReflectClient for CapturingFake {
        fn complete(&self, _m: &str, _s: &str, user: &str, _t: u32) -> Result<ReflectResponse, String> {
            self.calls.set(self.calls.get() + 1);
            *self.last_user.borrow_mut() = user.to_string();
            Ok(ReflectResponse { json_text: self.json.clone(), input_tokens: 10, output_tokens: 5 })
        }
    }
    fn fake(json: String) -> CapturingFake {
        CapturingFake { calls: Cell::new(0), last_user: RefCell::new(String::new()), json }
    }

    /// Build a `{"proposals":[…]}` reply from `(title, detail)` pairs.
    fn proposals_json(items: &[(&str, &str)]) -> String {
        let body = items
            .iter()
            .map(|(t, d)| {
                format!(
                    r#"{{"kind":"insight","title":{},"detail":{},"scope":"project","confidence":"high"}}"#,
                    serde_json::to_string(t).unwrap(),
                    serde_json::to_string(d).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"proposals":[{body}]}}"#)
    }

    /// A temp store with one memory note (so the corpus is non-empty) and the budget
    /// armed. Returns (scratch_dir, index, default cfg).
    fn temp_index_with_note(label: &str) -> (PathBuf, SqliteIndex, ReflectConfig) {
        let base = std::env::temp_dir().join(format!("koden-reflect-dedup-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("proj");
        let note = root.join(".koden-memory").join("n1.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::write(
            &note,
            "---\nid: n1\ntype: insight\ntitle: A seed note\nstatus: active\n---\n# A seed note\n\nBody.\n",
        )
        .unwrap();
        let db = base.join("store").join("index.sqlite");
        let idx = SqliteIndex::open_with_recovery(&db).expect("open store");
        scan_project_memory(&idx, "p", &root);
        idx.set_budget_ceiling(1.0, 1).expect("arm budget");
        (base, idx, ReflectConfig::default())
    }

    // The paraphrase pair the live gauntlet's title-signature dedup let through, plus a
    // genuinely distinct fact that must survive.
    const PARAPHRASE_A: (&str, &str) = (
        "Stripe webhook verifies signature before parsing",
        "The Stripe webhook handler verifies the request signature before parsing the payload body.",
    );
    const PARAPHRASE_B: (&str, &str) = (
        "Webhook signature check precedes body parsing",
        "The webhook signature check precedes parsing of the request payload body in the handler.",
    );
    const DISTINCT_C: (&str, &str) = (
        "Database migrations run automatically on startup",
        "Prisma schema migrations are applied during application boot via the migrate deploy command.",
    );

    #[test]
    fn near_dupe_proposals_are_suppressed_at_enqueue() {
        let (base, idx, cfg) = temp_index_with_note("suppress");
        let fk = fake(proposals_json(&[PARAPHRASE_A, PARAPHRASE_B, DISTINCT_C]));
        let out = reflect_with_client(&idx, &fk, &cfg, "p", Some("2026-07-10"), 1000);
        assert!(matches!(out.reason, ReflectReason::Ok), "{:?}", out.reason);
        assert_eq!(out.proposals.len(), 2, "one paraphrase + the distinct fact enqueued (1 suppressed)");
        // Persisted exactly the two survivors.
        let pending = idx.proposal_dedup_texts("p", 50);
        assert_eq!(pending.len(), 2, "only the two survivors are pending: {pending:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cross_round_paraphrase_is_suppressed_against_pending() {
        // The gauntlet's "accepted fact resurfaced 3 rounds later under a fresh title":
        // a paraphrase of an ALREADY-PENDING proposal must be suppressed on a later round.
        let (base, idx, cfg) = temp_index_with_note("crossround");
        let out1 = reflect_with_client(&idx, &fake(proposals_json(&[PARAPHRASE_A])), &cfg, "p", Some("2026-07-10"), 1000);
        assert_eq!(out1.proposals.len(), 1, "round 1 enqueues the fact");
        // A later round proposes the same fact reworded → caught against the pending row.
        let out2 = reflect_with_client(&idx, &fake(proposals_json(&[PARAPHRASE_B])), &cfg, "p", Some("2026-07-10"), 2000);
        assert_eq!(out2.proposals.len(), 0, "the reworded resurfacing is suppressed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn enqueue_does_not_change_the_delta_gate_hash() {
        // The load-bearing invariant of the pending-aware digest split: enqueuing
        // proposals must NOT move the delta-gate hash, or the next round self-re-fires
        // a paid call forever.
        let (base, idx, cfg) = temp_index_with_note("noselffire");
        let fk = fake(proposals_json(&[("A caching insight", "Cache entries expire after ten minutes to bound staleness.")]));
        let (out1, h1) = reflect_auto_with_client(&idx, &fk, &cfg, "p", Some("2026-07-10"), 1000, None);
        assert!(matches!(out1.reason, ReflectReason::Ok), "{:?}", out1.reason);
        assert_eq!(out1.proposals.len(), 1, "round 1 enqueues");
        assert_eq!(fk.calls.get(), 1);
        let h1 = h1.expect("Ok pins a hash");
        // Same corpus, now with a pending proposal: must short-circuit to Unchanged/$0.
        let (out2, h2) = reflect_auto_with_client(&idx, &fk, &cfg, "p", Some("2026-07-10"), 2000, Some(&h1));
        assert!(matches!(out2.reason, ReflectReason::Unchanged), "{:?}", out2.reason);
        assert_eq!(out2.spent_usd, 0.0, "Unchanged is $0");
        assert_eq!(fk.calls.get(), 1, "no second paid call after an enqueue");
        assert_eq!(h2.as_deref(), Some(h1.as_str()), "gate hash stable across the enqueue");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ADR-020 delta-gate purity (the invariant the send-only split protects):
    /// ingesting 50 turn rows must leave `build_digest` BYTE-IDENTICAL — an
    /// unchanged corpus short-circuits to Unchanged/$0 even under heavy session
    /// activity — while the SENT message carries the trail via the split.
    #[test]
    fn turn_ingest_never_moves_the_delta_gate_hash_but_rides_the_send() {
        let (base, idx, cfg) = temp_index_with_note("activity_split");
        let before = build_digest(&idx, "p", Some("2026-07-12")).expect("corpus");

        // Round 1 establishes the pin.
        let fk = fake(proposals_json(&[]));
        let (out1, h1) = reflect_auto_with_client(&idx, &fk, &cfg, "p", Some("2026-07-12"), 1000, None);
        assert!(matches!(out1.reason, ReflectReason::Ok), "{:?}", out1.reason);
        let h1 = h1.expect("pinned");

        // 50 turns + a files row + session boundaries land in the trail.
        for i in 0..50i64 {
            idx.record_activity("p", Some(3), "turn", &format!("investigate flaky test {i}"), 2_000 + i)
                .unwrap();
        }
        idx.record_activity("p", None, "files", r#"["src/auth.rs"]"#, 2_100).unwrap();
        idx.record_activity("p", Some(3), "start", "claude", 2_200).unwrap();

        // The STABLE digest — and with it the delta-gate hash — is untouched.
        assert_eq!(
            build_digest(&idx, "p", Some("2026-07-12")).as_deref(),
            Some(before.as_str()),
            "activity ingest must never move the corpus digest"
        );
        let (out2, h2) = reflect_auto_with_client(&idx, &fk, &cfg, "p", Some("2026-07-12"), 3000, Some(&h1));
        assert!(matches!(out2.reason, ReflectReason::Unchanged), "{:?}", out2.reason);
        assert_eq!(out2.spent_usd, 0.0, "no paid round bought by turns");
        assert_eq!(h2.as_deref(), Some(h1.as_str()), "gate hash stable across 52 activity rows");
        assert_eq!(fk.calls.get(), 1, "the provider was NOT called again");

        // A round that DOES send (fresh pin) carries the trail in the message —
        // the send-only half of the split.
        let fk2 = fake(proposals_json(&[]));
        let _ = reflect_with_client(&idx, &fk2, &cfg, "p", Some("2026-07-12"), 4000);
        let sent = fk2.last_user.borrow().clone();
        assert!(
            sent.contains("## Recent session activity"),
            "trail section missing from the sent message: {sent}"
        );
        assert!(sent.contains("investigate flaky test 49"), "newest turn rides along: {sent}");
        assert!(sent.contains("src/auth.rs"), "files trail rides along: {sent}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sent_message_lists_already_proposed_titles() {
        let (base, idx, cfg) = temp_index_with_note("advisory");
        // Round 1 enqueues a distinctly-titled proposal.
        let title = "Retry queue drains oldest first";
        let out1 = reflect_with_client(
            &idx,
            &fake(proposals_json(&[(title, "The retry queue is FIFO so the oldest failed job runs first.")])),
            &cfg, "p", Some("2026-07-10"), 1000,
        );
        assert_eq!(out1.proposals.len(), 1);
        // Round 2 sends: the user message must now carry the advisory naming round 1's
        // title (so the model is told not to restate it).
        let fk2 = fake(proposals_json(&[]));
        let _ = reflect_with_client(&idx, &fk2, &cfg, "p", Some("2026-07-10"), 2000);
        let sent = fk2.last_user.borrow().clone();
        assert!(sent.contains("## Already proposed (do not re-propose)"), "advisory present: {sent}");
        assert!(sent.contains(title), "advisory names the pending title: {sent}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn unactionable_targets_dropped_and_valid_target_enqueues_then_applies() {
        // The D2 seam fix, end to end. The seed note's id is `n1`.
        let (base, idx, cfg) = temp_index_with_note("target_validation");
        let root = base.join("proj");
        // stale w/ valid target n1 (actionable), stale w/ unknown target, conflict w/
        // NO target, and a plain insight (create, no target needed).
        let json = r#"{"proposals":[
          {"kind":"stale","title":"Archive n1","detail":"the seed note is stale now","scope":"project","confidence":"high","target":"n1"},
          {"kind":"stale","title":"Archive ghost","detail":"a note that does not exist","scope":"project","confidence":"high","target":"ghost"},
          {"kind":"conflict","title":"Conflict without a target","detail":"names no note at all","scope":"project","confidence":"high"},
          {"kind":"insight","title":"A brand new insight","detail":"Something genuinely worth keeping around later.","scope":"project","confidence":"high"}
        ]}"#;
        let out = reflect_with_client(&idx, &fake(json.to_string()), &cfg, "p", Some("2026-07-10"), 1000);
        assert!(matches!(out.reason, ReflectReason::Ok), "{:?}", out.reason);
        let titles: Vec<&str> = out.proposals.iter().map(|p| p.title.as_str()).collect();
        assert!(titles.contains(&"Archive n1"), "valid-target stale enqueues: {titles:?}");
        assert!(titles.contains(&"A brand new insight"), "insight (create) enqueues: {titles:?}");
        assert!(!titles.contains(&"Archive ghost"), "unknown-target archive dropped");
        assert!(!titles.contains(&"Conflict without a target"), "target-less update dropped");
        assert_eq!(out.proposals.len(), 2, "only the two actionable proposals: {titles:?}");

        // Direction 2: the enqueued valid-target stale is now actually APPLYABLE (D2) —
        // approving it materializes the archive against the real note.
        let stale = out.proposals.iter().find(|p| p.title == "Archive n1").unwrap();
        assert_eq!(stale.target_id.as_deref(), Some("n1"), "target carried onto the proposal");
        idx.apply_proposal("p", &root, &stale.signature, "2026-07-10", 5_000, false).unwrap();
        let raw = std::fs::read_to_string(root.join(".koden-memory").join("n1.md")).unwrap();
        assert!(raw.contains("status: archived"), "approve archived the target note: {raw}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
