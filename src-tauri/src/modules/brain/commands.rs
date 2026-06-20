//! `#[tauri::command]` surface — the only thing the UI touches (CONCEPT §9).
//! Commands are READERS: they open their own read-only SQLite connection (WAL →
//! wait-free vs. the worker's writes) and never block the worker. Fail-open: a
//! warming/degraded brain returns partial/empty results, not errors.

use tauri::State;

use crate::modules::brain::events::BrainEvent;
use crate::modules::brain::memory::proposal::MemoryProposal;
use crate::modules::brain::memory::NoteSummary;
use crate::modules::brain::registry::Project;
use crate::modules::brain::store;
use crate::modules::brain::{BrainState, BrainStatus, Hit};

#[derive(serde::Serialize)]
pub struct ProjectStatus {
    pub project: Project,
    pub files: i64,
}

#[derive(serde::Serialize)]
pub struct BrainStatusReport {
    pub status: BrainStatus,
    pub projects: Vec<ProjectStatus>,
}

/// The registered project list.
#[tauri::command]
pub fn brain_list_projects(state: State<BrainState>) -> Vec<Project> {
    state.registry.projects()
}

/// Overall status + per-project indexed file counts.
#[tauri::command]
pub fn brain_index_status(state: State<BrainState>) -> BrainStatusReport {
    let status = state
        .status
        .read()
        .map(|s| s.clone())
        .unwrap_or(BrainStatus::Degraded { reason: "status lock poisoned".into() });
    let db = state.db_path.read().ok().and_then(|p| p.clone());
    let projects = state
        .registry
        .projects()
        .into_iter()
        .map(|project| {
            let files = match &db {
                Some(path) => store::file_count_readonly(path, &project.id).unwrap_or(0),
                None => 0,
            };
            ProjectStatus { project, files }
        })
        .collect();
    BrainStatusReport { status, projects }
}

/// Lexical (BM25 + weighted RRF) search across code (and, from P1, notes).
/// `project = None` searches every registered project.
#[tauri::command]
pub fn brain_search(
    state: State<BrainState>,
    project: Option<String>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<Hit>, String> {
    // Fail-open: not-ready brain → empty, not an error.
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(20).clamp(1, 200);
    match store::search_readonly(&db, project.as_deref(), &query, limit) {
        Ok(hits) => Ok(hits),
        // A missing/locked DB during warmup is not a user error.
        Err(e) => {
            log::debug!("brain_search soft error: {e}");
            Ok(Vec::new())
        }
    }
}

/// Structured memory notes (review inbox / cards). `project = None` = all.
#[tauri::command]
pub fn brain_notes(state: State<BrainState>, project: Option<String>) -> Vec<NoteSummary> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Vec::new();
    };
    store::list_notes_readonly(&db, project.as_deref()).unwrap_or_default()
}

/// Trigger a full reconcile (add/change/delete) of all registered projects, or a
/// single project. Enqueues onto the worker — non-blocking.
#[tauri::command]
pub fn brain_rescan(state: State<BrainState>, project: Option<String>) -> Result<(), String> {
    enqueue(&state, BrainEvent::Rescan { project })
}

/// Pending memory proposals (the review inbox). `project = None` = all.
#[tauri::command]
pub fn brain_proposals(state: State<BrainState>, project: Option<String>) -> Vec<MemoryProposal> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Vec::new();
    };
    store::list_proposals_readonly(&db, project.as_deref()).unwrap_or_default()
}

/// Run the memory doctor (queues proposals on the worker). `now_date` (ISO
/// YYYY-MM-DD) enables the staleness check; omit it for structural checks only.
#[tauri::command]
pub fn brain_doctor(
    state: State<BrainState>,
    project: Option<String>,
    now_date: Option<String>,
) -> Result<(), String> {
    enqueue(&state, BrainEvent::Doctor { project, now_date })
}

/// Resolve a proposal: `reject = true` declines it (persists a reject-signature
/// so it can't reappear); otherwise marks it applied. The Librarian never edits
/// user files itself — applying the change is the human's (or a tasked agent's) job.
#[tauri::command]
pub fn brain_resolve_proposal(
    state: State<BrainState>,
    project: String,
    signature: String,
    reject: bool,
) -> Result<(), String> {
    enqueue(&state, BrainEvent::ResolveProposal { project, signature, reject })
}

/// Enqueue a worker event via the registered sender (single-writer discipline).
fn enqueue(state: &State<BrainState>, ev: BrainEvent) -> Result<(), String> {
    let guard = state.tx.lock().map_err(|_| "brain tx poisoned".to_string())?;
    match guard.as_ref() {
        Some(tx) => tx
            .send(ev)
            .map_err(|e| format!("brain worker unavailable: {e}")),
        None => Err("brain not started yet".to_string()),
    }
}
