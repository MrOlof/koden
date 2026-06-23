//! The single GUI-resident Brain worker thread — a clone of the usage poller
//! template (`usage/poll.rs:384`). Spawned from `lib.rs .setup()` after the usage
//! poller; fail-open; never blocks first paint (spawn returns immediately, all
//! real work runs on this thread). CONCEPT §5.1, EXECUTION_PLAN §2.3.

use std::sync::mpsc;
use std::time::Duration;

use tauri::{AppHandle, Listener, Manager};

use crate::modules::brain::events::{AgentSignalPayload, BrainEvent};
use crate::modules::brain::freshness::{hash, walk, watch};
use crate::modules::brain::curate;
use crate::modules::brain::memory;
use crate::modules::brain::reflect;
use crate::modules::brain::registry::Project;
use crate::modules::brain::resume;
use crate::modules::brain::secrets;
use crate::modules::brain::store::SqliteIndex;
use crate::modules::brain::{BrainState, BrainStatus, LiveSession};
use crate::modules::fs::to_canon;
use crate::modules::pty::PtyState;

const AGENT_EVENT: &str = "koden:agent-signal";
const TICK_SECS: u64 = 60;
/// How often the autonomous Librarian does a "round": at most one delta-gated
/// reflect per project per this interval, and only when something changed since the
/// last round. Deliberately NOT a fixed change COUNT — a count is a bad proxy (20
/// edits means nothing in a huge repo, a lot in a tiny one). The digest hash inside
/// [reflect::reflect_auto] is the real "is there anything new to reflect on" signal;
/// this interval only sets *cadence*. The round is budget-gated (no ceiling ⇒ no
/// spend) and hash-gated (digest unchanged ⇒ $0), so it never decides overspend.
/// ponytail: fixed interval; lift to a user setting if someone wants to tune it.
const LIBRARIAN_ROUNDS_MS: i64 = 15 * 60 * 1000;
/// Binary sniff window — a NUL byte in the first 8 KiB means "not text".
const BINARY_SNIFF_BYTES: usize = 8192;

/// Spawn the worker. Mirrors `usage::poll::spawn_poller`. `launch_dir` is the
/// authorized launch directory (the dir the user opened); the brain seeds its
/// P0 project from it rather than blindly indexing the process cwd.
pub fn spawn_brain_worker(app: AppHandle, launch_dir: Option<String>) {
    std::thread::Builder::new()
        .name("koden-brain-worker".into())
        .spawn(move || brain_loop(app, launch_dir))
        .expect("spawn koden-brain worker thread");
}

fn set_status(app: &AppHandle, status: BrainStatus) {
    if let Some(state) = app.try_state::<BrainState>() {
        if let Ok(mut s) = state.status.write() {
            *s = status;
        }
    }
}

