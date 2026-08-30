//! The Brain's internal event spine. Two ingest legs (agent lifecycle + file
//! changes) fold into one `BrainEvent` enum handled on the single worker thread
//! (CONCEPT §5.2). All ingest paths only *translate and send* — never mutate
//! state directly (the concurrency rule, EXECUTION_PLAN §2.6).

use std::path::PathBuf;

use crate::modules::brain::reflect::{ReflectFinish, ReflectResponse};
use crate::modules::brain::ProjectId;

/// Reply channel for a `ResolveProposal` / `RevertProposal`: the worker (the single
/// writer) sends the outcome back so the command can surface a SOFT failure — e.g.
/// a missing target note on approve, or a pre-ADR-018 row with no undo snapshot on
/// revert — to the caller as `Err(String)`, while the write itself stays on the
/// writer thread (single-writer discipline). `None` = fire and forget (the worker
/// just logs a failure).
pub type ResolveReply = std::sync::mpsc::Sender<Result<(), String>>;

/// Independent mirror of the pty `koden:agent-signal` JSON payload. Resolves
/// blocker **B2** without touching `pty/agent_detect.rs`: the brain `app.listen`s
/// the already-JSON event and deserializes into *its own* type, so it never
/// depends on pty's private `AgentSignal` (no wire change, no surface widening).
/// Shape must stay in sync with `AgentSignal` (`pty/agent_detect.rs:36-41`):
/// `{ id: u32, kind: "started"|"working"|"attention"|"finished"|"exited",
///    agent: Option<String> }`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AgentSignalPayload {
    pub id: u32,
    pub kind: String,
    #[serde(default)]
    pub agent: Option<String>,
}

/// Everything the worker loop reacts to.
#[derive(Clone, Debug)]
pub enum BrainEvent {
    /// From `app.listen("koden:agent-signal")`. `pty_id` is the pty session id.
    Agent {
        pty_id: u32,
        kind: String,
        agent: Option<String>,
    },
    /// From the recursive `notify` watcher (P1), already coalesced per project.
    Fs {
        project: ProjectId,
        changed: Vec<PathBuf>,
    },
    /// A submitted user prompt (ADR-020), forwarded by the frontend AgentBusBridge
    /// from the UserPromptSubmit hook bus line via `brain_record_turn`. The worker
    /// pre-filters (empty / slash-command-only), truncates, REDACTS at ingest, and
    /// resolves pty → cwd → project exactly like [`BrainEvent::Agent`] before the
    /// single writer stores it as a `turn` activity row. `session_id` is the
    /// payload's Claude session id (already allowlisted by the command): the
    /// Tier-2 resume capture, independent of whether the prompt text is kept.
    Turn {
        pty_id: u32,
        prompt: String,
        session_id: Option<String>,
    },
    /// Periodic self-tick: flush WAL, reconcile ledger (P4), retry degraded store.
    Tick,
    /// Webview/command-initiated reindex (e.g. a wizard "rescan", or `None` =
    /// reconcile every project).
    Rescan { project: Option<ProjectId> },
    /// Unregister a project + prune all its indexed state (writer-side). Does not
    /// touch user-authored files. `root` is the removed project's root, captured
    /// by the command BEFORE the registry entry vanished, so the worker can also
    /// delete the DERIVED gist hook artifact (ADR-019) — an unregistered project
    /// must not keep injecting a frozen gist into sessions.
    RemoveProject {
        project: ProjectId,
        root: Option<String>,
    },
    /// Run the memory doctor and queue proposals. `now_date` (ISO YYYY-MM-DD)
    /// enables the staleness check; `None` runs structural checks only.
    Doctor {
        project: Option<ProjectId>,
        now_date: Option<String>,
    },
    /// Resolve a proposal: `reject` persists its reject-signature + marks it
    /// rejected; otherwise APPLIES it (materializes the note change onto disk,
    /// snapshotting the inverse first, + marks it applied). `reply` (if set)
    /// receives the outcome so the command can surface a soft failure.
    ResolveProposal {
        project: ProjectId,
        signature: String,
        reject: bool,
        reply: Option<ResolveReply>,
    },
    /// Revert an APPLIED proposal (ADR-018 undo): restore the pre-apply file
    /// snapshot, flip the row to `reverted`, and persist its reject-signature so
    /// the Librarian does not re-propose (and re-auto-apply) the same change next
    /// round. Idempotent — reverting twice is a no-op. `reply` mirrors
    /// `ResolveProposal`.
    RevertProposal {
        project: ProjectId,
        signature: String,
        reply: Option<ResolveReply>,
    },
    /// Set the Librarian's curation mode (ADR-018): `autonomous` — the worker
    /// applies proposals itself (snapshot-undo recorded); `review` — proposals
    /// wait for a human in the inbox. Writer-side; journaled like SetLibrarian.
    SetCurationMode { mode: String },
    /// Toggle real-time memory injection (ADR-019): ON — the worker maintains a
    /// per-project gist hook artifact that every Claude Code turn picks up; OFF —
    /// the worker deletes the artifacts and stops regenerating (the hook then
    /// finds nothing — no stale memory). Writer-side; journaled like SetLibrarian.
    SetInjectGist { on: bool },
    /// Run a budgeted LLM reflect pass (P4) — the only token-spending path.
    /// Manual-trigger only; `project = None` reflects every registered project.
    /// `now_date` (ISO YYYY-MM-DD) feeds the doctor findings in the digest.
    Reflect {
        project: Option<ProjectId>,
        now_date: Option<String>,
    },
    /// Set the reflect spend ceiling (USD; 0.0 disables). Writer-side (P4).
    SetBudget { ceiling_usd: f64 },
    /// Set the Librarian's LLM selection (which provider/model the budgeted
    /// reflect+curate path uses). Rates are $/million-tokens (0 for free local
    /// models). The key is read at call time from the per-provider keyring account.
    /// Writer-side.
    SetLibrarian {
        provider: String,
        model: String,
        base_url: String,
        in_rate_mtok: f64,
        out_rate_mtok: f64,
    },
    /// Run stale-ADR / memory curation (V2 Flow G). Manual-trigger; `project = None`
    /// curates every registered project. `now_date` (ISO YYYY-MM-DD) drives the
    /// staleness signal. ACT-band proposals are $0; the escalate band shares the
    /// reflect budget ceiling.
    Curate {
        project: Option<ProjectId>,
        now_date: Option<String>,
    },
    /// A librarian round's provider call finished on a helper thread
    /// (LIB-DESIGN-01): the worker completes the round (reconcile + validate +
    /// enqueue) on the single writer thread. Offloading the call is what keeps the
    /// worker free to serve `Fs` deltas while the model is thinking — the network
    /// wait never stalls incremental indexing. `finish` carries the reservation +
    /// config captured at prepare time; `result` is the raw provider outcome.
    LibrarianDone {
        project: ProjectId,
        finish: ReflectFinish,
        result: Result<ReflectResponse, String>,
    },
}
