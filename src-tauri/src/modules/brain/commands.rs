//! `#[tauri::command]` surface — the only thing the UI touches (CONCEPT §9).
//! Commands are READERS: they open their own read-only SQLite connection (WAL →
//! wait-free vs. the worker's writes) and never block the worker. Fail-open: a
//! warming/degraded brain returns partial/empty results, not errors.
//!
//! Threading (ADR-010 cluster 6): every command that opens a SQLite connection is
//! `async` (off the Tauri MAIN thread) and runs the read on the blocking pool via
//! [`blocking`], so a 5s `busy_timeout` during an indexing burst can freeze
//! neither the UI nor the async runtime. Pure in-memory/enqueue commands stay
//! sync — they are lock-snapshot + mpsc-send, microseconds on the main thread.

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
/// Async: the `is_dir`/canonicalize stats can stall on a dead network drive.
#[tauri::command]
pub async fn brain_add_project(state: State<'_, BrainState>, path: String) -> Result<Project, String> {
    let pb = std::path::PathBuf::from(&path);
    if !pb.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    // Same sanity gate as the boot seed (ADR-010 cluster 7): a drive/filesystem
    // root or the bare home dir would index a machine's worth of non-project
    // files. The workspace wizard is the intentional path for project parents.
    // The gate canonicalizes internally, so it judges the same path `add_root`
    // will register (a raw `c:\users\me` or `C:\x\..\..` cannot slip past).
    if !crate::modules::brain::worker::is_sane_root(&pb) {
        return Err(
            "refusing to index a filesystem root or home directory — add a project folder instead"
                .to_string(),
        );
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
    // Registry first (the worker's RemoveProject handler re-arms the watcher and
    // persists FROM the registry, so enqueue-first would race it) — but roll the
    // mutation back if the prune can't be enqueued, else the index keeps orphaned
    // rows while the persisted project list diverges (ADR-010 cluster 7).
    let removed = state.registry.remove(&project);
    if let Err(e) = enqueue(&state, BrainEvent::RemoveProject { project }) {
        if let Some(p) = removed {
            state.registry.restore(p);
        }
        return Err(e);
    }
    Ok(())
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
/// Async: the child-marker discovery walk is filesystem I/O (one-shot wizard action).
#[tauri::command]
pub async fn brain_set_workspace(
    state: State<'_, BrainState>,
    path: String,
) -> Result<Vec<Project>, String> {
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
pub async fn brain_index_status(state: State<'_, BrainState>) -> Result<BrainStatusReport, String> {
    let status = state
        .status
        .read()
        .map(|s| s.clone())
        .unwrap_or(BrainStatus::Degraded { reason: "status lock poisoned".into() });
    let db = state.db_path.read().ok().and_then(|p| p.clone());
    let registered = state.registry.projects();
    blocking(move || {
        let projects = registered
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
    })
    .await
}

/// Lexical (BM25 + weighted RRF) search across code (and, from P1, notes).
/// `project = None` searches every registered project.
#[tauri::command]
pub async fn brain_search(
    state: State<'_, BrainState>,
    project: Option<String>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<Hit>, String> {
    // Fail-open: not-ready brain → empty, not an error.
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(20).clamp(1, 200);
    blocking(move || match store::search_readonly(&db, project.as_deref(), &query, limit) {
        Ok(hits) => hits,
        // A missing/locked DB during warmup is not a user error.
        Err(e) => {
            log::debug!("brain_search soft error: {e}");
            Vec::new()
        }
    })
    .await
}

/// Build the cache-stable gist for a project + intent (P3). Zero tokens; an
/// unchanged relaunch returns a byte-identical gist. `None` if the index isn't ready.
#[tauri::command]
pub async fn brain_build_gist(
    state: State<'_, BrainState>,
    project: String,
    intent: String,
    budget: Option<usize>,
) -> Result<Option<Gist>, String> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(None);
    };
    let name = state
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project)
        .map(|p| p.name)
        .unwrap_or_else(|| project.clone());
    blocking(move || Some(gist::build_gist_auto(&db, &project, &name, &intent, budget.unwrap_or(800))))
        .await
}

/// Allowlist for ids spliced into a filename: `[A-Za-z0-9_-]{1,64}`. Rejects path
/// traversal (`..`, separators) and shell metachars by construction.
fn valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Build the gist (cold-start-synthesized if `intent` is blank) and write it to
/// the agent's `--append-system-prompt` file (`~/.koden/agent-<agent_id>.txt`) so
/// a launching agent gets project context. Returns the gist for the toast.
#[tauri::command]
pub async fn brain_write_gist(
    state: State<'_, BrainState>,
    project: String,
    intent: String,
    agent_id: String,
    budget: Option<usize>,
) -> Result<Gist, String> {
    // `agent_id` becomes a filename component under ~/.koden — reject, never sanitize.
    if !valid_agent_id(&agent_id) {
        return Err("invalid agent_id: expected [A-Za-z0-9_-]{1,64}".to_string());
    }
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
    blocking(move || {
        gist::write_gist(&db, &project, &name, &intent, budget.unwrap_or(800), &path)
            .map_err(|e| format!("write gist: {e}"))
    })
    .await?
}

/// Definition locations of a symbol (path/kind/line) — the AST graph (P2).
#[tauri::command]
pub async fn brain_get_symbol(
    state: State<'_, BrainState>,
    project: String,
    symbol: String,
) -> Result<Vec<SymbolInfo>, String> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(Vec::new());
    };
    blocking(move || store::get_symbol_readonly(&db, &project, &symbol).unwrap_or_default()).await
}

