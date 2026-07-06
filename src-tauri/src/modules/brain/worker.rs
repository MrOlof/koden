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
/// Autonomous Librarian cadence — EVENT-DRIVEN, not clock-driven. A dirty project
/// triggers a round when it either (a) goes QUIET for `LIBRARIAN_IDLE_SETTLE_MS`
/// (she works in the gaps, never interrupting active edits) or (b) an AI session
/// just EXITED in it (tidy right after the assistant wraps). `LIBRARIAN_MIN_GAP_MS`
/// caps how close two rounds can land (anti-hammer). Deliberately NO fixed change
/// count and NO fixed clock — the digest-hash gate inside [reflect::reflect_auto] is
/// the real "is there anything new" signal and keeps an unchanged round at $0; these
/// only pick the *moment*. ponytail: fixed intervals; lift to user settings to tune.
pub const LIBRARIAN_IDLE_SETTLE_MS: i64 = 3 * 60 * 1000;
pub const LIBRARIAN_MIN_GAP_MS: i64 = 5 * 60 * 1000;
/// Paid-retry policy (ADR-010 cluster 5): after this many CONSECUTIVE failed paid
/// rounds the project stops re-arming itself — only a NEW content change (an Fs
/// event setting `dirty`) buys another attempt.
pub const LIBRARIAN_MAX_CONSEC_FAILURES: u32 = 3;
/// Backoff clamp: the anti-hammer gap doubles per consecutive failure, capped at
/// `MIN_GAP << 4` (5 → 10 → 20 → 40 → 80 min).
const LIBRARIAN_BACKOFF_MAX_SHIFT: u32 = 4;
/// Binary sniff window — a NUL byte in the first 8 KiB means "not text".
const BINARY_SNIFF_BYTES: usize = 8192;

/// Spawn the worker. Mirrors `usage::poll::spawn_poller`. `launch_dir` is the
/// authorized launch directory (the dir the user opened); the brain seeds its
/// P0 project from it rather than blindly indexing the process cwd.
pub fn spawn_brain_worker(app: AppHandle, launch_dir: Option<String>) {
    std::thread::Builder::new()
        .name("koden-brain-worker".into())
        .spawn(move || {
            // Panic observability (ADR-010 cluster 6): if the loop unwinds, the
            // guard flips status to Degraded so the UI sees a dead brain instead
            // of a permanently-"Ready" zombie. Defused on a clean return, which
            // keeps whatever status the loop last set (incl. early Degraded exits).
            let app_panic = app.clone();
            let guard = PanicStatusGuard::arm(move || {
                log::error!("brain: worker thread panicked; status -> degraded");
                set_status(
                    &app_panic,
                    BrainStatus::Degraded { reason: "worker thread panicked".into() },
                );
            });
            brain_loop(app, launch_dir);
            guard.defuse();
        })
        .expect("spawn koden-brain worker thread");
}

/// Drop-guard armed at worker start: runs `on_panic` when dropped by an unwind,
/// does nothing after `defuse()` (the clean-shutdown path).
struct PanicStatusGuard<F: FnOnce()> {
    on_panic: Option<F>,
}

impl<F: FnOnce()> PanicStatusGuard<F> {
    fn arm(on_panic: F) -> Self {
        Self { on_panic: Some(on_panic) }
    }
    fn defuse(mut self) {
        self.on_panic = None; // the ensuing Drop sees None and no-ops
    }
}

