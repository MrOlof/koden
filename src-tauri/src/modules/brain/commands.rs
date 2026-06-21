//! `#[tauri::command]` surface — the only thing the UI touches (CONCEPT §9).
//! Commands are READERS: they open their own read-only SQLite connection (WAL →
//! wait-free vs. the worker's writes) and never block the worker. Fail-open: a
//! warming/degraded brain returns partial/empty results, not errors.

use tauri::State;

use crate::modules::brain::ast::{Impact, SymbolInfo};
use crate::modules::brain::events::BrainEvent;
use crate::modules::brain::gist::{self, Gist};
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

/// Register a new project root (the wizard / "add folder" flow) and trigger a
/// reconcile so it gets indexed + watched. Returns the registered project.
#[tauri::command]
pub fn brain_add_project(state: State<BrainState>, path: String) -> Result<Project, String> {
    let pb = std::path::PathBuf::from(&path);
    if !pb.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let proj = state
        .registry
        .add_root(&pb)
        .ok_or_else(|| "could not register project".to_string())?;
    // Reconcile-all re-indexes the new project and re-arms the watcher to cover it.
    enqueue(&state, BrainEvent::Rescan { project: None })?;
    Ok(proj)
}

/// Unregister a project and prune all its indexed state. Does NOT delete user files
/// (brain-local only). Removed from the registry immediately; the index prune + a
/// watcher re-arm happen on the worker.
#[tauri::command]
pub fn brain_remove_project(state: State<BrainState>, project: String) -> Result<(), String> {
    state.registry.remove(&project);
    enqueue(&state, BrainEvent::RemoveProject { project })
}

/// Status of the workspace/source-of-truth setup — drives the first-run wizard.
#[derive(serde::Serialize)]
pub struct WorkspaceStatus {
    pub root: Option<String>,
    pub configured: bool,
    pub projects: usize,
}

/// Whether a workspace is set up (a root was chosen OR any project is registered).
#[tauri::command]
pub fn brain_workspace_status(state: State<BrainState>) -> WorkspaceStatus {
    let root = state.registry.workspace_root();
    let projects = state.registry.projects().len();
    // `configured` = the user has explicitly chosen a workspace root. An auto-seeded
    // launch-dir project does NOT count, so the first-run wizard still appears.
    WorkspaceStatus { configured: root.is_some(), root, projects }
}

/// Set the workspace root (source of truth) and auto-register each immediate child
/// that looks like a real project (has .git / a manifest) as its OWN project. Returns
/// the added projects; the worker indexes them, re-arms the watcher, and persists.
#[tauri::command]
pub fn brain_set_workspace(state: State<BrainState>, path: String) -> Result<Vec<Project>, String> {
    let root = std::path::PathBuf::from(&path);
    if !root.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let root_norm = crate::modules::fs::to_canon(&root).trim_end_matches('/').to_string();
    state.registry.set_workspace_root(Some(root_norm));
    let mut added = Vec::new();
    for child in crate::modules::brain::worker::discover_workspace_projects(&root) {
        if let Some(p) = state.registry.add_root(&child) {
            added.push(p);
        }
    }
    enqueue(&state, BrainEvent::Rescan { project: None })?;
    Ok(added)
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

/// Build the cache-stable gist for a project + intent (P3). Zero tokens; an
/// unchanged relaunch returns a byte-identical gist. `None` if the index isn't ready.
#[tauri::command]
pub fn brain_build_gist(
    state: State<BrainState>,
    project: String,
    intent: String,
    budget: Option<usize>,
) -> Option<Gist> {
    let db = state.db_path.read().ok().and_then(|p| p.clone())?;
    let name = state
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project)
        .map(|p| p.name)
        .unwrap_or_else(|| project.clone());
    Some(gist::build_gist_auto(&db, &project, &name, &intent, budget.unwrap_or(800)))
}