/// Tiered impact of a symbol: depth-annotated AST import-graph rows (upstream
/// dependents / downstream dependencies / both) + lexical candidates.
/// `direction` defaults to "upstream" (who depends on this); an unknown value
/// is a caller error. `max_results` bounds the row list (truncation is flagged
/// on the result); `exclude_tests` drops conventional test paths.
#[tauri::command]
pub async fn brain_code_impact(
    state: State<'_, BrainState>,
    project: String,
    symbol: String,
    depth: Option<usize>,
    direction: Option<String>,
    max_results: Option<usize>,
    exclude_tests: Option<bool>,
) -> Result<Impact, String> {
    let dir = crate::modules::brain::ast::ImpactDirection::parse(
        direction.as_deref().unwrap_or("upstream"),
    )
    .ok_or_else(|| "invalid direction: expected upstream | downstream | both".to_string())?;
    let fail_open = move |symbol: String| Impact {
        symbol,
        direction: dir.as_str().to_string(),
        ..Default::default()
    };
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(fail_open(symbol));
    };
    let depth = depth.unwrap_or(5).clamp(1, 20);
    let max_results = max_results.unwrap_or(200).clamp(1, 2000);
    let exclude_tests = exclude_tests.unwrap_or(false);
    blocking(move || {
        match store::code_impact_readonly(&db, &project, &symbol, depth, dir, max_results, exclude_tests) {
            Ok(impact) => impact,
            Err(_) => fail_open(symbol),
        }
    })
    .await
}

/// Map the project's git diff onto the index: affected indexed files + their
/// first-degree `code_edges` dependents. `mode` defaults to "both" (working ∪
/// staged); an unknown value is a caller error. A non-git root or unavailable
/// git is a SOFT result (`skipped_reason`), never an error — fail-open like
/// every other reader. The git probe is read-only (`diff --name-only`).
#[tauri::command]
pub async fn brain_detect_changes(
    state: State<'_, BrainState>,
    project: String,
    mode: Option<String>,
) -> Result<store::DetectedChanges, String> {
    let mode = store::DetectMode::parse(mode.as_deref().unwrap_or("both"))
        .ok_or_else(|| "invalid mode: expected working | staged | both".to_string())?;
    let root = state
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project)
        .map(|p| p.root)
        .ok_or_else(|| format!("unknown project: {project}"))?;
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(store::DetectedChanges::skipped(mode, "index-not-ready"));
    };
    blocking(move || {
        let root = std::path::Path::new(&root);
        store::detect_changes_readonly(&db, &project, root, mode).unwrap_or_else(|e| {
            // A missing/locked DB during warmup is not a user error.
            log::debug!("brain_detect_changes soft error: {e}");
            store::DetectedChanges::skipped(mode, "index-unavailable")
        })
    })
    .await
}