fn brain_loop(app: AppHandle, launch_dir: Option<String>) {
    // 1. Resolve + open the store (fail-open → Degraded, never panic).
    let db_path = match app.path().app_local_data_dir() {
        Ok(dir) => dir.join("koden").join("brain").join("index.sqlite"),
        Err(e) => {
            log::warn!("brain: no app_local_data_dir ({e}); brain disabled");
            set_status(&app, BrainStatus::Degraded { reason: "no data dir".into() });
            return;
        }
    };
    let index = match SqliteIndex::open(&db_path) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("brain: store open failed ({e}); degraded");
            set_status(&app, BrainStatus::Degraded { reason: format!("store open: {e}") });
            return;
        }
    };
    if let Some(state) = app.try_state::<BrainState>() {
        if let Ok(mut p) = state.db_path.write() {
            *p = Some(db_path.clone());
        }
    }

    // Boot sweep (P4): charge any orphaned budget reservation (reflect OR curation —
    // one shared ledger, no source filter) at its estimate, so a crashed paid call
    // over-counts rather than leaking free spend. Fail-open — never blocks startup.
    match index.sweep_orphaned_reservations(now_epoch_ms()) {
        Ok(n) if n > 0 => log::info!("brain: swept {n} orphaned reflect reservation(s)"),
        Ok(_) => {}
        Err(e) => log::warn!("brain: budget sweep failed ({e}); continuing"),
    }

    // Boot crash-resume recovery (P4): fold the per-pane journals into recoverable
    // panes for the UI's resume cards, then GC expired journals. Done BEFORE the
    // agent-signal listener is registered so a live signal can't race the recovered
    // map. Fail-open — no journals / a torn journal just yields fewer cards.
    if let Some(rdir) = resume_dir(&app) {
        let recovered = resume::recover_all(&rdir);
        if !recovered.is_empty() {
            log::info!("brain: {} pane(s) recoverable from the previous session", recovered.len());
        }
        if let Some(state) = app.try_state::<BrainState>() {
            if let Ok(mut r) = state.recovered.write() {
                *r = recovered;
            }
        }
        let gc = resume::gc_resume_dir(&rdir, now_epoch_ms(), resume::RESUME_TTL_DAYS);
        if gc > 0 {
            log::debug!("brain: GC'd {gc} expired resume journal(s)");
        }
    }

    // 2. Internal event channel; register the sender so commands can enqueue.
    let (tx, rx) = mpsc::channel::<BrainEvent>();
    if let Some(state) = app.try_state::<BrainState>() {
        if let Ok(mut slot) = state.tx.lock() {
            *slot = Some(tx.clone());
        }
    }

    // 3. Agent lifecycle leg (B2: deserialize into our own payload type).
    {
        let tx_agent = tx.clone();
        app.listen(AGENT_EVENT, move |event| {
            if let Ok(p) = serde_json::from_str::<AgentSignalPayload>(event.payload()) {
                let _ = tx_agent.send(BrainEvent::Agent {
                    pty_id: p.id,
                    kind: p.kind,
                    agent: p.agent,
                });
            }
        });
    }

    // 4. Periodic self-tick (flush WAL / future ledger reconcile). Fail-open: if
    // the thread can't spawn, carry on without the periodic checkpoint.
    {
        let tx_tick = tx.clone();
        let tick = std::thread::Builder::new()
            .name("koden-brain-tick".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(TICK_SECS));
                if tx_tick.send(BrainEvent::Tick).is_err() {
                    break;
                }
            });
        if let Err(e) = tick {
            log::warn!("brain: tick thread spawn failed ({e}); no periodic checkpoint");
        }
    }

    // 5. Bootstrap registry: load the persisted workspace.json (the source of truth)
    // so projects + the workspace root survive restarts; fall back to the authorized
    // launch dir only on first run (no config yet).
    let cfg_path = db_path.with_file_name("workspace.json");
    let loaded = app
        .try_state::<BrainState>()
        .map(|s| s.registry.load_from(&cfg_path))
        .unwrap_or(false);
    let has_projects = app
        .try_state::<BrainState>()
        .map(|s| !s.registry.projects().is_empty())
        .unwrap_or(false);
    if !loaded || !has_projects {
        seed_registry(&app, launch_dir.as_deref());
    }

    // 6. Warm population — project by project so the first is searchable early.
    warm_population(&app, &index);
    // Seed the review inbox with structural doctor findings (no date check yet).
    run_doctor_all(&app, &index, None);
    set_status(&app, BrainStatus::Ready);

    // 6b. Arm the recursive watcher over each seeded project root (P1 freshness).
    // Held for the worker's lifetime — dropping it stops watching. Re-armed on a
    // full Rescan so newly-registered projects get watched too.
    let mut watcher = arm_watcher(&app, &tx);

    // Per-project autonomous-reflect bookkeeping (worker-thread-local; resets on
    // restart, which is fine — dirty flags only matter while the app is open).
    let mut lib_state: std::collections::HashMap<String, LibrarianAuto> = std::collections::HashMap::new();

    // 7. Steady-state event loop. Single writer; ingest paths only send events.
    for ev in rx {
        match ev {
            BrainEvent::Agent { pty_id, kind, agent } => handle_agent(&app, pty_id, &kind, agent),
            BrainEvent::Rescan { .. } => {
                warm_population(&app, &index); // full reconcile (add/change/delete)
                drop(watcher.take()); // drop the old watcher first → no double-watch window
                watcher = arm_watcher(&app, &tx); // pick up any new project roots
                if let Some(s) = app.try_state::<BrainState>() {
                    s.registry.save_to(&cfg_path); // persist the project list
                }
            }
            BrainEvent::RemoveProject { project } => {
                if let Err(e) = index.remove_project(&project) {
                    log::warn!("brain: remove_project '{project}' prune failed ({e})");
                } else {
                    log::info!("brain: removed project '{project}' (unregistered + pruned)");
                }
                drop(watcher.take());
                watcher = arm_watcher(&app, &tx); // stop watching the removed root
                if let Some(s) = app.try_state::<BrainState>() {
                    s.registry.save_to(&cfg_path); // persist the updated project list
                }
            }
            BrainEvent::Tick => {
                index.checkpoint();
                run_librarian_rounds(&app, &index, &mut lib_state);
            }
            BrainEvent::Doctor { project, now_date } => {
                let now_ms = now_epoch_ms();
                let pids: Vec<String> = match &project {
                    Some(p) => vec![p.clone()],
                    None => app
                        .try_state::<BrainState>()
                        .map(|s| s.registry.projects().into_iter().map(|p| p.id).collect())
                        .unwrap_or_default(),
                };
                for pid in pids {
                    let n = memory::doctor::run_doctor(&index, &pid, now_date.as_deref(), now_ms);
                    if n > 0 {
                        log::info!("brain: doctor queued {n} proposal(s) for '{pid}'");
                    }
                }
            }
            BrainEvent::ResolveProposal { project, signature, reject } => {
                let _ = index.resolve_proposal(&project, &signature, reject);
            }
            BrainEvent::SetBudget { ceiling_usd } => {
                if let Err(e) = index.set_budget_ceiling(ceiling_usd, now_epoch_ms()) {
                    log::warn!("brain: set budget ceiling failed ({e})");
                }
            }
            BrainEvent::SetLibrarian { provider, model, base_url, in_rate_mtok, out_rate_mtok } => {
                let cfg = crate::modules::brain::reflect::librarian::LibrarianConfig {
                    provider,
                    model,
                    base_url,
                    in_rate_mtok,
                    out_rate_mtok,
                };
                if let Err(e) = index.set_librarian_config(&cfg, now_epoch_ms()) {
                    log::warn!("brain: set librarian config failed ({e})");
                }
            }
            BrainEvent::Curate { project, now_date } => {
                let pids: Vec<String> = match &project {
                    Some(p) => vec![p.clone()],
                    None => app
                        .try_state::<BrainState>()
                        .map(|s| s.registry.projects().into_iter().map(|p| p.id).collect())
                        .unwrap_or_default(),
                };
                let now_ms = now_epoch_ms();
                for pid in pids {
                    let o = curate::curate_once(&app, &index, &pid, now_date.as_deref(), now_ms);
                    log::info!(
                        "brain: curate '{pid}' → {:?} ({} proposal(s): {} acted/{} escalated, ${:.4})",
                        o.reason,
                        o.proposals.len(),
                        o.acted,
                        o.escalated,
                        o.spent_usd
                    );
                    // V2.4: contradiction detection over co-anchored pairs (shares the
                    // same budget ledger; a separate paid pass).
                    let c = curate::contradiction::curate_contradictions_once(&app, &index, &pid, now_ms);
                    if c.escalated > 0 || !c.proposals.is_empty() {
                        log::info!(
                            "brain: contradictions '{pid}' → {:?} ({} flagged, {} judged, ${:.4})",
                            c.reason,
                            c.proposals.len(),
                            c.escalated,
                            c.spent_usd
                        );
                    }
                }
            }
            BrainEvent::Reflect { project, now_date } => {
                // Manual, single-flight, $0-by-default. The network call blocks this
                // worker briefly (acceptable for a rare manual action); fail-open.
                let pids: Vec<String> = match &project {
                    Some(p) => vec![p.clone()],
                    None => app
                        .try_state::<BrainState>()
                        .map(|s| s.registry.projects().into_iter().map(|p| p.id).collect())
                        .unwrap_or_default(),
                };
                let now_ms = now_epoch_ms();
                for pid in pids {
                    let outcome = reflect::reflect_once(&app, &index, &pid, now_date.as_deref(), now_ms);
                    log::info!(
                        "brain: reflect '{pid}' → {:?} ({} proposal(s), ${:.4})",
                        outcome.reason,
                        outcome.proposals.len(),
                        outcome.spent_usd
                    );
                }
            }
            BrainEvent::Fs { project, changed } => {
                if let Some(root) = project_root(&app, &project) {
                    let stats = index_changed(&index, &project, std::path::Path::new(&root), &changed);
                    if stats.indexed > 0 || stats.pruned > 0 {
                        log::debug!(
                            "brain: incremental '{project}' indexed {}, pruned {}",
                            stats.indexed,
                            stats.pruned
                        );
                        // Mark dirty; the periodic Librarian round (Tick) decides via
                        // the digest hash whether there's anything new to reflect on.
                        lib_state.entry(project.clone()).or_default().dirty = true;
                    }
                }
            }
        }
    }
    drop(watcher);
}

