//! The single GUI-resident Brain worker thread — a clone of the usage poller
//! template (`usage/poll.rs:384`). Spawned from `lib.rs .setup()` after the usage
//! poller; fail-open; never blocks first paint (spawn returns immediately, all
//! real work runs on this thread). CONCEPT §5.1, EXECUTION_PLAN §2.3.

use std::sync::mpsc;
use std::time::Duration;

use tauri::{AppHandle, Listener, Manager};

use crate::modules::brain::events::{AgentSignalPayload, BrainEvent};
use crate::modules::brain::freshness::{hash, walk};
use crate::modules::brain::registry::Project;
use crate::modules::brain::secrets;
use crate::modules::brain::store::SqliteIndex;
use crate::modules::brain::{BrainState, BrainStatus, LiveSession};
use crate::modules::pty::PtyState;

const AGENT_EVENT: &str = "koden:agent-signal";
const TICK_SECS: u64 = 60;
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

    // 5. Bootstrap registry (P0: authorized launch dir; P1 wizard manages the
    // canonical multi-project source).
    seed_registry(&app, launch_dir.as_deref());

    // 6. Warm population — project by project so the first is searchable early.
    warm_population(&app, &index);
    set_status(&app, BrainStatus::Ready);

    // 7. Steady-state event loop. Single writer; ingest paths only send events.
    for ev in rx {
        match ev {
            BrainEvent::Agent { pty_id, kind, agent } => handle_agent(&app, pty_id, &kind, agent),
            BrainEvent::Rescan { .. } => warm_population(&app, &index), // P0: full reconcile
            BrainEvent::Tick => index.checkpoint(),
            BrainEvent::Fs { .. } => { /* P1: incremental reindex from the watcher */ }
        }
    }
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

fn has_project_marker(p: &std::path::Path) -> bool {
    [".git", "package.json", "Cargo.toml", "pyproject.toml", "go.mod", ".kodenignore"]
        .iter()
        .any(|m| p.join(m).exists())
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

fn index_project(index: &SqliteIndex, proj: &Project) {
    let root = std::path::Path::new(&proj.root);
    let files = walk::walk_files(root);
    let mut indexed = 0usize;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in files {
        let rel = rel_path(root, &path);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        // Binary sniff — skip files with a NUL in the first window.
        if bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
            continue;
        }
        // Freshness hash is over the RAW bytes (any change reindexes).
        let file_hash = hash::hash_bytes(&bytes);
        let content = String::from_utf8_lossy(&bytes);
        // Secrets gate: redact secret-shaped content before it is tokenized/stored.
        let (redacted, nredact) = secrets::redact(&content);
        if nredact > 0 {
            log::debug!("brain: redacted {nredact} secret-shaped span(s) in {rel}");
        }
        if let Err(e) = index.index_file(&proj.id, &rel, &redacted, &file_hash, bytes.len() as i64) {
            log::debug!("brain: index_file failed for {rel}: {e}");
            continue;
        }
        seen.insert(rel);
        indexed += 1;
    }
    // Reconcile deletions: prune index rows for files no longer present on disk
    // (CONCEPT Flow B delta; EXECUTION_PLAN §3 SearchIndex::remove). Without this,
    // a deleted/moved file would match searches forever.
    let mut pruned = 0usize;
    if let Ok(existing) = index.existing_paths(&proj.id) {
        for rel in existing {
            if !seen.contains(&rel) && index.remove_file(&proj.id, &rel).unwrap_or(false) {
                pruned += 1;
            }
        }
    }
    log::info!("brain: project '{}' indexed {indexed}, pruned {pruned}", proj.name);
}

fn rel_path(root: &std::path::Path, path: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Update the live per-pane session map from agent lifecycle signals. Resolves
/// pty → cwd (B1/B3 accessor) → project (registry longest-prefix). Consumed by
/// P3 gist synthesis.
fn handle_agent(app: &AppHandle, pty_id: u32, kind: &str, agent: Option<String>) {
    let Some(brain) = app.try_state::<BrainState>() else {
        return;
    };
    match kind {
        "started" => {
            let project = app
                .try_state::<PtyState>()
                .and_then(|pty| pty.session_cwd(pty_id))
                .and_then(|cwd| brain.registry.resolve(&cwd))
                .map(|p| p.id);
            if let Ok(mut sessions) = brain.sessions.write() {
                sessions.insert(pty_id, LiveSession { project, agent });
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