/// Churn hotspots (ADR-013 first step): indexed paths ranked by DISTINCT
/// commits touching them in a bounded `git log` window, optionally narrowed
/// by a git-parsed `since` (e.g. "2.weeks" or an ISO date). `limit` defaults
/// to 25 (clamped 1..200). Read-only end to end; a non-git root, bad `since`
/// shape, or unavailable git is a SOFT result (`skipped_reason`), never an
/// error — fail-open like every other reader.
#[tauri::command]
pub async fn brain_hotspots(
    state: State<'_, BrainState>,
    project: String,
    since: Option<String>,
    limit: Option<usize>,
) -> Result<store::Hotspots, String> {
    let root = state
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project)
        .map(|p| p.root)
        .ok_or_else(|| format!("unknown project: {project}"))?;
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(store::Hotspots::skipped("index-not-ready"));
    };
    blocking(move || {
        let root = std::path::Path::new(&root);
        store::hotspots_readonly(&db, &project, root, since.as_deref(), limit).unwrap_or_else(|e| {
            // A missing/locked DB during warmup is not a user error.
            log::debug!("brain_hotspots soft error: {e}");
            store::Hotspots::skipped("index-unavailable")
        })
    })
    .await
}

/// Paths touched between two git anchors (`from`..`to`, `to` defaulting to
/// HEAD), each mapped onto the index (ADR-013 first step). Read-only end to
/// end; a bad/unknown anchor, non-git root, or unavailable git is a SOFT
/// result (`skipped_reason`), never an error — fail-open like every other
/// reader.
#[tauri::command]
pub async fn brain_changed_between(
    state: State<'_, BrainState>,
    project: String,
    from: String,
    to: Option<String>,
) -> Result<store::ChangedBetween, String> {
    let root = state
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project)
        .map(|p| p.root)
        .ok_or_else(|| format!("unknown project: {project}"))?;
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        let to = to.as_deref().unwrap_or("HEAD");
        return Ok(store::ChangedBetween::skipped(&from, to, "index-not-ready"));
    };
    blocking(move || {
        let root = std::path::Path::new(&root);
        store::changed_between_readonly(&db, &project, root, &from, to.as_deref()).unwrap_or_else(
            |e| {
                // A missing/locked DB during warmup is not a user error.
                log::debug!("brain_changed_between soft error: {e}");
                let to = to.as_deref().unwrap_or("HEAD");
                store::ChangedBetween::skipped(&from, to, "index-unavailable")
            },
        )
    })
    .await
}

/// One-call read-only planning bundle: task-text search hits + git-diff
/// affected files + (when `target` is given) upstream impact of the target
/// symbol. Pure composition over the existing readers; each leg that cannot
/// run becomes an `advisories[]` entry and the rest of the bundle still
/// returns. An unknown project is a caller error (like `brain_detect_changes`);
/// a not-yet-ready index is a fully-advised soft bundle.
#[tauri::command]
pub async fn brain_plan_context(
    state: State<'_, BrainState>,
    project: String,
    task: String,
    target: Option<String>,
) -> Result<store::PlanContext, String> {
    let root = state
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project)
        .map(|p| p.root)
        .ok_or_else(|| format!("unknown project: {project}"))?;
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(store::PlanContext::skipped(task, target, "index-not-ready"));
    };
    blocking(move || {
        let root = std::path::Path::new(&root);
        store::plan_context_readonly(&db, &project, root, &task, target)
    })
    .await
}