/// Build the gist (cold-start-synthesized if `intent` is blank) and write it to
/// the agent's `--append-system-prompt` file (`~/.koden/agent-<agent_id>.txt`) so
/// a launching agent gets project context. Returns the gist for the toast.
#[tauri::command]
pub fn brain_write_gist(
    state: State<BrainState>,
    project: String,
    intent: String,
    agent_id: String,
    budget: Option<usize>,
) -> Result<Gist, String> {
    let db = state
        .db_path
        .read()
        .ok()
        .and_then(|p| p.clone())
        .ok_or_else(|| "brain index not ready".to_string())?;
    let name = state
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project)
        .map(|p| p.name)
        .unwrap_or_else(|| project.clone());
    let home = dirs::home_dir().ok_or_else(|| "no home dir".to_string())?;
    let path = home.join(".koden").join(format!("agent-{agent_id}.txt"));
    gist::write_gist(&db, &project, &name, &intent, budget.unwrap_or(800), &path)
        .map_err(|e| format!("write gist: {e}"))
}

/// Definition locations of a symbol (path/kind/line) — the AST graph (P2).
#[tauri::command]
pub fn brain_get_symbol(
    state: State<BrainState>,
    project: String,
    symbol: String,
) -> Vec<SymbolInfo> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Vec::new();
    };
    store::get_symbol_readonly(&db, &project, &symbol).unwrap_or_default()
}

/// Tiered impact of a symbol: AST reverse-import dependents + lexical candidates.
#[tauri::command]
pub fn brain_code_impact(
    state: State<BrainState>,
    project: String,
    symbol: String,
    depth: Option<usize>,
) -> Impact {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Impact { symbol, ..Default::default() };
    };
    let depth = depth.unwrap_or(5).clamp(1, 20);
    store::code_impact_readonly(&db, &project, &symbol, depth).unwrap_or(Impact {
        symbol,
        ..Default::default()
    })
}

/// Whole-brain knowledge graph for the Brain Map: project hubs + (capped) files +
/// memory notes, with containment/import/anchor edges. Read-only snapshot.
#[tauri::command]
pub fn brain_graph(state: State<BrainState>, max_files: Option<usize>) -> store::BrainGraph {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return store::BrainGraph::default();
    };
    let projects: Vec<(String, String)> =
        state.registry.projects().into_iter().map(|p| (p.id, p.name)).collect();
    store::graph_readonly(&db, &projects, max_files.unwrap_or(80).clamp(1, 2000)).unwrap_or_default()
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

/// Trigger a budgeted LLM reflect pass (P4) — the only token-spending path.
/// Runs on the worker (single writer); proposals land in the review inbox and the
/// spend updates `brain_budget_status`. Off unless a ceiling > 0 is set. Manual only.
#[tauri::command]
pub fn brain_reflect(
    state: State<BrainState>,
    project: Option<String>,
    now_date: Option<String>,
) -> Result<(), String> {
    enqueue(&state, BrainEvent::Reflect { project, now_date })
}

/// Set the reflect monthly spend ceiling (USD). `0.0` disables reflect entirely.
/// The only feature that spends money, and it uses the user's own Anthropic key.
#[tauri::command]
pub fn brain_set_budget(state: State<BrainState>, ceiling_usd: f64) -> Result<(), String> {
    enqueue(&state, BrainEvent::SetBudget { ceiling_usd })
}

/// Run stale-ADR / memory curation (V2 Flow G) on the worker. Decisive stale notes
/// get a $0 archive proposal; borderline ones escalate to a budget-gated LLM
/// classification. Archive-biased, human-gated — never edits/deletes a user file.
#[tauri::command]
pub fn brain_curate(
    state: State<BrainState>,
    project: Option<String>,
    now_date: Option<String>,
) -> Result<(), String> {
    enqueue(&state, BrainEvent::Curate { project, now_date })
}

/// Reflect budget meter: `(ceiling_usd, spent_total_usd)`. Read-only.
#[tauri::command]
pub fn brain_budget_status(state: State<BrainState>) -> (f64, f64) {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return (0.0, 0.0);
    };
    store::budget_state_readonly(&db).unwrap_or((0.0, 0.0))
}

/// Panes recoverable from the previous session (P4 crash-resume), computed at boot
/// from the per-pane journals. Drives the UI's "resume where you left off" cards.
#[tauri::command]
pub fn brain_recovered_panes(state: State<BrainState>) -> Vec<crate::modules::brain::resume::RecoveredPane> {
    state.recovered.read().map(|r| r.clone()).unwrap_or_default()
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