fn arm_watcher(
    app: &AppHandle,
    tx: &mpsc::Sender<BrainEvent>,
) -> Option<notify::RecommendedWatcher> {
    let projects: Vec<(String, String)> = app
        .try_state::<BrainState>()
        .map(|s| {
            s.registry
                .projects()
                .into_iter()
                .map(|p| (p.id, p.root))
                .collect()
        })
        .unwrap_or_default();
    watch::spawn(projects, tx.clone())
}

fn seed_registry(app: &AppHandle, launch_dir: Option<&str>) {
    let Some(state) = app.try_state::<BrainState>() else {
        return;
    };
    // Prefer the explicit authorized launch dir. Fall back to the process cwd
    // ONLY if it looks like a project root — never blindly index a packaged app's
    // install dir, a filesystem root, or the home dir.
    let root = launch_dir
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok().filter(|p| has_project_marker(p)));
    match root {
        Some(p) if is_sane_root(&p) => match state.registry.add_root(&p) {
            Some(proj) => log::info!("brain: seeded project '{}' ({})", proj.name, proj.root),
            None => log::warn!("brain: failed to seed project for {}", p.display()),
        },
        _ => log::info!("brain: no seed project (awaiting wizard / brain_rescan)"),
    }
}

pub fn has_project_marker(p: &std::path::Path) -> bool {
    [".git", "package.json", "Cargo.toml", "pyproject.toml", "go.mod", ".kodenignore"]
        .iter()
        .any(|m| p.join(m).exists())
}

