//! Koden Brain — the in-process "Librarian". One Rust module tree + one
//! GUI-resident worker thread that keeps a per-project index (lexical now; AST,
//! memory, semantic in later phases) fresh and serves it to the terminal's
//! agents. Architecture of record: ADR-006. Concept: `koden-brain-CONCEPT.md`.
//!
//! P0 (this phase): warm lexical brain — store + worker + registry + FTS5 BM25 +
//! weighted RRF + ignore-walk population + the secrets gate + the
//! `brain_search`/`brain_index_status`/`brain_list_projects` commands.

pub mod commands;
pub mod events;
pub mod freshness;
pub mod memory;
pub mod rank;
pub mod registry;
pub mod secrets;
pub mod store;
pub mod tokenize;
pub mod worker;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, RwLock};

use events::BrainEvent;
use registry::KodenBrainRegistry;

/// Stable per-project id (derived from the canonical root; see `registry`).
pub type ProjectId = String;

/// A search result.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Hit {
    pub project: ProjectId,
    pub path: String,
    /// Fused RRF score (higher = better).
    pub score: f64,
}

/// Index readiness, surfaced via `brain_index_status`.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum BrainStatus {
    Warming { pct: u8 },
    Ready,
    Degraded { reason: String },
}

/// Runtime config (P3+: injection on/off, reflect budget, feature flags).
#[derive(Clone, Debug)]
pub struct BrainConfig {
    pub enabled: bool,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Live per-pane session context: pty leaf → resolved project + agent. Populated
/// by agent-signal events; consumed by P3 gist synthesis.
#[derive(Clone, Debug, Default)]
pub struct LiveSession {
    pub project: Option<ProjectId>,
    pub agent: Option<String>,
}

/// Managed Tauri state. `default()` is cheap + infallible (only allocates locks),
/// so `.manage()` cannot block first paint — all I/O happens on the worker thread.
pub struct BrainState {
    pub status: RwLock<BrainStatus>,
    pub registry: KodenBrainRegistry,
    pub sessions: RwLock<HashMap<u32, LiveSession>>,
    pub config: RwLock<BrainConfig>,
    /// Set by the worker once started; commands enqueue `Rescan` etc. here.
    pub tx: Mutex<Option<Sender<BrainEvent>>>,
    /// Set by the worker after opening the store; read-only command connections
    /// open against this path.
    pub db_path: RwLock<Option<PathBuf>>,
}

impl Default for BrainState {
    fn default() -> Self {
        Self {
            status: RwLock::new(BrainStatus::Warming { pct: 0 }),
            registry: KodenBrainRegistry::default(),
            sessions: RwLock::new(HashMap::new()),
            config: RwLock::new(BrainConfig::default()),
            tx: Mutex::new(None),
            db_path: RwLock::new(None),
        }
    }
}