/// Whole-brain knowledge graph for the Brain Map: project hubs + (capped) files +
/// memory notes, with containment/import/anchor edges. Read-only snapshot.
#[tauri::command]
pub async fn brain_graph(
    state: State<'_, BrainState>,
    max_files: Option<usize>,
) -> Result<store::BrainGraph, String> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(store::BrainGraph::default());
    };
    let projects: Vec<(String, String)> =
        state.registry.projects().into_iter().map(|p| (p.id, p.name)).collect();
    blocking(move || {
        store::graph_readonly(&db, &projects, max_files.unwrap_or(80).clamp(1, 2000))
            .unwrap_or_default()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::valid_agent_id;

    #[test]
    fn agent_id_allowlist_rejects_traversal_and_metachars() {
        // The shape the UI actually generates (`uid("ag")`).
        assert!(valid_agent_id("ag-lx3k2j-a1b2c3"));
        assert!(valid_agent_id("Agent_01"));
        // Rejections: empty, traversal, separators, shell metachars, over-long.
        assert!(!valid_agent_id(""));
        assert!(!valid_agent_id("../../.ssh/authorized_keys"));
        assert!(!valid_agent_id("..\\..\\evil"));
        assert!(!valid_agent_id("a/b"));
        assert!(!valid_agent_id("a b; rm -rf ~"));
        assert!(!valid_agent_id(&"a".repeat(65)));
    }
}

/// Structured memory notes (review inbox / cards). `project = None` = all.
#[tauri::command]
pub async fn brain_notes(
    state: State<'_, BrainState>,
    project: Option<String>,
) -> Result<Vec<NoteSummary>, String> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(Vec::new());
    };
    blocking(move || store::list_notes_readonly(&db, project.as_deref()).unwrap_or_default()).await
}

/// Trigger a full reconcile (add/change/delete) of all registered projects, or a
/// single project. Enqueues onto the worker — non-blocking.
#[tauri::command]
pub fn brain_rescan(state: State<BrainState>, project: Option<String>) -> Result<(), String> {
    enqueue(&state, BrainEvent::Rescan { project })
}

/// Pending memory proposals (the review inbox). `project = None` = all.
#[tauri::command]
pub async fn brain_proposals(
    state: State<'_, BrainState>,
    project: Option<String>,
) -> Result<Vec<MemoryProposal>, String> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(Vec::new());
    };
    blocking(move || store::list_proposals_readonly(&db, project.as_deref()).unwrap_or_default())
        .await
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

/// Set the reflect cumulative spend ceiling (USD). `0.0` disables reflect entirely.
/// The only feature that spends money, and it uses the user's own Anthropic key.
#[tauri::command]
pub fn brain_set_budget(state: State<BrainState>, ceiling_usd: f64) -> Result<(), String> {
    enqueue(&state, BrainEvent::SetBudget { ceiling_usd })
}

/// Set the Librarian's LLM provider/model (the budgeted reflect+curate path). The
/// key is read at call time from the per-provider `koden-ai` keyring account; local
/// providers (ollama/lmstudio/mlx) need none. `in_rate_usd_mtok`/`out_rate_usd_mtok`
/// are $/million-tokens (0 for free local models) so the spend meter stays accurate.
/// Defaults to Anthropic Haiku until set. Writer-side.
#[tauri::command]
pub fn brain_set_librarian(
    state: State<BrainState>,
    provider: String,
    model: String,
    base_url: String,
    in_rate_usd_mtok: f64,
    out_rate_usd_mtok: f64,
) -> Result<(), String> {
    enqueue(
        &state,
        BrainEvent::SetLibrarian {
            provider,
            model,
            base_url,
            in_rate_mtok: in_rate_usd_mtok,
            out_rate_mtok: out_rate_usd_mtok,
        },
    )
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
pub async fn brain_budget_status(state: State<'_, BrainState>) -> Result<(f64, f64), String> {
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok((0.0, 0.0));
    };
    blocking(move || store::budget_state_readonly(&db).unwrap_or((0.0, 0.0))).await
}