fn is_ignored_dir(p: &std::path::Path) -> bool {
    matches!(
        p.file_name().and_then(|s| s.to_str()),
        Some(n) if n.starts_with('.') || n == "node_modules" || n == "target" || n == "dist"
    )
}

/// Immediate child directories of `root` that look like real projects (have a
/// project marker). The workspace-root setup registers each as its OWN project, so a
/// parent of 20 repos becomes 20 hubs — not one giant parent project.
pub fn discover_workspace_projects(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !is_ignored_dir(p) && has_project_marker(p))
        .collect();
    out.sort();
    out
}

fn is_sane_root(p: &std::path::Path) -> bool {
    if p.parent().is_none() {
        return false; // filesystem root
    }
    if dirs::home_dir().as_deref() == Some(p) {
        return false; // bare home dir — the wizard handles intentional home workspaces
    }
    true
}

fn warm_population(app: &AppHandle, index: &SqliteIndex) {
    let projects = match app.try_state::<BrainState>() {
        Some(state) => state.registry.projects(),
        None => return,
    };
    let total = projects.len().max(1);
    for (i, proj) in projects.iter().enumerate() {
        set_status(app, BrainStatus::Warming { pct: ((i * 100) / total) as u8 });
        index_project(index, proj);
    }
}

/// Counts from one indexing pass.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct IndexStats {
    pub indexed: usize,
    pub pruned: usize,
}

