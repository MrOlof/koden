//! `#[tauri::command]` surface — the only thing the UI touches (CONCEPT §9).
//! Commands are READERS: they open their own read-only SQLite connection (WAL →
//! wait-free vs. the worker's writes) and never block the worker. Fail-open: a
//! warming/degraded brain returns partial/empty results, not errors.

use tauri::State;

use crate::modules::brain::events::BrainEvent;
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

/// Trigger a full reconcile (add/change/delete) of all registered projects, or a
/// single project. Enqueues onto the worker — non-blocking.
#[tauri::command]
pub fn brain_rescan(state: State<BrainState>, project: Option<String>) -> Result<(), String> {
    let guard = state.tx.lock().map_err(|_| "brain tx poisoned".to_string())?;
    match guard.as_ref() {
        Some(tx) => tx
            .send(BrainEvent::Rescan { project })
            .map_err(|e| format!("brain worker unavailable: {e}")),
        None => Err("brain not started yet".to_string()),
    }
}
