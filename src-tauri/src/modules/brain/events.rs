//! The Brain's internal event spine. Two ingest legs (agent lifecycle + file
//! changes) fold into one `BrainEvent` enum handled on the single worker thread
//! (CONCEPT §5.2). All ingest paths only *translate and send* — never mutate
//! state directly (the concurrency rule, EXECUTION_PLAN §2.6).

use std::path::PathBuf;

use crate::modules::brain::ProjectId;

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
    /// Periodic self-tick: flush WAL, reconcile ledger (P4), retry degraded store.
    Tick,
    /// Webview/command-initiated reindex (e.g. a wizard "rescan", or `None` =
    /// reconcile every project).
    Rescan { project: Option<ProjectId> },
}