/// Read → binary-sniff → blake3 → secrets-redact → index one file. Returns true
/// iff present (indexed OR an unchanged no-op; false on read error / binary / store
/// error). Shared by the full walk (`index_dir`) and the incremental watcher
/// (`index_changed`). On a REAL content change (index_file → Ok(true)) it stamps the
/// temporal recency via `record_access(now_ms)`; an unchanged no-op (Ok(false)) does
/// NOT re-stamp — so a warm pass over an unchanged index leaves accessed_at_ms fixed,
/// preserving the gist byte-identity gate ([DP-12]).
fn index_one_file(
    index: &SqliteIndex,
    project_id: &str,
    rel: &str,
    path: &std::path::Path,
    now_ms: i64,
) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    // Binary sniff — skip files with a NUL in the first window.
    if bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
        return false;
    }
    // Freshness hash is over the RAW bytes (any change reindexes).
    let file_hash = hash::hash_bytes(&bytes);
    let content = String::from_utf8_lossy(&bytes);
    // Secrets gate: redact secret-shaped content before it is tokenized/stored.
    let (redacted, nredact) = secrets::redact(&content);
    if nredact > 0 {
        log::debug!("brain: redacted {nredact} secret-shaped span(s) in {rel}");
    }
    match index.index_file(project_id, rel, &redacted, &file_hash, bytes.len() as i64) {
        Ok(true) => {
            // Real change → advance recency (only here, so unchanged passes don't move it).
            let _ = index.record_access(project_id, rel, now_ms);
            true
        }
        Ok(false) => true, // unchanged no-op — present, but recency unchanged
        Err(e) => {
            log::debug!("brain: index_file failed for {rel}: {e}");
            false
        }
    }
}

/// The per-project indexing pipeline: walk → (per file) index → reconcile-delete.
/// Deliberately free of `AppHandle`/registry so the deterministic offline sandbox
/// and integration tests drive the **real** pipeline (BUILD-PROMPT §6.5). `root`
/// is the absolute project root; `project_id` the id the rows are keyed under.
pub fn index_dir(index: &SqliteIndex, project_id: &str, root: &std::path::Path) -> IndexStats {
    let files = walk::walk_files(root);
    let now_ms = now_epoch_ms(); // one recency stamp for everything changed in this pass
    let mut indexed = 0usize;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in files {
        let rel = rel_path(root, &path);
        if index_one_file(index, project_id, &rel, &path, now_ms) {
            seen.insert(rel);
            indexed += 1;
        }
    }
    // Reconcile deletions: prune index rows for files no longer present on disk
    // (CONCEPT Flow B delta; EXECUTION_PLAN §3 SearchIndex::remove). Without this,
    // a deleted/moved file would match searches forever.
    let mut pruned = 0usize;
    if let Ok(existing) = index.existing_paths(project_id) {
        for rel in existing {
            if !seen.contains(&rel) && index.remove_file(project_id, &rel).unwrap_or(false) {
                pruned += 1;
            }
        }
    }
    // Rebuild resolved import edges once (pure fn of imports + file set).
    let _ = index.rebuild_edges(project_id);
    IndexStats { indexed, pruned }
}

fn index_project(index: &SqliteIndex, proj: &Project) {
    let root = std::path::Path::new(&proj.root);
    let stats = index_dir(index, &proj.id, root);
    let notes = memory::scan_project_memory(index, &proj.id, root);
    log::info!(
        "brain: project '{}' indexed {}, pruned {}, notes {}",
        proj.name,
        stats.indexed,
        stats.pruned,
        notes
    );
}

/// Incremental reindex of specific changed paths (from the recursive watcher) for
/// one project: existing text files are re-indexed (the hash-skip makes no-ops
/// cheap), vanished files are pruned. Touches ONLY the given paths — the P1
/// freshness gate ("an out-of-band edit reindexes only the changed file").
pub fn index_changed(
    index: &SqliteIndex,
    project_id: &str,
    root: &std::path::Path,
    changed: &[std::path::PathBuf],
) -> IndexStats {
    let now_ms = now_epoch_ms(); // recency stamp for whatever changed in this delta
    let mut indexed = 0usize;
    let mut pruned = 0usize;
    for path in changed {
        if walk::under_skip_dir(path) || secrets::is_denylisted_path(&to_canon(path)) {
            continue;
        }
        let rel = rel_path(root, path);
        if rel.is_empty() {
            continue;
        }
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => {
                if m.len() > walk::MAX_INDEX_FILE_BYTES {
                    continue;
                }
                if index_one_file(index, project_id, &rel, path, now_ms) {
                    indexed += 1;
                }
            }
            Ok(_) => {
                // A directory event (e.g. an atomic move-in of an existing tree)
                // whose children weren't individually reported — index them so the
                // incremental graph converges with a full rebuild.
                for child in walk::walk_files(path) {
                    let crel = rel_path(root, &child);
                    if !crel.is_empty() && index_one_file(index, project_id, &crel, &child, now_ms) {
                        indexed += 1;
                    }
                }
            }
            Err(_) => {
                // gone (deleted / moved away) — prune the stale row + FTS doc.
                if index.remove_file(project_id, &rel).unwrap_or(false) {
                    pruned += 1;
                }
                // The vanished path may have been a directory whose children were
                // not individually reported — prune any indexed files under it.
                if let Ok(existing) = index.existing_paths(project_id) {
                    let prefix = format!("{rel}/");
                    for p in existing {
                        if p.starts_with(&prefix)
                            && index.remove_file(project_id, &p).unwrap_or(false)
                        {
                            pruned += 1;
                        }
                    }
                }
            }
        }
    }
    // If a memory note changed, re-sync the structured notes table for the project.
    let mem_marker = format!("/{}/", memory::MEMORY_DIR);
    if changed.iter().any(|p| to_canon(p).contains(&mem_marker)) {
        memory::scan_project_memory(index, project_id, root);
    }
    // Rebuild resolved import edges (cheap; converges with a full rebuild).
    let _ = index.rebuild_edges(project_id);
    IndexStats { indexed, pruned }
}