/// The current Librarian LLM selection (read-only). Defaults to Anthropic Haiku
/// when unset. Lets Settings show + edit which model the reflect/curate path uses.
#[derive(serde::Serialize)]
pub struct LibrarianStatus {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub in_rate_mtok: f64,
    pub out_rate_mtok: f64,
}

#[tauri::command]
pub async fn brain_librarian_status(state: State<'_, BrainState>) -> Result<LibrarianStatus, String> {
    let def = || LibrarianStatus {
        provider: "anthropic".to_string(),
        model: "claude-haiku-4-5".to_string(),
        base_url: String::new(),
        in_rate_mtok: 1.0,
        out_rate_mtok: 5.0,
    };
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(def());
    };
    blocking(move || match store::librarian_config_readonly(&db) {
        Ok((provider, model, base_url, in_rate_mtok, out_rate_mtok)) => {
            LibrarianStatus { provider, model, base_url, in_rate_mtok, out_rate_mtok }
        }
        Err(_) => def(),
    })
    .await
}

/// One Librarian LLM call from the budget ledger. `cost_usd` is the actual reported
/// cost once reconciled, else the conservative estimate (a still-`reserved` row).
#[derive(serde::Serialize)]
pub struct LedgerCall {
    pub status: String, // "reserved" | "spent"
    pub cost_usd: f64,
    pub model: String,
    pub at_ms: i64,
}

/// Read-only "is the Librarian actually working?" snapshot for the UI: the budget
/// meter, the pending-proposal count, and the most recent real LLM calls — so a
/// user can see activity without devtools or sqlite. Empty meter + no calls = it
/// has not spent (no key / no corpus / not triggered yet).
#[derive(serde::Serialize)]
pub struct LibrarianActivity {
    pub ceiling_usd: f64,
    pub spent_usd: f64,
    pub pending_proposals: i64,
    pub calls: Vec<LedgerCall>,
}

#[tauri::command]
pub async fn brain_librarian_activity(
    state: State<'_, BrainState>,
) -> Result<LibrarianActivity, String> {
    let empty = LibrarianActivity {
        ceiling_usd: 0.0,
        spent_usd: 0.0,
        pending_proposals: 0,
        calls: Vec::new(),
    };
    let Some(db) = state.db_path.read().ok().and_then(|p| p.clone()) else {
        return Ok(empty);
    };
    blocking(move || {
        let (ceiling_usd, spent_usd) = store::budget_state_readonly(&db).unwrap_or((0.0, 0.0));
        let pending_proposals = store::pending_proposals_readonly(&db).unwrap_or(0);
        let calls = store::librarian_ledger_readonly(&db, 12)
            .unwrap_or_default()
            .into_iter()
            .map(|(status, est, actual, model, at_ms)| LedgerCall {
                cost_usd: actual.unwrap_or(est),
                status,
                model,
                at_ms,
            })
            .collect();
        LibrarianActivity { ceiling_usd, spent_usd, pending_proposals, calls }
    })
    .await
}

/// Panes recoverable from the previous session (P4 crash-resume), computed at boot
/// from the per-pane journals. Drives the UI's "resume where you left off" cards.
#[tauri::command]
pub fn brain_recovered_panes(state: State<BrainState>) -> Vec<crate::modules::brain::resume::RecoveredPane> {
    state.recovered.read().map(|r| r.clone()).unwrap_or_default()
}

/// Run a blocking read-only store call on the blocking pool (mirrors
/// `git::commands::blocking`). The async command is already off the Tauri main
/// thread; this keeps a `busy_timeout`-bound SQLite read from starving the async
/// runtime too. `Err` only if the task can't be joined (it panicked) — which
/// previously would have unwound the main thread.
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("brain read task failed: {e}"))
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