impl<F: FnOnce()> Drop for PanicStatusGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.on_panic.take() {
            f();
        }
    }
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
    // `open_with_recovery` (ADR-006 rebuildable cache): transient BUSY at boot is
    // retried briefly; a CORRUPT/NOTADB cache is moved aside + rebuilt fresh (with
    // best-effort canonical salvage) instead of bricking every launch until the
    // user finds and deletes an app-data file. Only genuine failures degrade.
    let index = match SqliteIndex::open_with_recovery(&db_path) {
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

    // 6. Arm the recursive watcher over each seeded project root (P1 freshness)
    // BEFORE the warm walk: an edit made while the initial index runs fires an
    // event that buffers in the worker channel and replays through the normal
    // delta path once the loop starts (the blake3 hash-skip makes replay
    // idempotent). Armed after, such edits would be missed forever — no event,
    // hash already recorded. Held for the worker's lifetime — dropping it stops
    // watching. Re-armed on a full Rescan so newly-registered projects get
    // watched too.
    let mut watcher = arm_watcher(&app, &tx);

    // 6b. Warm population — project by project so the first is searchable early.
    warm_population(&app, &index);
    // Seed the review inbox with structural doctor findings (no date check yet).
    run_doctor_all(&app, &index, None);
    set_status(&app, BrainStatus::Ready);

    // Per-project autonomous-reflect bookkeeping (worker-thread-local; resets on
    // restart, which is fine — dirty flags only matter while the app is open).
    let mut lib_state: std::collections::HashMap<String, LibrarianAuto> = std::collections::HashMap::new();

    // 7. Steady-state event loop. Single writer; ingest paths only send events.
    for ev in rx {
        match ev {
            BrainEvent::Agent { pty_id, kind, agent } => {
                let project = handle_agent(&app, pty_id, &kind, agent);
                // An AI session exiting is a natural "settle now" boundary: if that
                // project already has pending changes, let the Librarian tidy right
                // after, without waiting out the idle-settle.
                if kind == "exited" {
                    if let Some(st) = project.and_then(|p| lib_state.get_mut(&p)) {
                        st.boundary = true;
                    }
                }
            }
            BrainEvent::Rescan { project } => match project {
                // Targeted reconcile — a full blake3 sweep of ONE project (the
                // watcher's missed-event recovery, or a single-project
                // brain_rescan). The watched root set is unchanged, so the
                // watcher is left alone.
                Some(pid) => {
                    let proj = app
                        .try_state::<BrainState>()
                        .and_then(|s| s.registry.projects().into_iter().find(|p| p.id == pid));
                    match proj {
                        Some(p) => index_project(&index, &p),
                        None => log::debug!("brain: rescan for unknown project '{pid}' ignored"),
                    }
                }
                None => {
                    // Arm the NEW watcher (covering any newly-registered roots)
                    // BEFORE the walk and before retiring the old one: a brief
                    // double-watch only duplicates events (idempotent via the
                    // hash-skip), whereas walk-then-arm misses edits made during
                    // the walk.
                    let old = watcher.take();
                    watcher = arm_watcher(&app, &tx);
                    drop(old);
                    warm_population(&app, &index); // full reconcile (add/change/delete)
                    if let Some(s) = app.try_state::<BrainState>() {
                        s.registry.save_to(&cfg_path); // persist the project list
                    }
                }
            },
            BrainEvent::RemoveProject { project } => {
                if let Err(e) = index.remove_project(&project) {
                    log::warn!("brain: remove_project '{project}' prune failed ({e})");
                } else {
                    log::info!("brain: removed project '{project}' (unregistered + pruned)");
                }
                // Arm-then-drop (same rationale as the full Rescan): the removed
                // root stops being watched when the OLD watcher retires; its
                // in-flight events resolve to an unregistered project and no-op.
                let old = watcher.take();
                watcher = arm_watcher(&app, &tx);
                drop(old);
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
                    let (outcome, digest_hash) =
                        reflect::reflect_once(&app, &index, &pid, now_date.as_deref(), now_ms);
                    // Feed the autonomous delta gate: the digest just (paid-)reflected
                    // on must not be re-paid by the next auto round. Ok = success
                    // (resets the failure streak); InvalidOutput = paid but rejected —
                    // identical bytes would fail identically, so pin its hash too.
                    if digest_hash.is_some() {
                        let st = lib_state.entry(pid.clone()).or_default();
                        match outcome.reason {
                            reflect::ReflectReason::Ok => {
                                st.digest_hash = digest_hash;
                                st.fail_streak = 0;
                            }
                            reflect::ReflectReason::InvalidOutput => st.digest_hash = digest_hash,
                            _ => {} // CallFailed: retrying the same digest is legitimate
                        }
                    }
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
                        // Record the change (dirty + when). The Librarian settles in
                        // after a quiet spell, deciding via the digest hash whether
                        // there's actually anything new to reflect on.
                        note_content_change(lib_state.entry(project.clone()).or_default(), now_epoch_ms());
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

/// Root sanity gate, shared by the boot seed and `brain_add_project`: never index
/// a filesystem/drive root or the bare home dir (the wizard handles intentional
/// home workspaces). Judges the CANONICAL path — the same form `add_root` will
/// register — so a non-canonical spelling (`c:\users\me`, `C:\x\..\..`) cannot
/// slip past the gate and index a home dir or whole drive (ADR-010 cluster 7).
pub fn is_sane_root(p: &std::path::Path) -> bool {
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canon.parent().is_none() {
        return false; // filesystem root
    }
    // Compare home in the same canonical string form (`\\?\`-stripped, case-folded
    // on Windows) — raw `Path` equality is byte/component-sensitive, so a
    // differently-cased spelling would sail past it.
    let canon_s = crate::modules::brain::registry::fold_case(&crate::modules::fs::to_canon(&canon));
    let home_s = dirs::home_dir().map(|h| {
        let h = std::fs::canonicalize(&h).unwrap_or(h);
        crate::modules::brain::registry::fold_case(&crate::modules::fs::to_canon(h))
    });
    if home_s.as_deref() == Some(canon_s.as_str()) {
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

/// Outcome of indexing one file. Reconcile-delete (ADR-010) must distinguish
/// "positively absent" from "unknown": only NotFound is evidence of deletion —
/// a read/store error means the file's state is UNKNOWN and any last-good index
/// row must be kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileOutcome {
    /// Indexed (real change) or an unchanged no-op — present and in the index.
    Indexed,
    /// Present on disk but deliberately not indexable (binary, over the size
    /// cap) — a stale index row SHOULD be pruned (matches a full rebuild).
    NotIndexable,
    /// Positive evidence the file is gone (NotFound at open time).
    Absent,
    /// Read or store error — state UNKNOWN; never treat as absence.
    Unknown,
}

/// Read → binary-sniff → blake3 → secrets-redact → index one file. Shared by the
/// full walk (`index_dir`) and the incremental watcher (`index_changed`). On a
/// REAL content change (index_file → Ok(true)) it stamps the temporal recency via
/// `record_access(now_ms)`; an unchanged no-op (Ok(false)) does NOT re-stamp — so
/// a warm pass over an unchanged index leaves accessed_at_ms fixed, preserving the
/// gist byte-identity gate ([DP-12]).
fn index_one_file(
    index: &SqliteIndex,
    project_id: &str,
    rel: &str,
    path: &std::path::Path,
    now_ms: i64,
) -> FileOutcome {
    // Bounded read (ADR-010 TOCTOU): the walker's stat-time size check can be
    // minutes stale, so re-enforce the cap at read time with a take()-bounded
    // reader — a file that grew past the cap can never balloon memory.
    use std::io::Read as _;
    let mut bytes: Vec<u8> = Vec::new();
    match std::fs::File::open(path) {
        Ok(f) => {
            if let Err(e) = f.take(walk::MAX_INDEX_FILE_BYTES + 1).read_to_end(&mut bytes) {
                log::debug!("brain: read failed for {rel}: {e}");
                return FileOutcome::Unknown;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FileOutcome::Absent,
        Err(e) => {
            log::debug!("brain: open failed for {rel}: {e}");
            return FileOutcome::Unknown;
        }
    }
    if bytes.len() as u64 > walk::MAX_INDEX_FILE_BYTES {
        return FileOutcome::NotIndexable; // grew past the cap since the stat
    }
    // Binary sniff — skip files with a NUL in the first window.
    if bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
        return FileOutcome::NotIndexable;
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
            FileOutcome::Indexed
        }
        Ok(false) => FileOutcome::Indexed, // unchanged no-op — present, recency unchanged
        Err(e) => {
            log::debug!("brain: index_file failed for {rel}: {e}");
            FileOutcome::Unknown
        }
    }
}

/// The per-project indexing pipeline: walk → (per file) index → reconcile-delete.
/// Deliberately free of `AppHandle`/registry so the deterministic offline sandbox
/// and integration tests drive the **real** pipeline (BUILD-PROMPT §6.5). `root`
/// is the absolute project root; `project_id` the id the rows are keyed under.
pub fn index_dir(index: &SqliteIndex, project_id: &str, root: &std::path::Path) -> IndexStats {
    // ADR-010: an unreadable/absent root is UNKNOWN, not "everything deleted" —
    // an unmounted drive or a permission blip must never wipe the last-good index
    // (temporal state + pending paid proposals are not rebuildable). Skip the pass.
    if let Err(e) = std::fs::read_dir(root) {
        log::warn!(
            "brain: project root {} unreadable ({e}); keeping last-good index",
            root.display()
        );
        return IndexStats::default();
    }
    index_walked(index, project_id, root, walk::walk_files(root))
}

/// Pipeline body, parametrized over the walk outcome so tests can drive the
/// reconcile gate directly (a real >MAX_SCANNED repo is too heavy for CI).
fn index_walked(
    index: &SqliteIndex,
    project_id: &str,
    root: &std::path::Path,
    walked: walk::Walked,
) -> IndexStats {
    let now_ms = now_epoch_ms(); // one recency stamp for everything changed in this pass
    let mut indexed = 0usize;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in walked.files {
        let rel = rel_path(root, &path);
        match index_one_file(index, project_id, &rel, &path, now_ms) {
            FileOutcome::Indexed => {
                seen.insert(rel);
                indexed += 1;
            }
            // Unknown (read/store error): the file may well exist — keep any
            // last-good row out of the deletion set (ADR-010 positive evidence).
            FileOutcome::Unknown => {
                seen.insert(rel);
            }
            // NotIndexable: present but excluded (binary/oversize) — a stale row
            // is pruned, matching a full rebuild. Absent: positively gone.
            FileOutcome::NotIndexable | FileOutcome::Absent => {}
        }
    }
    // Reconcile deletions: prune index rows for files no longer present on disk
    // (CONCEPT Flow B delta; EXECUTION_PLAN §3 SearchIndex::remove). Without this,
    // a deleted/moved file would match searches forever. ONLY when the walk was
    // complete — a truncated/errored walk is a partial view, not evidence of
    // absence (ADR-010: files past the cap would otherwise oscillate every pass).
    let mut pruned = 0usize;
    if walked.complete {
        if let Ok(existing) = index.existing_paths(project_id) {
            for rel in existing {
                if !seen.contains(&rel) && index.remove_file(project_id, &rel).unwrap_or(false) {
                    pruned += 1;
                }
            }
        }
    } else {
        // ponytail: a partial walk skips reconcile-delete for the WHOLE project, so a
        // permanently-capped (>MAX_SCANNED) repo only prunes via watcher events; upgrade
        // path = track which subtrees were fully walked and reconcile inside those only.
        log::warn!(
            "brain: walk of {} was partial; additions/updates only, reconcile-delete skipped",
            root.display()
        );
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
        let rel = rel_path(root, path);
        if rel.is_empty() {
            continue;
        }
        // Skip-dir gate on the PROJECT-RELATIVE path: an absolute-path check
        // would zero out incremental updates for a project that itself lives
        // under a dir named e.g. `build/` or `vendor/` (ADR-010 cluster 2).
        if walk::rel_under_skip_dir(&rel) || secrets::is_denylisted_path(&to_canon(path)) {
            continue;
        }
        // Ignore-file gate: the full walk never yields a .gitignore'd/.kodenignore'd
        // file, so the watcher must not index one either (it would oscillate —
        // indexed here, pruned by the next full pass). Deleted paths stat as
        // non-dirs inside the check and fall through to the prune branch below.
        if walk::is_ignored_file(root, path) {
            continue;
        }
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => {
                if m.len() > walk::MAX_INDEX_FILE_BYTES {
                    continue;
                }
                if index_one_file(index, project_id, &rel, path, now_ms) == FileOutcome::Indexed {
                    indexed += 1;
                }
            }
            Ok(_) => {
                // A directory event (e.g. an atomic move-in of an existing tree)
                // whose children weren't individually reported — index them so the
                // incremental graph converges with a full rebuild. Additions only,
                // so a partial child walk is harmless here. `walk_files_under`
                // replays in-project ancestor ignore files so this subtree walk
                // agrees with the full walk.
                for child in walk::walk_files_under(root, path).files {
                    // Per-child ignore gate: the subtree walk replays ancestors at
                    // the walker's LOWEST precedence, so a deeper per-dir whitelist
                    // can over-yield a file the full walk ignores (root .kodenignore
                    // `x` + sub/.gitignore `!x`). The gate has the exact cross-source
                    // precedence, so an over-yielded child never enters the index.
                    if walk::is_ignored_file(root, &child) {
                        continue;
                    }
                    let crel = rel_path(root, &child);
                    if !crel.is_empty()
                        && index_one_file(index, project_id, &crel, &child, now_ms)
                            == FileOutcome::Indexed
                    {
                        indexed += 1;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Positively gone (deleted / moved away) — prune the stale row + FTS doc.
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
            Err(e) => {
                // Unreadable stat (AV/editor lock, permission blip) — state UNKNOWN,
                // not absence (ADR-010): keep the last-good row; a later event or
                // full pass re-syncs it.
                log::debug!("brain: stat failed for {rel} ({e}); keeping last-good row");
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

/// Per-project autonomous-reflect bookkeeping (worker-thread-local; resets on
/// restart, fine — only matters while the app is open). `pub` (like [index_dir])
/// so the `tests/` integration driver can exercise the REAL retry policy through
/// [librarian_round_step] — not part of any stable surface.
#[derive(Default)]
pub struct LibrarianAuto {
    /// Something was indexed/pruned here since the last round.
    pub dirty: bool,
    /// An AI session exited here since the last round — a "settle now" boundary.
    pub boundary: bool,
    /// Epoch ms of the last indexed change (drives the idle-settle wait).
    pub last_change_ms: i64,
    /// Epoch ms of the last round (drives the anti-hammer min-gap).
    pub last_pass_ms: i64,
    /// Hash of the last digest we reflected on (drives the delta gate).
    pub digest_hash: Option<String>,
    /// CONSECUTIVE failed PAID rounds (CallFailed/InvalidOutput) since the last
    /// success — drives the backoff + the stop-re-arming cap (ADR-010 cluster 5).
    pub fail_streak: u32,
}

/// Fold one indexed content change into the state — the Fs-handler half of the
/// retry policy: re-arm the round and restart the idle-settle clock. The fail
/// streak is deliberately NOT reset here — only a successful round clears it —
/// so a still-failing provider keeps its widened backoff gap even as edits land.
pub fn note_content_change(st: &mut LibrarianAuto, now_ms: i64) {
    st.dirty = true;
    st.last_change_ms = now_ms;
}

/// Pure round predicate (testable): a dirty project, past the anti-hammer min-gap,
/// that has EITHER settled into idle OR just had an AI session exit. No change count,
/// no fixed clock — the digest hash inside [reflect::reflect_auto] is the real
/// "anything new" signal; this only picks the *moment*. Consecutive failed paid
/// rounds widen the gap exponentially (backoff) so a failing provider can't be
/// re-charged every min-gap.
fn due_for_round(
    dirty: bool,
    boundary: bool,
    last_change_ms: i64,
    last_pass_ms: i64,
    now_ms: i64,
    fail_streak: u32,
) -> bool {
    let gap = LIBRARIAN_MIN_GAP_MS << fail_streak.min(LIBRARIAN_BACKOFF_MAX_SHIFT);
    if !dirty || now_ms.saturating_sub(last_pass_ms) < gap {
        return false;
    }
    boundary || now_ms.saturating_sub(last_change_ms) >= LIBRARIAN_IDLE_SETTLE_MS
}

/// Fold one round's outcome into the per-project state — the paid-retry policy
/// (ADR-010 cluster 5). Pure + testable:
///  - Ok/Unchanged: remember the digest hash, reset the failure streak.
///  - Disabled/NoKey/OverBudget/EmptyCorpus: $0 pre-flight skips (nothing reserved
///    or charged) — re-arm so the round retries once the gate clears.
///  - InvalidOutput: PAID (the provider answered; the JSON failed validation).
///    Pin the digest hash anyway — byte-identical input would fail identically, so
///    the SAME digest is never re-paid; only NEW content (a fresh Fs event → new
///    digest) buys another attempt. Counts toward the streak.
///  - CallFailed: charged on uncertainty (a 4xx was charged $0 upstream). Retry
///    with exponential backoff; past LIBRARIAN_MAX_CONSEC_FAILURES stop re-arming —
///    a NEW content change re-arms via the Fs handler.
fn apply_round_outcome(
    st: &mut LibrarianAuto,
    reason: &reflect::ReflectReason,
    digest_hash: Option<String>,
) {
    use reflect::ReflectReason as R;
    match reason {
        R::Ok | R::Unchanged => {
            st.digest_hash = digest_hash;
            st.fail_streak = 0;
        }
        R::Disabled | R::NoKey | R::OverBudget | R::EmptyCorpus => {
            st.dirty = true;
        }
        R::InvalidOutput => {
            st.digest_hash = digest_hash;
            st.fail_streak = st.fail_streak.saturating_add(1);
        }
        R::CallFailed(_) => {
            st.fail_streak = st.fail_streak.saturating_add(1);
            if st.fail_streak < LIBRARIAN_MAX_CONSEC_FAILURES {
                st.dirty = true;
            }
        }
    }
}

/// One project's full round step — the production sequencing (ADR-010 cluster 5):
/// gate via [due_for_round], consume the dirty/boundary flags, stamp `last_pass_ms`
/// (the next round is gated regardless of outcome), run exactly ONE reflect attempt
/// via `run` (handed the previous digest hash for the delta gate), and fold the
/// outcome back via [apply_round_outcome]. Returns `None` when no attempt fired
/// this tick (not due, or parked past the failure cap). Extracted from
/// [run_librarian_rounds] — which MUST keep delegating here — so the `tests/`
/// integration driver exercises the identical decision path without an AppHandle.
pub fn librarian_round_step<F>(
    st: &mut LibrarianAuto,
    now_ms: i64,
    run: F,
) -> Option<reflect::ReflectOutcome>
where
    F: FnOnce(Option<&str>) -> (reflect::ReflectOutcome, Option<String>),
{
    if !due_for_round(st.dirty, st.boundary, st.last_change_ms, st.last_pass_ms, now_ms, st.fail_streak) {
        return None;
    }
    st.dirty = false;
    st.boundary = false;
    st.last_pass_ms = now_ms; // gate the next round regardless of outcome
    let prev = st.digest_hash.clone();
    let (outcome, digest_hash) = run(prev.as_deref());
    apply_round_outcome(st, &outcome.reason, digest_hash);
    Some(outcome)
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
    // Real current date (UTC) so date-dependent findings (stale_revalidate) are
    // visible to autonomous rounds, not only to manual clicks (ADR-010 cluster 5).
    let today = utc_date_ymd(now_ms);
    for (project_id, st) in state.iter_mut() {
        let Some(outcome) = librarian_round_step(st, now_ms, |prev| {
            reflect::reflect_auto(app, index, project_id, Some(&today), now_ms, prev)
        }) else {
            continue;
        };
        match &outcome.reason {
            reflect::ReflectReason::Ok | reflect::ReflectReason::Unchanged => {
                log::info!(
                    "brain: auto-reflect '{project_id}' → {:?} ({} proposal(s), ${:.4})",
                    outcome.reason,
                    outcome.proposals.len(),
                    outcome.spent_usd
                );
            }
            // PAID failures — surface them (real money), with the retry stance.
            reflect::ReflectReason::CallFailed(_) | reflect::ReflectReason::InvalidOutput => {
                log::warn!(
                    "brain: auto-reflect '{project_id}' paid round failed ({:?}, ${:.4}); {} consecutive failure(s), {}",
                    outcome.reason,
                    outcome.spent_usd,
                    st.fail_streak,
                    if st.dirty { "retrying with backoff" } else { "parked until new content changes" }
                );
            }
            // $0 pre-flight skips (re-armed; recover once a budget/key is set).
            other => log::debug!("brain: auto-reflect '{project_id}' skipped: {other:?}"),
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

/// UTC calendar date `YYYY-MM-DD` for an epoch-ms instant — no new dependency
/// (civil-from-days, Howard Hinnant's algorithm). UTC rather than local is fine
/// for note-staleness horizons measured in days; the doctor compares lexically.
fn utc_date_ymd(epoch_ms: i64) -> String {
    let days = epoch_ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
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
/// Returns the resolved project id (if any) so the caller can mark a Librarian
/// boundary on an `exited` signal.
fn handle_agent(app: &AppHandle, pty_id: u32, kind: &str, agent: Option<String>) -> Option<String> {
    let brain = app.try_state::<BrainState>()?;
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
    let resolved = project.clone(); // returned to the caller (the `started` arm moves `project`)

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
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_for_round_fires_on_idle_or_boundary_past_min_gap() {
        let gap = LIBRARIAN_MIN_GAP_MS;
        let settle = LIBRARIAN_IDLE_SETTLE_MS;
        let now = 100 * gap; // comfortably past any min-gap from last_pass = 0
        // Not dirty → never, even with a boundary.
        assert!(!due_for_round(false, true, now, 0, now, 0));
        // Dirty but still inside the min-gap since the last round → hold.
        assert!(!due_for_round(true, true, now, now - 1, now, 0));
        // Dirty, past min-gap, but still actively changing (not settled, no boundary) → hold.
        assert!(!due_for_round(true, false, now, 0, now, 0));
        // Dirty, past min-gap, settled into idle → fire.
        assert!(due_for_round(true, false, now - settle, 0, now, 0));
        // Dirty, past min-gap, an AI session just exited → fire even though not idle.
        assert!(due_for_round(true, true, now, 0, now, 0));
    }

    /// ADR-010 cluster 5: consecutive paid failures widen the anti-hammer gap
    /// exponentially, clamped so it can never overflow or grow unbounded.
    #[test]
    fn due_for_round_backs_off_after_failures() {
        let gap = LIBRARIAN_MIN_GAP_MS;
        // Boundary + dirty, one failure: min-gap alone no longer fires…
        assert!(!due_for_round(true, true, 0, 0, gap, 1));
        // …the doubled gap does.
        assert!(due_for_round(true, true, 0, 0, 2 * gap, 1));
        // The shift clamps: a huge streak behaves like the max shift (no overflow).
        let max_gap = gap << LIBRARIAN_BACKOFF_MAX_SHIFT;
        assert!(!due_for_round(true, true, 0, 0, max_gap - 1, u32::MAX));
        assert!(due_for_round(true, true, 0, 0, max_gap, u32::MAX));
    }

    /// ADR-010 cluster 5: the paid-retry policy — success resets, free skips
    /// re-arm, InvalidOutput pins the digest hash (identical bytes never re-paid),
    /// CallFailed retries with a cap then parks until new content re-arms.
    #[test]
    fn apply_round_outcome_paid_retry_policy() {
        use reflect::ReflectReason as R;
        let mut st = LibrarianAuto::default();

        // $0 pre-flight skip: re-armed, nothing counted.
        apply_round_outcome(&mut st, &R::Disabled, None);
        assert!(st.dirty && st.fail_streak == 0 && st.digest_hash.is_none());

        // InvalidOutput: hash pinned (the SAME digest must never be re-paid),
        // streak counted, NOT re-armed — new content is the only retry trigger.
        st.dirty = false;
        apply_round_outcome(&mut st, &R::InvalidOutput, Some("h1".into()));
        assert!(!st.dirty, "InvalidOutput must not re-arm");
        assert_eq!(st.digest_hash.as_deref(), Some("h1"));
        assert_eq!(st.fail_streak, 1);

        // CallFailed: retries (re-arms) below the cap…
        for expected in 2..LIBRARIAN_MAX_CONSEC_FAILURES {
            st.dirty = false;
            apply_round_outcome(&mut st, &R::CallFailed("x".into()), None);
            assert_eq!(st.fail_streak, expected);
            assert!(st.dirty, "below the cap → retry");
        }
        // …and parks at the cap.
        st.dirty = false;
        apply_round_outcome(&mut st, &R::CallFailed("x".into()), None);
        assert_eq!(st.fail_streak, LIBRARIAN_MAX_CONSEC_FAILURES);
        assert!(!st.dirty, "at the cap → stop re-arming until new content");

        // Success: streak reset, hash updated.
        apply_round_outcome(&mut st, &R::Ok, Some("h2".into()));
        assert_eq!(st.fail_streak, 0);
        assert_eq!(st.digest_hash.as_deref(), Some("h2"));
    }

    /// ADR-010 cluster 6: a worker panic must flip status to Degraded (via the
    /// armed guard's Drop during unwind); a clean, defused return must not.
    #[test]
    fn panic_guard_fires_on_unwind_not_on_clean_return() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        // Clean shutdown: defused guard never fires.
        let fired = Arc::new(AtomicBool::new(false));
        let f = fired.clone();
        let guard = PanicStatusGuard::arm(move || f.store(true, Ordering::SeqCst));
        guard.defuse();
        assert!(!fired.load(Ordering::SeqCst), "defused guard must not fire");

        // Panic path: the unwind drops the armed guard → the hook fires.
        let f = fired.clone();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = PanicStatusGuard::arm(move || f.store(true, Ordering::SeqCst));
            panic!("simulated worker death");
        }));
        assert!(unwound.is_err());
        assert!(fired.load(Ordering::SeqCst), "unwound guard must flip status");
    }

    /// ADR-010 cluster 7: the gate now also fronts `brain_add_project` — a drive
    /// root or the bare home dir must be rejected; a normal folder passes.
    #[test]
    fn is_sane_root_rejects_fs_root_and_home() {
        let fs_root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(!is_sane_root(std::path::Path::new(fs_root)));
        if let Some(home) = dirs::home_dir() {
            assert!(!is_sane_root(&home));
            // Non-canonical spellings must not slip past: the gate judges the
            // CANONICAL path `add_root` would register (ADR-010 cluster 7 repair).
            assert!(!is_sane_root(&home.join(".")), "home/. must be rejected");
            #[cfg(windows)]
            {
                let lower = std::path::PathBuf::from(home.to_string_lossy().to_lowercase());
                assert!(!is_sane_root(&lower), "case-different home spelling must be rejected");
            }
            // Enough `..` to escape past the fs root (extra `..` at root is a no-op):
            // canonicalizes to the filesystem root, which must be rejected.
            let mut escape = home.clone();
            for _ in home.components() {
                escape.push("..");
            }
            assert!(!is_sane_root(&escape), "..-escape to the fs root must be rejected");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(is_sane_root(dir.path()));
    }

    #[test]
    fn utc_date_ymd_known_dates() {
        assert_eq!(utc_date_ymd(0), "1970-01-01");
        assert_eq!(utc_date_ymd(86_400_000 - 1), "1970-01-01", "just before midnight");
        assert_eq!(utc_date_ymd(86_400_000), "1970-01-02");
        assert_eq!(utc_date_ymd(1_704_067_200_000), "2024-01-01");
        assert_eq!(utc_date_ymd(1_709_164_800_000), "2024-02-29", "leap day");
    }

    /// ADR-010 cluster 2: the incremental skip-dir gate must be PROJECT-RELATIVE.
    /// A project that itself lives under a dir named `build` still gets its
    /// incremental updates; a `dist/` INSIDE the project stays skipped.
    #[test]
    fn index_changed_skip_dirs_are_project_relative() {
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("build").join("proj"); // root UNDER a skip-named dir
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(root.join("src").join("main.ts"), "alpha").unwrap();
        std::fs::write(root.join("dist").join("bundle.js"), "bravo").unwrap();

        let changed = vec![root.join("src").join("main.ts"), root.join("dist").join("bundle.js")];
        let stats = index_changed(&index, "p", &root, &changed);
        assert_eq!(stats.indexed, 1, "src file indexed despite 'build' in the ABSOLUTE path");
        let paths = index.existing_paths("p").unwrap();
        assert!(paths.contains(&"src/main.ts".to_string()));
        assert!(!paths.iter().any(|p| p.starts_with("dist/")), "in-project dist stays skipped");
    }

    /// The incremental watcher path must agree with the FULL walk on ignore
    /// rules in a NON-git root (CONCEPT §7.1 uniformity): per-file events for
    /// .gitignore'd files are not indexed, and a dir event's subtree walk
    /// honors in-project ancestor ignore files — same final path set as a full
    /// pass, so nothing oscillates between watcher-index and full-pass prune.
    #[test]
    fn index_changed_honors_gitignore_like_the_full_walk() {
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        let dir = tempfile::tempdir().unwrap(); // NON-git project root
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.zzgen\n").unwrap();
        std::fs::write(root.join("kept.txt"), "alpha").unwrap();
        std::fs::write(root.join("skip.zzgen"), "bravo").unwrap();
        let moved = root.join("moved"); // arrives later as one dir event
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::write(moved.join("in.txt"), "charlie").unwrap();
        std::fs::write(moved.join("out.zzgen"), "delta").unwrap();

        // Incremental: per-file events + one dir event.
        index_changed(&index, "inc", root, &[
            root.join("kept.txt"),
            root.join("skip.zzgen"),
            root.join(".gitignore"),
            moved.clone(),
        ]);
        let mut inc = index.existing_paths("inc").unwrap();
        inc.sort();
        assert!(inc.contains(&"kept.txt".to_string()));
        assert!(!inc.contains(&"skip.zzgen".to_string()), "gitignored file event must not index");
        assert!(inc.contains(&"moved/in.txt".to_string()), "dir event indexes non-ignored children");
        assert!(!inc.contains(&"moved/out.zzgen".to_string()), "dir event honors ancestor .gitignore");

        // Full walk over the same disk state lands on the SAME path set.
        index_dir(&index, "full", root);
        let mut full = index.existing_paths("full").unwrap();
        full.sort();
        assert_eq!(inc, full, "incremental path must agree with the full walk");
    }

    /// Residual of the subtree-walk ancestor replay: `add_ignore` ranks a
    /// replayed root .kodenignore BELOW a per-dir .gitignore, so a deeper
    /// `!negation` can make `walk_files_under` over-yield a .kodenignore'd
    /// file. The dir-event branch's per-child gate must catch it — the file
    /// must never enter the index, not even transiently until the next full pass.
    #[test]
    fn dir_event_never_indexes_kodenignored_child_despite_deeper_negation() {
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        let dir = tempfile::tempdir().unwrap(); // NON-git project root
        let root = dir.path();
        std::fs::write(root.join(".kodenignore"), "zz.txt\n").unwrap();
        let sub = root.join("sub"); // arrives as ONE dir event (atomic move-in)
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(".gitignore"), "!zz.txt\n").unwrap();
        std::fs::write(sub.join("zz.txt"), "alpha").unwrap();
        std::fs::write(sub.join("kept.txt"), "bravo").unwrap();

        index_changed(&index, "p", root, &[sub.clone()]);
        let paths = index.existing_paths("p").unwrap();
        assert!(paths.contains(&"sub/kept.txt".to_string()), "non-ignored child is indexed");
        assert!(
            !paths.contains(&"sub/zz.txt".to_string()),
            ".kodenignore'd child must not enter the index even when the subtree walk over-yields it"
        );
    }

    /// ADR-010: a PARTIAL walk (scan cap hit / unreadable subtree) must never feed
    /// reconcile-delete; the same disk state with a COMPLETE walk does prune.
    #[test]
    fn partial_walk_never_feeds_reconcile_delete() {
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        index.index_file("p", "a.ts", "alpha", "h1", 5).unwrap();
        index.index_file("p", "b.ts", "bravo", "h2", 5).unwrap();
        let root = tempfile::tempdir().unwrap(); // nothing on disk

        let stats = index_walked(&index, "p", root.path(), walk::Walked {
            files: Vec::new(),
            complete: false,
        });
        assert_eq!(stats.pruned, 0, "partial walk must not prune");
        assert_eq!(index.existing_paths("p").unwrap().len(), 2, "last-good rows kept");

        let stats = index_walked(&index, "p", root.path(), walk::Walked {
            files: Vec::new(),
            complete: true,
        });
        assert_eq!(stats.pruned, 2, "complete walk over empty disk prunes");
    }
}