/// Per-project autonomous-reflect bookkeeping. `dirty` = something changed since the
/// last round; `digest_hash` = the hash of the last digest we reflected on (drives
/// the delta gate). Worker-thread-local; resets on restart (fine — only matters
/// while the app is open).
#[derive(Default)]
struct LibrarianAuto {
    dirty: bool,
    last_pass_ms: i64,
    digest_hash: Option<String>,
}

/// Pure round predicate (testable): something changed AND a full round interval has
/// passed. No change COUNT — the digest hash inside [reflect::reflect_auto] is the
/// real "is there anything new" signal; this only decides cadence.
fn due_for_round(dirty: bool, last_pass_ms: i64, now_ms: i64) -> bool {
    dirty && now_ms.saturating_sub(last_pass_ms) >= LIBRARIAN_ROUNDS_MS
}

/// One Librarian sweep (driven by the periodic Tick): for each project that changed
/// since its last round and is past the round interval, run ONE delta-gated reflect.
/// End-to-end safe: [reflect::reflect_auto] no-ops (Disabled) without a budget
/// ceiling and skips the paid call ($0, Unchanged) when the digest is byte-identical
/// to the last round. The network call briefly blocks this worker — acceptable for a
/// rare, budgeted background action.
fn run_librarian_rounds(
    app: &AppHandle,
    index: &SqliteIndex,
    state: &mut std::collections::HashMap<String, LibrarianAuto>,
) {
    let now_ms = now_epoch_ms();
    for (project_id, st) in state.iter_mut() {
        if !due_for_round(st.dirty, st.last_pass_ms, now_ms) {
            continue;
        }
        st.dirty = false;
        st.last_pass_ms = now_ms; // gate the next round regardless of outcome
        let prev = st.digest_hash.clone();
        let (outcome, digest_hash) =
            reflect::reflect_auto(app, index, project_id, None, now_ms, prev.as_deref());
        match outcome.reason {
            reflect::ReflectReason::Ok | reflect::ReflectReason::Unchanged => {
                st.digest_hash = digest_hash;
                log::info!(
                    "brain: auto-reflect '{project_id}' → {:?} ({} proposal(s), ${:.4})",
                    outcome.reason,
                    outcome.proposals.len(),
                    outcome.spent_usd
                );
            }
            // Disabled/NoKey/OverBudget/CallFailed/InvalidOutput: re-arm so the next
            // round retries (recovers once a budget is set / a transient error clears).
            other => {
                st.dirty = true;
                log::debug!("brain: auto-reflect '{project_id}' skipped: {other:?}");
            }
        }
    }
}

fn now_epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Run the doctor across every registered project (boot-time inbox seed).
fn run_doctor_all(app: &AppHandle, index: &SqliteIndex, now_date: Option<&str>) {
    let Some(state) = app.try_state::<BrainState>() else {
        return;
    };
    let now_ms = now_epoch_ms();
    for proj in state.registry.projects() {
        let n = memory::doctor::run_doctor(index, &proj.id, now_date, now_ms);
        if n > 0 {
            log::info!("brain: doctor seeded {n} proposal(s) for '{}'", proj.name);
        }
    }
}

fn project_root(app: &AppHandle, project_id: &str) -> Option<String> {
    app.try_state::<BrainState>()?
        .registry
        .projects()
        .into_iter()
        .find(|p| p.id == project_id)
        .map(|p| p.root)
}

/// Project-relative, forward-slash path. Routes both sides through `to_canon` so
/// the full walk and the incremental watcher (which sees native absolute paths
/// — possibly `\\?\`-prefixed on Windows) produce the SAME rel for a given file.
fn rel_path(root: &std::path::Path, path: &std::path::Path) -> String {
    let root_c = to_canon(root);
    let path_c = to_canon(path);
    path_c
        .strip_prefix(&root_c)
        .map(|r| r.trim_start_matches('/').to_string())
        .unwrap_or(path_c)
}

/// The brain-private data dir (`<app_local>/koden/brain/resume`) for the P4
/// journals — alongside `index.sqlite`, NOT the frontend's `~/.koden` agent bus.
fn resume_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_local_data_dir()
        .ok()
        .map(|d| d.join("koden").join("brain").join("resume"))
}

/// Update the live per-pane session map from agent lifecycle signals AND journal
/// the event for crash-resume (P4). Resolves pty → cwd (B1/B3 accessor) → project
/// (registry longest-prefix) for EVERY kind. cwd is remembered on the session so an
/// `exited` signal can still derive its key after the pty session map has dropped.
fn handle_agent(app: &AppHandle, pty_id: u32, kind: &str, agent: Option<String>) {
    let Some(brain) = app.try_state::<BrainState>() else {
        return;
    };
    // The agent name + cwd arrive only on the 'started' signal (agent_detect sets
    // agent=None on working/attention/finished/exited), so remember both on the
    // session and reuse them for every later signal — otherwise each kind would
    // hash a DIFFERENT SessionKey and 'exited' would never reach the 'started'
    // journal (stale recovery card forever; Tier-2 never fires).
    let (remembered_cwd, remembered_agent) = brain
        .sessions
        .read()
        .ok()
        .and_then(|s| s.get(&pty_id).map(|x| (x.cwd.clone(), x.agent.clone())))
        .unwrap_or((None, None));
    let live_cwd = app.try_state::<PtyState>().and_then(|pty| pty.session_cwd(pty_id));
    let cwd = live_cwd.or(remembered_cwd);
    let effective_agent = agent.clone().or(remembered_agent);
    let project = cwd.as_deref().and_then(|c| brain.registry.resolve(c)).map(|p| p.id);

    // Journal the lifecycle event (fail-open). pane_uuid is None until P4-a wires a
    // restart-stable uuid; the key falls back to cwd+agent (spec-sanctioned).
    if let (Some(cwd_s), Some(rdir)) = (cwd.as_ref(), resume_dir(app)) {
        let key = resume::SessionKey::derive(cwd_s, effective_agent.as_deref().unwrap_or(""), None);
        let rec = resume::ResumeRecord {
            ts: now_epoch_ms(),
            kind: kind.to_string(),
            agent: effective_agent.clone(),
            cwd: cwd_s.clone(),
            project: project.clone(),
            claude_session_id: None,
        };
        if let Err(e) = resume::record_event(&rdir, &key, &rec) {
            log::debug!("brain: resume journal write failed ({e})");
        }
    }

    match kind {
        "started" => {
            if let Ok(mut sessions) = brain.sessions.write() {
                sessions.insert(pty_id, LiveSession { project, agent: effective_agent, cwd });
            }
        }
        "exited" => {
            if let Ok(mut sessions) = brain.sessions.write() {
                sessions.remove(&pty_id);
            }
        }
        _ => {} // working / attention / finished — status only, not tracked in P0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_for_round_needs_dirty_and_interval() {
        let g = LIBRARIAN_ROUNDS_MS;
        // Not dirty → never, however long it's been.
        assert!(!due_for_round(false, 0, g * 10));
        // Dirty AND a full interval elapsed → due.
        assert!(due_for_round(true, 0, g));
        // Dirty but still inside the interval → hold.
        assert!(!due_for_round(true, 1_000, 1_000 + g - 1));
    }
}
