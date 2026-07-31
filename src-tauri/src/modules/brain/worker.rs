//! The single GUI-resident Brain worker thread — a clone of the usage poller
//! template (`usage/poll.rs:384`). Spawned from `lib.rs .setup()` after the usage
//! poller; fail-open; never blocks first paint (spawn returns immediately, all
//! real work runs on this thread). CONCEPT §5.1, EXECUTION_PLAN §2.3.

use std::sync::mpsc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::modules::brain::events::{AgentSignalPayload, BrainEvent};
use crate::modules::brain::freshness::{hash, walk, watch};
use crate::modules::brain::curate;
use crate::modules::brain::gist;
use crate::modules::brain::memory;
use crate::modules::brain::reflect;
use crate::modules::brain::registry::{KodenBrainRegistry, Project};
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

// --- ADR-020 session activity layer -----------------------------------------
/// Coalesced Librarian activity event to the frontend (toast + bell + status
/// bar) — one per apply-sweep batch / reflect round / revert, never per-proposal.
const ACTIVITY_EVENT: &str = "koden:brain-activity";
/// Per-project activity retention: newest rows kept (turnStore MAX_TURNS
/// precedent) + a day horizon (RESUME_TTL_DAYS precedent). Pruned on Tick.
pub const ACTIVITY_MAX_ROWS: usize = 500;
pub const ACTIVITY_TTL_DAYS: i64 = 14;
/// Coarse files-touched rows are debounced per project — the watcher already
/// coalesces bursts, this bounds a long edit session to ~1 row/min.
const FILES_ACTIVITY_DEBOUNCE_MS: i64 = 60_000;
/// Stored prompt text cap (chars) — mirrors the turn-store trim+cap idiom
/// (turnStore.ts caps at 400 for display; the trail keeps more for the digest).
const TURN_MAX_CHARS: usize = 1500;
/// Rel paths kept on one coarse `files` activity row.
const FILES_ACTIVITY_MAX_PATHS: usize = 20;

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

    // 5b. Boot re-discovery (ADR-021): a project cloned/created in the workspace
    // while Koden was closed is registered BEFORE the watcher arms and the warm
    // walk runs, so it gets watched + indexed + doctored + its first artifact
    // emission through the normal boot flow below. Idempotent — the same shared
    // loop `brain_set_workspace` runs; already-registered children no-op and
    // explicitly REMOVED children stay removed (persisted registry tombstones —
    // an auto scan never undoes the user's confirmed remove).
    if let Some(state) = app.try_state::<BrainState>() {
        if let Some(root) = state.registry.workspace_root() {
            let (_, added) =
                register_workspace_children(&state.registry, std::path::Path::new(&root));
            if added > 0 {
                log::info!("brain: boot re-discovery registered {added} new workspace project(s)");
                state.registry.save_to(&cfg_path);
            }
        }
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
    // ADR-019: first emission once every project is ready (byte-compare-gated, so
    // an unchanged relaunch rewrites nothing and the agents' turn context — and
    // with it their prompt-cache behavior — stays byte-identical).
    emit_all_gist_artifacts(&app, &index);
    // Day-boundary tracker for the Tick re-emit: the overdue-note set (and thus
    // the possibly-stale labels) transitions at UTC midnight; purely event-driven
    // re-emission would leave a wrong label until the next memory event.
    let mut last_emit_day = utc_date_ymd(now_epoch_ms());
    set_status(&app, BrainStatus::Ready);

    // Per-project autonomous-reflect bookkeeping (worker-thread-local; resets on
    // restart, which is fine — dirty flags only matter while the app is open).
    let mut lib_state: std::collections::HashMap<String, LibrarianAuto> = std::collections::HashMap::new();
    // ADR-020: per-project debounce stamp for coarse files-touched activity rows
    // (worker-thread-local like lib_state; a restart just re-arms the first row).
    let mut files_activity_last: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    // Last HEAD sha recorded per project, so an unchanged HEAD never re-writes
    // the same commit subject (worker-thread-local; a restart re-records once,
    // which the day-set fold absorbs as a no-op).
    let mut head_commit_last: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // 7. Steady-state event loop. Single writer; ingest paths only send events.
    for ev in rx {
        match ev {
            BrainEvent::Agent { pty_id, kind, agent } => {
                let project = handle_agent(&app, &index, &tx, pty_id, &kind, agent);
                // An AI session exiting is a natural "settle now" boundary: if that
                // project already has pending changes, let the Librarian tidy right
                // after, without waiting out the idle-settle.
                if kind == "exited" {
                    if let Some(p) = &project {
                        if let Some(st) = lib_state.get_mut(p) {
                            st.boundary = true;
                        }
                        enqueue_exit_reconcile(&tx, p);
                    }
                }
            }
            BrainEvent::Turn { pty_id, prompt } => {
                // ADR-020 turn ingest: pre-filter → truncate → REDACT (the ingest
                // gate — prompt text never lands raw), then resolve pty → project
                // and store on this single writer. Unresolvable/filtered turns drop.
                if let Some(cleaned) = clean_turn_text(&prompt) {
                    if let Some(project) = resolve_pty_project(&app, &tx, pty_id) {
                        if let Err(e) = index.record_activity(
                            &project,
                            Some(pty_id as i64),
                            "turn",
                            &cleaned,
                            now_epoch_ms(),
                        ) {
                            log::debug!("brain: turn activity write failed ({e})");
                        }
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
                        Some(p) => {
                            index_project(&index, &p);
                            // ADR-019: the rescan just converged — refresh the artifact.
                            emit_gist_artifact(&app, &index, &p.id);
                        }
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
                    // GUI-validation D1: warm_population only emits Warming{pct}
                    // steps; without this the status sticks at the last pct
                    // ("Indexing… 50%") forever after a mid-session add/rescan —
                    // only the boot path (brain_loop) set Ready.
                    set_status(&app, BrainStatus::Ready);
                    // ADR-019: full reconcile done — refresh every artifact (a
                    // newly-registered project gets its first emission here).
                    emit_all_gist_artifacts(&app, &index);
                    if let Some(s) = app.try_state::<BrainState>() {
                        s.registry.save_to(&cfg_path); // persist the project list
                    }
                }
            },
            BrainEvent::RemoveProject { project, root } => {
                if let Err(e) = index.remove_project(&project) {
                    log::warn!("brain: remove_project '{project}' prune failed ({e})");
                } else {
                    log::info!("brain: removed project '{project}' (unregistered + pruned)");
                }
                // Drop the per-project librarian state too, so a round still in flight
                // (offloaded provider call) completes on the reconcile-only path — no
                // orphan proposals, no resurrected pin. Without this the LibrarianDone
                // handler would find a stale entry and re-persist the pin the prune just
                // deleted. [LIB-DESIGN-01 miss2]
                lib_state.remove(&project);
                // ADR-019: delete the DERIVED gist hook artifact too (root captured
                // by the command before the registry entry vanished) — an
                // unregistered project must not keep injecting a frozen gist.
                // Serialized on this writer thread AFTER any queued emits, so a
                // racing emit can't resurrect it. Never touches user-authored files.
                if let Some(r) = &root {
                    if gist::artifact::remove(std::path::Path::new(r)) {
                        log::debug!("brain: removed gist hook artifact for '{project}'");
                    }
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
                run_librarian_rounds(&app, &index, &mut lib_state, &tx);
                // ADR-020 retention: cap + TTL the activity trail per project. When
                // a prune actually dropped rows the activity SET changed (gist-
                // material) — refresh that artifact so it never quotes pruned rows.
                prune_activity_all(&app, &index);
                // ADR-019 day-boundary refresh: the overdue-note set is
                // day-granular, so labels can flip at UTC midnight with no memory
                // event to piggyback on. Once per day; the per-project byte
                // compare keeps an unaffected artifact untouched.
                let today = utc_date_ymd(now_epoch_ms());
                if today != last_emit_day {
                    last_emit_day = today;
                    emit_all_gist_artifacts(&app, &index);
                }
            }
            BrainEvent::LibrarianDone { project, finish, result } => {
                // The offloaded provider call finished (LIB-DESIGN-01). Complete the
                // round on THIS single writer thread. Whether the project is still
                // registered decides the tail: `lib_state` is pruned on RemoveProject
                // (below), so a MISSING entry means the project was unregistered
                // mid-flight. [LIB-DESIGN-01 miss2]
                let now_ms = now_epoch_ms();
                let mut registered = false;
                match lib_state.get_mut(&project) {
                    Some(st) => {
                        // Registered: reconcile + validate + enqueue proposals, then
                        // CAS-fold the outcome + persist the digest pin.
                        registered = true;
                        st.in_flight = false;
                        let (outcome, digest_hash) =
                            reflect::reflect_finish(&index, &project, finish, result, now_ms);
                        // Guard the pin against a manual Reflect that pinned a newer
                        // digest while this round's call was in flight. [miss1]
                        let expected = st.in_flight_from.take();
                        fold_offloaded_outcome(st, expected, &outcome.reason, digest_hash);
                        persist_lib_pin(&index, &project, st, now_ms);
                        log_round_outcome(&project, &outcome, st);
                        // ADR-020: ONE coalesced event per completed paid round
                        // (Unchanged/$0 skips and failures stay silent — ambient,
                        // not alarming; failures already log loudly).
                        if matches!(outcome.reason, reflect::ReflectReason::Ok) {
                            emit_brain_activity(
                                &app,
                                &project,
                                "reflected",
                                outcome.proposals.len(),
                                Some(outcome.spent_usd),
                            );
                        }
                    }
                    None => {
                        // Unregistered mid-flight: reconcile the budget reservation
                        // ONLY. Enqueuing proposals would leave orphan rows and
                        // re-persisting the pin would resurrect the one remove_project
                        // deliberately deleted (re-added identical corpus → Unchanged/$0
                        // forever, pruned proposals never regenerate). [miss2]
                        let outcome =
                            reflect::reflect_reconcile_only(&index, &project, finish, result, now_ms);
                        log::debug!(
                            "brain: librarian result for gone project '{project}' reconciled ({:?})",
                            outcome.reason
                        );
                    }
                }
                // ADR-018: in autonomous mode, apply what the finished round just
                // enqueued (plus any leftovers, e.g. from a mode flip), then re-pin
                // the post-apply digest so the round doesn't self-re-fire. Runs
                // AFTER the fold so the `st` borrow has ended; skipped for the
                // unregistered branch, which enqueued nothing.
                if registered {
                    if let Some(h) = auto_apply_sweep(&app, &index, &project, now_ms) {
                        lib_entry(&mut lib_state, &index, &project).digest_hash = Some(h);
                    }
                }
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
                    // ADR-018: in autonomous mode the findings land immediately
                    // (revertible); re-pin so the applies don't re-fire a paid round.
                    if let Some(h) = auto_apply_sweep(&app, &index, &pid, now_ms) {
                        lib_entry(&mut lib_state, &index, &pid).digest_hash = Some(h);
                    }
                }
            }
            BrainEvent::ResolveProposal { project, signature, reject, reply } => {
                // Reject = status flip + reject-signature (unchanged). Approve = APPLY:
                // materialize the note change onto disk on this single writer thread,
                // where the writer connection + the registry (project root) both live.
                // (Review-mode path; auto=false — a human clicked Approve.)
                let now_ms = now_epoch_ms();
                let result: Result<(), String> = if reject {
                    index
                        .resolve_proposal(&project, &signature, true)
                        .map(|_| ())
                        .map_err(|e| format!("reject proposal: {e}"))
                } else {
                    match project_root(&app, &project) {
                        Some(root) => {
                            let now_date = memory::apply::epoch_ms_to_iso_date(now_ms);
                            index.apply_proposal(
                                &project,
                                std::path::Path::new(&root),
                                &signature,
                                &now_date,
                                now_ms,
                                false,
                            )
                        }
                        None => Err(format!("unknown project '{project}'")),
                    }
                };
                if let Err(e) = &result {
                    log::warn!("brain: resolve proposal '{signature}' failed ({e})");
                }
                // ADR-019: a review-mode approve just materialized a memory change.
                if !reject && result.is_ok() {
                    emit_gist_artifact(&app, &index, &project);
                }
                if let Some(tx) = reply {
                    let _ = tx.send(result); // command awaits this; ignore a dropped rx
                }
            }
            BrainEvent::RevertProposal { project, signature, reply } => {
                // ADR-018 undo: restore the pre-apply snapshot on this single writer
                // thread, flip to `reverted`, persist the reject-signature (an undone
                // change must not be re-proposed + re-applied next round).
                let now_ms = now_epoch_ms();
                let mut reverted = false;
                let result: Result<(), String> = match project_root(&app, &project) {
                    Some(root) => index
                        .revert_proposal(&project, std::path::Path::new(&root), &signature, now_ms)
                        .map(|did| {
                            reverted = did;
                        }),
                    None => Err(format!("unknown project '{project}'")),
                };
                if let Err(e) = &result {
                    log::warn!("brain: revert proposal '{signature}' failed ({e})");
                }
                // A revert is a brain-originated memory write too — re-pin so the
                // next autonomous round doesn't pay to re-read its own undo.
                if reverted {
                    let today = utc_date_ymd(now_ms);
                    if let Some(h) = reflect::pin_corpus_digest(&index, &project, Some(&today), now_ms)
                    {
                        lib_entry(&mut lib_state, &index, &project).digest_hash = Some(h);
                    }
                    // ADR-019: the undo changed memory — refresh the artifact.
                    emit_gist_artifact(&app, &index, &project);
                    // ADR-020: one event per revert (a human-visible memory change).
                    emit_brain_activity(&app, &project, "reverted", 1, None);
                }
                if let Some(tx) = reply {
                    let _ = tx.send(result);
                }
            }
            BrainEvent::SetCurationMode { mode } => {
                if let Err(e) = index.set_curation_mode(&mode, now_epoch_ms()) {
                    log::warn!("brain: set curation mode failed ({e})");
                }
            }
            BrainEvent::SetInjectGist { on } => {
                if let Err(e) = index.set_inject_gist(on, now_epoch_ms()) {
                    log::warn!("brain: set inject_gist failed ({e})");
                }
                if on {
                    emit_all_gist_artifacts(&app, &index);
                } else {
                    // OFF deletes the artifacts AND stops regeneration (every emit
                    // path re-checks the toggle) — the hook then finds nothing, so
                    // sessions never see stale memory (ADR-019).
                    if let Some(state) = app.try_state::<BrainState>() {
                        for p in state.registry.projects() {
                            if gist::artifact::remove(std::path::Path::new(&p.root)) {
                                log::debug!("brain: removed gist hook artifact for '{}'", p.name);
                            }
                        }
                    }
                }
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
                    // ADR-018: autonomous mode applies the curation verdicts now.
                    if let Some(h) = auto_apply_sweep(&app, &index, &pid, now_ms) {
                        lib_entry(&mut lib_state, &index, &pid).digest_hash = Some(h);
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
                        let st = lib_entry(&mut lib_state, &index, &pid);
                        match outcome.reason {
                            reflect::ReflectReason::Ok => {
                                st.digest_hash = digest_hash;
                                st.fail_streak = 0;
                            }
                            reflect::ReflectReason::InvalidOutput => st.digest_hash = digest_hash,
                            _ => {} // CallFailed: retrying the same digest is legitimate
                        }
                        // Persist the manually-reflected digest too, so the next
                        // autonomous round (this session OR after a restart) does not
                        // re-pay it. [LIB-SPEND-01]
                        persist_lib_pin(&index, &pid, st, now_ms);
                    }
                    log::info!(
                        "brain: reflect '{pid}' → {:?} ({} proposal(s), ${:.4})",
                        outcome.reason,
                        outcome.proposals.len(),
                        outcome.spent_usd
                    );
                    // ADR-020: one coalesced event per completed manual round.
                    if matches!(outcome.reason, reflect::ReflectReason::Ok) {
                        emit_brain_activity(
                            &app,
                            &pid,
                            "reflected",
                            outcome.proposals.len(),
                            Some(outcome.spent_usd),
                        );
                    }
                    // ADR-018: autonomous mode applies what this manual round
                    // enqueued; the sweep re-pins so it doesn't self-re-fire.
                    if let Some(h) = auto_apply_sweep(&app, &index, &pid, now_ms) {
                        lib_entry(&mut lib_state, &index, &pid).digest_hash = Some(h);
                    }
                }
            }
            BrainEvent::Fs { project, changed } => {
                if let Some(root) = project_root(&app, &project) {
                    let (stats, accepted_rels) = index_changed_accepted(
                        &index,
                        &project,
                        std::path::Path::new(&root),
                        &changed,
                    );
                    if stats.indexed > 0 || stats.pruned > 0 {
                        log::debug!(
                            "brain: incremental '{project}' indexed {}, pruned {}",
                            stats.indexed,
                            stats.pruned
                        );
                        // Record the change (dirty + when). The Librarian settles in
                        // after a quiet spell, deciding via the digest hash whether
                        // there's actually anything new to reflect on.
                        // `lib_entry` hydrates the persisted delta-gate pin on the
                        // first sighting of this project this boot, so a code-only edge
                        // (digest-neutral) after a restart won't re-pay. [LIB-SPEND-01]
                        note_content_change(lib_entry(&mut lib_state, &index, &project), now_epoch_ms());
                    }
                    // ADR-020: fan REAL indexed changes into one coarse files-touched
                    // activity row, debounced per project (last-touch coarseness —
                    // project-global attribution, not per-session). Redacted at
                    // ingest like every activity payload.
                    if stats.indexed > 0 {
                        let now_ms = now_epoch_ms();
                        let last = files_activity_last.get(&project).copied().unwrap_or(0);
                        if now_ms.saturating_sub(last) >= FILES_ACTIVITY_DEBOUNCE_MS {
                            files_activity_last.insert(project.clone(), now_ms);
                            let payload = files_activity_payload(&accepted_rels);
                            if let Err(e) =
                                index.record_activity(&project, None, "files", &payload, now_ms)
                            {
                                log::debug!("brain: files activity write failed ({e})");
                            }
                            // The NARRATIVE layer. A files row can only say which
                            // paths moved; the commit subject says what the work
                            // WAS, which is the actual answer to "what did we last
                            // work on?". Recorded only when HEAD has MOVED since the
                            // last row for this project, so a working-tree edit on an
                            // unchanged HEAD writes nothing and the day's commit set
                            // stays stable (the gist key folds it — re-recording an
                            // unchanged subject would be churn, not signal).
                            //
                            // Rides the same debounce, and is fail-open: a non-git
                            // project or a missing git simply records no commit rows.
                            if let Some((sha, subject)) =
                                crate::modules::brain::store::head_commit_readonly(
                                    std::path::Path::new(&root),
                                )
                            {
                                let seen = head_commit_last.get(&project);
                                if seen.map(|s| s != &sha).unwrap_or(true) {
                                    head_commit_last.insert(project.clone(), sha);
                                    let redacted = secrets::redact(&subject).0;
                                    if let Err(e) = index
                                        .record_activity(&project, None, "commit", &redacted, now_ms)
                                    {
                                        log::debug!("brain: commit activity write failed ({e})");
                                    }
                                }
                            }
                        }
                    }
                    // ADR-019: a note-FILE change is gist-material — refresh the
                    // artifact after `index_changed`'s note re-scan. Deliberately
                    // NOT keyed on plain code edits (the temporal digest moves on
                    // every real edit; re-emitting there would churn the file — and
                    // each session's turn context — near-constantly). The artifact's
                    // OWN write event lands here too and converges as a byte-
                    // identical no-write instead of oscillating.
                    let mem_marker = format!("/{}/", memory::MEMORY_DIR);
                    if changed.iter().any(|p| to_canon(p).contains(&mem_marker)) {
                        emit_gist_artifact(&app, &index, &project);
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
    // Discovered (not explicit) registration: the launch-dir seed also runs when
    // the config loaded but every project was removed — honoring the tombstone
    // keeps a removed launch-dir project removed across that boot too.
    match root {
        Some(p) if is_sane_root(&p) => match state.registry.add_root_discovered(&p) {
            Some(proj) => log::info!("brain: seeded project '{}' ({})", proj.name, proj.root),
            None => log::warn!("brain: did not seed project for {} (removed or unregisterable)", p.display()),
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

/// The single-dir form of the workspace child-marker test (ADR-021): a real
/// project is a non-ignored directory that is a git repo or carries a manifest.
/// Shared by [discover_workspace_projects] and the first-use candidate walk —
/// one rule, no drift copy.
pub fn qualifies_as_project(p: &std::path::Path) -> bool {
    p.is_dir() && !is_ignored_dir(p) && has_project_marker(p)
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
        .filter(|p| qualifies_as_project(p))
        .collect();
    out.sort();
    out
}

/// Register every qualifying immediate child of `root` as its OWN project — the
/// `brain_set_workspace` loop, shared with boot re-discovery (ADR-021; extracted
/// so the two paths cannot drift). Idempotent by stable id: an already-registered
/// child is returned but not re-added. An explicitly REMOVED child (tombstoned in
/// `workspace.json`) is skipped entirely — an automatic scan must not undo the
/// user's confirmed remove; `brain_add_project` is the opt-back-in. Returns
/// `(registered children, newly added)`.
pub fn register_workspace_children(
    registry: &KodenBrainRegistry,
    root: &std::path::Path,
) -> (Vec<Project>, usize) {
    let known: std::collections::HashSet<String> =
        registry.projects().into_iter().map(|p| p.id).collect();
    let mut children = Vec::new();
    let mut added = 0usize;
    for child in discover_workspace_projects(root) {
        if let Some(p) = registry.add_root_discovered(&child) {
            if !known.contains(&p.id) {
                added += 1;
            }
            children.push(p);
        }
    }
    (children, added)
}

/// ADR-021 nearest-ancestor rule: walking UP from `cwd`, the FIRST dir that
/// [qualifies_as_project] STRICTLY below the workspace root is the candidate —
/// nested-git-in-git picks the inner repo. A qualifying dir inside an ignored
/// subtree (`node_modules/<dep>/package.json` is a project marker on every npm
/// dependency) is discarded when the walk crosses the ignored component; the
/// search continues above it. `None` when `cwd` is not strictly under the root,
/// IS the root itself (never registered), or no ancestor qualifies. Comparisons
/// use the registry norms (`to_canon` + Windows-only case fold), so an OSC 7
/// `c:\ws\repo` cwd still matches a stored `C:/ws` root.
pub fn first_use_candidate(
    workspace_root: &std::path::Path,
    cwd: &std::path::Path,
) -> Option<std::path::PathBuf> {
    use crate::modules::brain::registry::fold_case;
    let root_n = fold_case(to_canon(workspace_root).trim_end_matches('/'));
    let cwd_n = fold_case(to_canon(cwd).trim_end_matches('/'));
    if root_n.is_empty() || cwd_n == root_n || !cwd_n.starts_with(&format!("{root_n}/")) {
        return None;
    }
    let mut candidate: Option<std::path::PathBuf> = None;
    for anc in cwd.ancestors() {
        let anc_n = fold_case(to_canon(anc).trim_end_matches('/'));
        if anc_n == root_n || !anc_n.starts_with(&format!("{root_n}/")) {
            break; // reached (or escaped) the workspace root — exclusive
        }
        if is_ignored_dir(anc) {
            candidate = None; // anything found below lives in an ignored subtree
            continue;
        }
        if candidate.is_none() && qualifies_as_project(anc) {
            candidate = Some(anc.to_path_buf());
        }
    }
    candidate
}

/// The resolve-or-register seam (ADR-021, testable over a bare registry):
/// resolve `cwd`; on a miss find the [first_use_candidate] under the workspace
/// root, register it, and RETRY the resolution — the triggering signal/turn is
/// never lost. Returns the project plus whether THIS call registered it.
/// `None`: cwd resolves nowhere and no candidate exists (no workspace root, cwd
/// outside it or the root itself, no qualifying ancestor, insane root, or the
/// candidate carries a removal tombstone — explicitly removed projects never
/// auto-return).
/// Guards: the root itself never qualifies (strict-prefix walk); a candidate
/// inside an EXISTING project is impossible (that prefix would have resolved);
/// debounce is structural — the single worker thread is the only caller, and
/// once registered the next trigger short-circuits on the resolve.
pub(crate) fn resolve_or_register_project(
    registry: &KodenBrainRegistry,
    cwd: &str,
) -> Option<(Project, bool)> {
    if let Some(p) = registry.resolve(cwd) {
        return Some((p, false));
    }
    // Canonicalize BEFORE the candidate walk: a junction/symlink or `..`
    // spelling inside the workspace can point anywhere, and `register`
    // stores the canonical TARGET — judging the raw spelling would let a
    // lexically-under-root cwd register a root outside the consent boundary,
    // and a lexical retry on the raw spelling would then miss what was just
    // registered (a silent "ghost" registration with no Rescan/log/toast).
    let canon_cwd = std::fs::canonicalize(cwd)
        .map(|p| crate::modules::fs::to_canon(&p))
        .unwrap_or_else(|_| cwd.to_string());
    if canon_cwd != cwd {
        if let Some(p) = registry.resolve(&canon_cwd) {
            return Some((p, false));
        }
    }
    let root = registry.workspace_root()?;
    let candidate =
        first_use_candidate(std::path::Path::new(&root), std::path::Path::new(&canon_cwd))?;
    if !is_sane_root(&candidate) {
        return None;
    }
    // Discovered (not explicit) registration: honors removal tombstones, so a
    // session inside a project the user removed does NOT re-register it. Use
    // the returned Project directly — never re-resolve a spelling that might
    // not match the canonical registered root.
    registry.add_root_discovered(&candidate).map(|p| (p, true))
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

/// A bounded raw read + binary sniff + blake3, classified. The shared front of
/// both the serial per-file path ([index_one_file]) and the parallel compute
/// workers ([compute_one_file]).
enum RawRead {
    /// Text within the size cap: raw bytes + their freshness hash.
    Text(Vec<u8>, String),
    NotIndexable,
    Absent,
    Unknown,
}

fn read_for_index(rel: &str, path: &std::path::Path) -> RawRead {
    // Bounded read (ADR-010 TOCTOU): the walker's stat-time size check can be
    // minutes stale, so re-enforce the cap at read time with a take()-bounded
    // reader — a file that grew past the cap can never balloon memory.
    use std::io::Read as _;
    let mut bytes: Vec<u8> = Vec::new();
    match std::fs::File::open(path) {
        Ok(f) => {
            if let Err(e) = f.take(walk::MAX_INDEX_FILE_BYTES + 1).read_to_end(&mut bytes) {
                log::debug!("brain: read failed for {rel}: {e}");
                return RawRead::Unknown;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RawRead::Absent,
        Err(e) => {
            log::debug!("brain: open failed for {rel}: {e}");
            return RawRead::Unknown;
        }
    }
    if bytes.len() as u64 > walk::MAX_INDEX_FILE_BYTES {
        return RawRead::NotIndexable; // grew past the cap since the stat
    }
    // Binary sniff — skip files with a NUL in the first window.
    if bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
        return RawRead::NotIndexable;
    }
    // Freshness hash is over the RAW bytes (any change reindexes).
    let file_hash = hash::hash_bytes(&bytes);
    RawRead::Text(bytes, file_hash)
}

/// Read → binary-sniff → blake3 → secrets-redact → index one file. The serial
/// per-file path, used by the incremental watcher (`index_changed`). On a
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
    let (bytes, file_hash) = match read_for_index(rel, path) {
        RawRead::Text(bytes, hash) => (bytes, hash),
        RawRead::NotIndexable => return FileOutcome::NotIndexable,
        RawRead::Absent => return FileOutcome::Absent,
        RawRead::Unknown => return FileOutcome::Unknown,
    };
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

/// Compute-only classification of one file — the parallel half of the full-walk
/// pipeline. NO store access: everything here (read, sniff, hash, redact,
/// tokenize, parse) is pure or fs-read-only, so N of these run concurrently
/// while the single writer applies results in walk order.
enum ComputedFile {
    /// Read + hashed + redacted + tokenized/parsed — ready for the writer.
    Prepared(crate::modules::brain::store::PreparedFile),
    /// Hash matches the pass-start snapshot — present, nothing to write
    /// (mirrors `index_file`'s Ok(false), which also skips `record_access`).
    Unchanged,
    NotIndexable,
    Absent,
    Unknown,
}

/// The compute worker body: read → sniff → hash → (snapshot skip) → redact →
/// tokenize/parse. `known_hash` is the project's pass-start hash for this rel;
/// matching it skips the expensive tokenize/parse exactly where the serial path's
/// writer-side hash check used to.
fn compute_one_file(
    rel: &str,
    path: &std::path::Path,
    known_hash: Option<&str>,
) -> ComputedFile {
    let (bytes, file_hash) = match read_for_index(rel, path) {
        RawRead::Text(bytes, hash) => (bytes, hash),
        RawRead::NotIndexable => return ComputedFile::NotIndexable,
        RawRead::Absent => return ComputedFile::Absent,
        RawRead::Unknown => return ComputedFile::Unknown,
    };
    if known_hash == Some(file_hash.as_str()) {
        return ComputedFile::Unchanged;
    }
    let content = String::from_utf8_lossy(&bytes);
    // Secrets gate: redact secret-shaped content before it is tokenized/stored.
    let (redacted, nredact) = secrets::redact(&content);
    if nredact > 0 {
        log::debug!("brain: redacted {nredact} secret-shaped span(s) in {rel}");
    }
    let size = bytes.len() as i64;
    drop(bytes); // raw bytes done — cap in-flight memory to the redacted text
    ComputedFile::Prepared(crate::modules::brain::store::prepare_file(
        rel, &redacted, file_hash, size,
    ))
}

/// Apply one computed result on the writer thread. Mirrors the tail of
/// [index_one_file]: recency advances only on a REAL change (Ok(true)).
fn apply_computed(
    index: &SqliteIndex,
    project_id: &str,
    rel: &str,
    computed: ComputedFile,
    now_ms: i64,
) -> FileOutcome {
    match computed {
        ComputedFile::Prepared(prep) => match index.index_file_prepared(project_id, rel, &prep) {
            Ok(true) => {
                let _ = index.record_access(project_id, rel, now_ms);
                FileOutcome::Indexed
            }
            Ok(false) => FileOutcome::Indexed, // raced-unchanged no-op — present
            Err(e) => {
                log::debug!("brain: index_file failed for {rel}: {e}");
                FileOutcome::Unknown
            }
        },
        ComputedFile::Unchanged => FileOutcome::Indexed, // present, recency unchanged
        ComputedFile::NotIndexable => FileOutcome::NotIndexable,
        ComputedFile::Absent => FileOutcome::Absent,
        ComputedFile::Unknown => FileOutcome::Unknown,
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

/// Parallel full-walk tuning. Compute workers are `available_parallelism`
/// leaving one core for the writer, clamped:
/// ponytail: fixed ceiling of 8 workers — the writer is a single SQLite
/// connection, so past a handful of parsers it becomes the bottleneck anyway;
/// lift to a setting only if a monster-repo profile shows headroom.
const PAR_MAX_WORKERS: usize = 8;
/// Bounded results channel (backpressure between compute and the writer).
const PAR_RESULTS_CAP: usize = 16;
/// Reorder-buffer bound: a worker may not START file `i` until the writer has
/// applied file `i - PAR_MAX_LEAD`, so compute leads the writer by at most this
/// many files. The channel cap alone does NOT bound memory: under head-of-line
/// skew (walk-first file is a slow ~1 MB parse) the consumer drains every result
/// into the `pending` reorder map without applying any, and peak RAM approaches
/// a tokenized copy of the whole repo. With the lead cap, in-flight payloads
/// (in workers + channel + `pending`) are hard-capped at PAR_MAX_LEAD total.
/// ponytail: 32 = channel cap + 2× max workers of headroom; profile a monster
/// repo before raising.
const PAR_MAX_LEAD: usize = 32;

fn par_workers(files: usize) -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1)) // leave a core for the writer/consumer
        .unwrap_or(1)
        .clamp(1, PAR_MAX_WORKERS)
        .min(files.max(1))
}

/// Pipeline body, parametrized over the walk outcome so tests can drive the
/// reconcile gate directly (a real >MAX_SCANNED repo is too heavy for CI).
///
/// The per-file compute (read+hash+redact+tokenize+parse — the wall-clock bulk
/// of a first index) fans out over [par_workers] scoped threads; ALL writes stay
/// on the caller's thread, which applies results IN WALK ORDER via stable
/// sequence numbers — so the single-writer invariant holds and the resulting DB
/// (incl. FTS insertion order) is byte-identical to the old serial pass.
/// The incremental watcher path ([index_changed]) deliberately stays serial:
/// deltas are a handful of files, the store's hash-skip already makes no-ops
/// cheap, and thread fan-out would add latency/machinery for no wall-clock win.
fn index_walked(
    index: &SqliteIndex,
    project_id: &str,
    root: &std::path::Path,
    walked: walk::Walked,
) -> IndexStats {
    let now_ms = now_epoch_ms(); // one recency stamp for everything changed in this pass
    let mut indexed = 0usize;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Pass-start hash snapshot: lets compute workers skip tokenize/parse on
    // unchanged files (the compute-side twin of the writer-side hash-skip).
    // Fail-open: an error just means workers prepare everything and the writer's
    // own hash check still no-ops the unchanged ones.
    let known_hashes = index.existing_hashes(project_id).unwrap_or_default();
    // Walk order is the deterministic apply sequence; rels are precomputed so
    // both sides of the pipeline agree on file identity.
    let files = walked.files;
    let rels: Vec<String> = files.iter().map(|p| rel_path(root, p)).collect();
    let n_workers = par_workers(files.len());
    if !files.is_empty() {
        let next = std::sync::atomic::AtomicUsize::new(0);
        // Writer progress (files applied so far) + condvar lead-capped workers
        // sleep on. Set to usize::MAX on consumer exit/unwind so waiting
        // workers always get released and the scope can join.
        let applied = (std::sync::Mutex::new(0usize), std::sync::Condvar::new());
        let (ctx, crx) = mpsc::sync_channel::<(usize, ComputedFile)>(PAR_RESULTS_CAP);
        std::thread::scope(|s| {
            for _ in 0..n_workers {
                let ctx = ctx.clone();
                let (next, files, rels, known_hashes, applied) =
                    (&next, &files, &rels, &known_hashes, &applied);
                s.spawn(move || loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= files.len() {
                        break;
                    }
                    // Lead cap: park until the writer is within PAR_MAX_LEAD
                    // files, bounding the consumer's reorder buffer. Deadlock-
                    // free: the worker holding the lowest unapplied index never
                    // waits (i < applied + PAR_MAX_LEAD holds for it).
                    {
                        let mut done = applied.0.lock().unwrap_or_else(|e| e.into_inner());
                        while i >= done.saturating_add(PAR_MAX_LEAD) {
                            done = applied.1.wait(done).unwrap_or_else(|e| e.into_inner());
                        }
                    }
                    let known = known_hashes.get(&rels[i]).map(String::as_str);
                    let computed = compute_one_file(&rels[i], &files[i], known);
                    if ctx.send((i, computed)).is_err() {
                        break; // consumer gone (unwind) — stop producing
                    }
                });
            }
            drop(ctx); // the consumer loop ends when the last worker finishes
            // On any consumer exit (normal or unwind) release lead-capped
            // workers, else they'd wait forever and the scope would never join.
            struct ReleaseWorkers<'a>(&'a (std::sync::Mutex<usize>, std::sync::Condvar));
            impl Drop for ReleaseWorkers<'_> {
                fn drop(&mut self) {
                    *self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = usize::MAX;
                    self.0 .1.notify_all();
                }
            }
            let _release = ReleaseWorkers(&applied);
            // Single consumer = THIS thread (the one writer). Results arrive in
            // completion order; a small reorder buffer re-sequences them so
            // writes land in walk order. Bounded: the lead cap above keeps
            // `pending` (plus channel + in-worker results) at <= PAR_MAX_LEAD.
            let mut pending: std::collections::HashMap<usize, ComputedFile> =
                std::collections::HashMap::new();
            let mut next_apply = 0usize;
            for (i, computed) in crx {
                pending.insert(i, computed);
                while let Some(c) = pending.remove(&next_apply) {
                    let rel = &rels[next_apply];
                    match apply_computed(index, project_id, rel, c, now_ms) {
                        FileOutcome::Indexed => {
                            seen.insert(rel.clone());
                            indexed += 1;
                        }
                        // Unknown (read/store error): the file may well exist — keep any
                        // last-good row out of the deletion set (ADR-010 positive evidence).
                        FileOutcome::Unknown => {
                            seen.insert(rel.clone());
                        }
                        // NotIndexable: present but excluded (binary/oversize) — a stale row
                        // is pruned, matching a full rebuild. Absent: positively gone.
                        FileOutcome::NotIndexable | FileOutcome::Absent => {}
                    }
                    next_apply += 1;
                    // Publish writer progress; wakes lead-capped workers.
                    *applied.0.lock().unwrap_or_else(|e| e.into_inner()) = next_apply;
                    applied.1.notify_all();
                }
            }
        });
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
/// Deliberately SERIAL (unlike the parallel full walk in [index_walked]): a
/// watcher delta is a handful of files, the store-side hash-skip keeps no-op
/// saves cheap without any pre-pass snapshot, and thread fan-out here would be
/// pure overhead.
pub fn index_changed(
    index: &SqliteIndex,
    project_id: &str,
    root: &std::path::Path,
    changed: &[std::path::PathBuf],
) -> IndexStats {
    index_changed_accepted(index, project_id, root, changed).0
}

/// [index_changed], additionally returning the project-relative paths that
/// actually SURVIVED every gate and were indexed. The activity trail (ADR-020)
/// consumes this instead of the raw watcher batch: deriving "files touched" from
/// `changed` re-implemented the gate set and drifted, so a `git commit` fanned
/// `.git/index.lock` into the injected gist's "Recent activity" while the indexer
/// had correctly skipped it. One gate, one definition — the trail cannot disagree
/// with the index about what a touched file is, by construction rather than by
/// a second filter kept in sync by hand.
pub fn index_changed_accepted(
    index: &SqliteIndex,
    project_id: &str,
    root: &std::path::Path,
    changed: &[std::path::PathBuf],
) -> (IndexStats, Vec<String>) {
    let now_ms = now_epoch_ms(); // recency stamp for whatever changed in this delta
    let mut indexed = 0usize;
    let mut pruned = 0usize;
    // Delta sets for the edge relink: paths (re)indexed this pass (may include
    // unchanged no-ops — a safe over-approximation, they relink to the same
    // edges) and paths positively removed. Drives `relink_edges_delta` so the
    // per-event edge cost is ∝ this delta, not O(project imports).
    let mut changed_rels: Vec<String> = Vec::new();
    let mut removed_rels: Vec<String> = Vec::new();
    for path in changed {
        let rel = rel_path(root, path);
        if rel.is_empty() {
            continue;
        }
        // Skip-dir gate on the PROJECT-RELATIVE path: an absolute-path check
        // would zero out incremental updates for a project that itself lives
        // under a dir named e.g. `build/` or `vendor/` (ADR-010 cluster 2).
        // The reserved-artifact gate (ADR-019) mirrors the full walk's — any
        // source honored on one side but not the other re-opens index/prune
        // oscillation, and for the gist artifact that oscillation is unbounded
        // (its bytes embed the project fingerprint).
        if walk::rel_under_skip_dir(&rel)
            || walk::is_reserved_artifact(path)
            || secrets::is_denylisted_path(&to_canon(path))
        {
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
                    changed_rels.push(rel.clone());
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
                        changed_rels.push(crel);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Positively gone (deleted / moved away) — prune the stale row + FTS doc.
                if index.remove_file(project_id, &rel).unwrap_or(false) {
                    pruned += 1;
                    removed_rels.push(rel.clone());
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
                            removed_rels.push(p);
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
    // Delta edge relink — only the srcs this change-set can affect (incl. the
    // dst side via stored import bases); converges byte-identically with a full
    // rebuild (perf pair: proportional to the delta, not O(project)).
    let _ = index.relink_edges_delta(project_id, &changed_rels, &removed_rels);
    (IndexStats { indexed, pruned }, changed_rels)
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
    /// A round for this project is dispatched and its provider call is running on a
    /// helper thread; the worker skips new rounds for it until the matching
    /// `BrainEvent::LibrarianDone` lands. Prevents overlapping paid rounds / double
    /// reservations while the network call is offloaded. [LIB-DESIGN-01]
    pub in_flight: bool,
    /// The delta-gate pin captured when the in-flight round was DISPATCHED. On
    /// `LibrarianDone` the handler CAS-folds this round's prepare-time hash into the
    /// pin ONLY if `digest_hash` still equals this — otherwise a manual `Reflect` that
    /// landed mid-flight already pinned a NEWER digest, and clobbering it with this
    /// (older) round's hash would make the next auto round re-pay it. [LIB-DESIGN-01 miss1]
    pub in_flight_from: Option<String>,
}

/// Fold one indexed content change into the state — the Fs-handler half of the
/// retry policy: re-arm the round and restart the idle-settle clock. The fail
/// streak is deliberately NOT reset here — only a successful round clears it —
/// so a still-failing provider keeps its widened backoff gap even as edits land.
pub fn note_content_change(st: &mut LibrarianAuto, now_ms: i64) {
    st.dirty = true;
    st.last_change_ms = now_ms;
}

/// Get-or-create the per-project state, HYDRATING the delta-gate pin from the
/// persisted store the first time a project is seen this boot. `lib_state` is a
/// HashMap rebuilt EMPTY on every worker start (:232), so without this read-through
/// the first post-restart round would run with `prev_digest_hash=None` and re-pay a
/// byte-identical digest — a call the pin would have short-circuited to Unchanged at
/// $0. The DB read happens only on the miss (`or_insert_with`), never when the entry
/// already exists. [LIB-SPEND-01]
pub fn lib_entry<'a>(
    state: &'a mut std::collections::HashMap<String, LibrarianAuto>,
    index: &SqliteIndex,
    project_id: &str,
) -> &'a mut LibrarianAuto {
    state
        .entry(project_id.to_string())
        .or_insert_with(|| LibrarianAuto { digest_hash: index.librarian_pin(project_id), ..Default::default() })
}

/// Persist a project's delta-gate pin after a round, so the "Unchanged => $0"
/// short-circuit survives a restart. A no-op when the pin is still `None` (no round
/// has established a digest yet); a repeat of an unchanged value is a harmless upsert.
/// A failed write is logged, never propagated — the in-memory pin still gates the
/// rest of THIS session, and the next successful round re-attempts the write. [LIB-SPEND-01]
fn persist_lib_pin(index: &SqliteIndex, project_id: &str, st: &LibrarianAuto, now_ms: i64) {
    if let Some(h) = st.digest_hash.as_deref() {
        if let Err(e) = index.set_librarian_pin(project_id, h, now_ms) {
            log::warn!("brain: persist librarian pin for '{project_id}' failed ({e})");
        }
    }
}

/// ADR-018 — autonomous curation: APPLY every pending proposal for a project on
/// the single writer thread, each apply snapshotting its inverse first (undo).
/// A no-op returning 0 unless the persisted curation mode is `autonomous` (in
/// 'review' mode proposals keep waiting for a human — behavior unchanged). A soft
/// apply failure (e.g. the target note was renamed/deleted) leaves THAT proposal
/// pending — it stays visible in the inbox — and is logged, never fatal to the
/// sweep. Returns the number applied. `pub` (like [index_dir]) so the `tests/`
/// integration driver exercises the real sweep; not a stable surface.
pub fn auto_apply_pending(
    index: &SqliteIndex,
    project_id: &str,
    root: &std::path::Path,
    now_ms: i64,
) -> usize {
    use crate::modules::brain::reflect::librarian;
    if index.curation_mode() != librarian::CURATION_AUTONOMOUS {
        return 0;
    }
    let sigs = index.pending_proposal_signatures(project_id).unwrap_or_default();
    if sigs.is_empty() {
        return 0;
    }
    let now_date = memory::apply::epoch_ms_to_iso_date(now_ms);
    let mut applied = 0usize;
    for sig in sigs {
        match index.apply_proposal(project_id, root, &sig, &now_date, now_ms, true) {
            Ok(()) => applied += 1,
            Err(e) => {
                log::warn!("brain: auto-apply of '{sig}' left pending for '{project_id}' ({e})")
            }
        }
    }
    if applied > 0 {
        log::info!("brain: auto-applied {applied} memory proposal(s) for '{project_id}' (revertible)");
    }
    applied
}

/// The ADR-018 sweep tail every enqueue site runs: auto-apply pending proposals
/// (autonomous mode only, unregistered projects skipped — the RemoveProject gate),
/// then — ONLY when something was applied — re-pin the post-apply corpus digest so
/// the next autonomous round doesn't pay to reflect on the brain's OWN writes (the
/// self-feeding loop `build_digest`'s enqueue-only invariant no longer covers).
/// Returns the new pin for the caller to fold into `LibrarianAuto.digest_hash`.
/// Deliberate trade-off: the pin covers the whole CURRENT corpus, so a user edit
/// landing in the same window is skipped for this round and picked up by its next
/// change (the watcher still fires; the gate short-circuits at $0) — budget
/// protection outranks reflecting on one intermediate state.
fn auto_apply_sweep(
    app: &AppHandle,
    index: &SqliteIndex,
    project_id: &str,
    now_ms: i64,
) -> Option<String> {
    let root = project_root(app, project_id)?;
    let applied = auto_apply_pending(index, project_id, std::path::Path::new(&root), now_ms);
    if applied == 0 {
        return None;
    }
    let today = utc_date_ymd(now_ms);
    let pin = reflect::pin_corpus_digest(index, project_id, Some(&today), now_ms);
    // ADR-019: the sweep just materialized memory changes (autonomous applies
    // after LibrarianDone / Doctor / Curate / Reflect / boot) — refresh the
    // per-project gist hook artifact so live sessions see them next turn.
    emit_gist_artifact(app, index, project_id);
    // ADR-020: ONE coalesced event for the whole batch, never per-proposal.
    emit_brain_activity(app, project_id, "applied", applied, None);
    pin
}

// --- ADR-020 session activity + ambient notifications ------------------------

/// Coalesced Librarian activity payload for the `koden:brain-activity` Tauri
/// event. Field names are the frontend contract (snake_case, like every brain DTO).
#[derive(Clone, Debug, serde::Serialize)]
pub struct BrainActivityEvent {
    pub project: String,
    pub project_name: String,
    pub kind: String, // "applied" | "reflected" | "reverted" | "registered"
    pub count: usize,
    pub spent_usd: Option<f64>,
}

/// The coalescing seam (testable without an AppHandle): ONE payload per
/// apply-sweep batch / reflect round / revert. `None` suppresses no-op batches —
/// applied/reverted require `count > 0`; a completed reflect round is activity
/// even at 0 proposals (it spent a paid call the user asked to see).
pub(crate) fn build_activity_event(
    project: &str,
    project_name: &str,
    kind: &str,
    count: usize,
    spent_usd: Option<f64>,
) -> Option<BrainActivityEvent> {
    if count == 0 && kind != "reflected" {
        return None;
    }
    Some(BrainActivityEvent {
        project: project.to_string(),
        project_name: project_name.to_string(),
        kind: kind.to_string(),
        count,
        spent_usd,
    })
}

/// Emit one coalesced activity event to the frontend (fail-open: no registry
/// entry / no window just drops it — ambient chrome, never load-bearing).
fn emit_brain_activity(
    app: &AppHandle,
    project_id: &str,
    kind: &str,
    count: usize,
    spent_usd: Option<f64>,
) {
    let Some(name) = app
        .try_state::<BrainState>()
        .and_then(|s| s.registry.projects().into_iter().find(|p| p.id == project_id))
        .map(|p| p.name)
    else {
        return;
    };
    if let Some(ev) = build_activity_event(project_id, &name, kind, count, spent_usd) {
        if let Err(e) = app.emit(ACTIVITY_EVENT, ev) {
            log::debug!("brain: activity event emit failed ({e})");
        }
    }
}

/// Turn-text ingest filter (pure, testable): trim → drop empty / too-short /
/// slash-command-only turns (the `addBusTurn` trim+cap idiom, turnStore.ts) →
/// truncate to [TURN_MAX_CHARS] on a char boundary → REDACT. Returns the
/// store-ready payload, or `None` for a turn not worth a row.
pub(crate) fn clean_turn_text(prompt: &str) -> Option<String> {
    let t = prompt.trim();
    if t.chars().count() < 2 {
        return None; // empty or a bare keystroke ("y") — noise
    }
    // Slash-command-only turns ("/clear", "/model opus") are UI plumbing, not
    // session intent. A multi-line prompt that merely STARTS with '/' is kept.
    if t.starts_with('/') && !t.contains('\n') {
        return None;
    }
    let cut: String = t.chars().take(TURN_MAX_CHARS).collect();
    // Redaction AT INGEST (the ADR-020 gate): prompt text never lands raw.
    // Redact the truncated text — the stored row IS the cut, so the detector
    // must run over exactly what is stored (a token straddling the cut is
    // stored as a partial, unrecognizable fragment either way; redacting first
    // over the full text and then cutting could split a REDACTED marker instead).
    Some(secrets::redact(&cut).0)
}

/// Coarse files-touched payload: project-relative changed paths, deduped +
/// sorted + capped, JSON-array-encoded (paths may contain any separator), each
/// passed through the same ingest redaction gate for uniformity.
/// Filtered by the same reserved-artifact + denylist gates as the watcher's
/// single-file path (see the Fs arm): the batch that carried a real edit often
/// also carries the brain's own `.koden-gist.json` refresh (an apply writes
/// notes, then the artifact — the watcher coalesces both), and recording our
/// own derived artifact as "activity" would put a self-referential line in the
/// injected gist and rotate its key for nothing.
/// Build the ADR-020 files-touched payload from the paths the indexer ACCEPTED
/// (see [index_changed_accepted]) — never from the raw watcher batch. Every
/// exclusion gate (skip-dirs incl. `.git`, `.gitignore`/`.kodenignore`, reserved
/// artifacts, secret denylist) has already run upstream, so the only thing left
/// to strip here is the transient-write chaff, which is a property of the write
/// rather than of the file's indexability.
fn files_activity_payload(accepted_rels: &[String]) -> String {
    let mut rels: Vec<String> = accepted_rels
        .iter()
        .filter(|r| !r.is_empty() && !is_transient_write(std::path::Path::new(r)))
        .map(|r| secrets::redact(r).0)
        .collect();
    rels.sort();
    rels.dedup();
    rels.truncate(FILES_ACTIVITY_MAX_PATHS);
    serde_json::to_string(&rels).unwrap_or_else(|_| "[]".to_string())
}

/// Editor/atomic-write droppings observed live in the trail (`x.md.tmp.<pid>.<hash>`
/// from atomic writers, `~`/`.#` editor locks): they vanish on rename, so listing
/// them as "touched files" is pure noise in the injected gist and rotates its key
/// for a path no session can ever read.
fn is_transient_write(p: &std::path::Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.contains(".tmp.")
        || name.ends_with(".tmp")
        || name.ends_with('~')
        || name.starts_with(".#")
}

/// Session boundary rows (ADR-020): `started` → a `start` row, `exited` → an
/// `end` row; other lifecycle kinds (working/attention/finished) are status
/// noise, not boundaries. Payload = the agent name (redacted for uniformity).
pub(crate) fn record_session_boundary(
    index: &SqliteIndex,
    project_id: &str,
    kind: &str,
    agent: Option<&str>,
    pty_id: u32,
    now_ms: i64,
) {
    let row_kind = match kind {
        "started" => "start",
        "exited" => "end",
        _ => return,
    };
    let payload = secrets::redact(agent.unwrap_or("")).0;
    if let Err(e) =
        index.record_activity(project_id, Some(pty_id as i64), row_kind, &payload, now_ms)
    {
        log::debug!("brain: session boundary activity write failed ({e})");
    }
}

/// ADR-020 hands-off freshness: an AI session exiting queues a TARGETED
/// reconcile of its project — the session may have written files no watcher
/// event has landed for yet (editor buffers, git operations). The Rescan arm
/// re-indexes and then refreshes the gist artifact (ADR-019 helpers), so the
/// next turn in any other live session sees the results with no manual rescan.
pub(crate) fn enqueue_exit_reconcile(tx: &mpsc::Sender<BrainEvent>, project_id: &str) {
    let _ = tx.send(BrainEvent::Rescan { project: Some(project_id.to_string()) });
}

/// Resolve pty → cwd → project for a non-lifecycle ingest (the Turn leg):
/// the exact `handle_agent` chain — live pty cwd first, then the cwd remembered
/// on the LiveSession (so a turn racing the pty teardown still resolves) —
/// including ADR-021 first-use registration on a miss.
fn resolve_pty_project(
    app: &AppHandle,
    tx: &mpsc::Sender<BrainEvent>,
    pty_id: u32,
) -> Option<String> {
    let brain = app.try_state::<BrainState>()?;
    let remembered = brain
        .sessions
        .read()
        .ok()
        .and_then(|s| s.get(&pty_id).and_then(|x| x.cwd.clone()));
    let cwd = app
        .try_state::<PtyState>()
        .and_then(|pty| pty.session_cwd(pty_id))
        .or(remembered)?;
    resolve_or_register_cwd(app, tx, &cwd).map(|p| p.id)
}

/// ADR-021 register-on-first-use: resolve `cwd` against the registry; on a miss
/// register the nearest qualifying ancestor under the workspace root and retry,
/// so the triggering signal/turn lands in the new project's trail instead of
/// being dropped. A NEW registration takes the exact `brain_add_project` path —
/// registry add, then a reconcile enqueue (index + watcher re-arm + persist +
/// the first artifact emission once indexing completes) — plus an INFO log and
/// ONE coalesced `registered` activity event for the frontend toast.
fn resolve_or_register_cwd(
    app: &AppHandle,
    tx: &mpsc::Sender<BrainEvent>,
    cwd: &str,
) -> Option<Project> {
    let state = app.try_state::<BrainState>()?;
    let (proj, newly_registered) = resolve_or_register_project(&state.registry, cwd)?;
    if newly_registered {
        log::info!(
            "brain: registered new project '{}' on first use ({})",
            proj.name,
            proj.root
        );
        let _ = tx.send(BrainEvent::Rescan { project: None });
        emit_brain_activity(app, &proj.id, "registered", 1, None);
    }
    Some(proj)
}

/// Tick-driven retention over every registered project. A prune that dropped
/// rows changed the activity set (a gist input) — refresh that artifact so it
/// never quotes rows the store no longer holds.
fn prune_activity_all(app: &AppHandle, index: &SqliteIndex) {
    let Some(state) = app.try_state::<BrainState>() else { return };
    let now_ms = now_epoch_ms();
    let ttl_ms = ACTIVITY_TTL_DAYS * 86_400_000;
    for p in state.registry.projects() {
        match index.prune_activity(&p.id, ACTIVITY_MAX_ROWS, ttl_ms, now_ms) {
            Ok(0) => {}
            Ok(n) => {
                log::debug!("brain: pruned {n} activity row(s) for '{}'", p.name);
                emit_gist_artifact(app, index, &p.id);
            }
            Err(e) => log::debug!("brain: activity prune for '{}' failed ({e})", p.name),
        }
    }
}

/// The store path the worker registered in [BrainState] (set early in
/// `brain_loop`, before any emit site can run).
fn state_db_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.try_state::<BrainState>()
        .and_then(|s| s.db_path.read().ok().and_then(|p| p.clone()))
}

/// ADR-019: emit/refresh ONE project's gist hook artifact. Toggle-gated
/// (`brain_librarian.inject_gist`), byte-compare-gated inside
/// [gist::artifact::emit] (unchanged memory → no write, no mtime churn — the
/// prompt-cache contract), and fail-open: any miss just skips this round.
fn emit_gist_artifact(app: &AppHandle, index: &SqliteIndex, project_id: &str) {
    if !index.inject_gist() {
        return;
    }
    let Some(db) = state_db_path(app) else { return };
    let Some(proj) = app
        .try_state::<BrainState>()
        .and_then(|s| s.registry.projects().into_iter().find(|p| p.id == project_id))
    else {
        return;
    };
    match gist::artifact::emit(&db, &proj.id, &proj.name, std::path::Path::new(&proj.root)) {
        Ok(gist::artifact::EmitOutcome::Written) => {
            log::debug!("brain: gist hook artifact refreshed for '{}'", proj.name);
        }
        Ok(_) => {} // Unchanged / NotReady — nothing touched
        Err(e) => {
            log::debug!("brain: gist hook artifact write for '{}' failed ({e})", proj.name);
        }
    }
}

/// ADR-019: emit/refresh every registered project's artifact (boot, full
/// rescan, toggle-on, day boundary). Byte-compare keeps unaffected files
/// untouched, so calling this broadly is cheap.
fn emit_all_gist_artifacts(app: &AppHandle, index: &SqliteIndex) {
    if !index.inject_gist() {
        return;
    }
    let Some(state) = app.try_state::<BrainState>() else { return };
    for p in state.registry.projects() {
        emit_gist_artifact(app, index, &p.id);
    }
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

/// Fold an OFFLOADED round's outcome, guarding the delta-gate pin against a manual
/// `Reflect` that re-pinned a NEWER digest while this round's provider call was in
/// flight (LIB-DESIGN-01 miss1). `expected_pin` is the pin captured when this round
/// was DISPATCHED (`st.in_flight_from`); if `st.digest_hash` no longer equals it, a
/// mid-flight manual round already paid for + pinned a newer digest — keep that pin
/// rather than clobbering it with this round's (older) prepare-time hash, which would
/// make the next autonomous round re-pay a byte-identical, already-paid digest. All
/// other bookkeeping (fail_streak / dirty / re-arm) is identical to
/// [apply_round_outcome] — only the pin write is conditionally preserved.
fn fold_offloaded_outcome(
    st: &mut LibrarianAuto,
    expected_pin: Option<String>,
    reason: &reflect::ReflectReason,
    digest_hash: Option<String>,
) {
    let repinned = st.digest_hash != expected_pin;
    let newer_pin = st.digest_hash.clone();
    apply_round_outcome(st, reason, digest_hash);
    if repinned {
        st.digest_hash = newer_pin; // preserve the mid-flight manual pin
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
    let prev = librarian_round_begin(st, now_ms)?;
    let (outcome, digest_hash) = run(prev.as_deref());
    apply_round_outcome(st, &outcome.reason, digest_hash);
    Some(outcome)
}

/// The "begin" half of a round, split out of [librarian_round_step] so the
/// OFFLOADED worker path (LIB-DESIGN-01) can start a round, run the provider call on
/// a helper thread, and fold the outcome later via [apply_round_outcome]. Decides
/// whether a round is due; if so, CONSUMES the dirty/boundary flags and stamps
/// `last_pass_ms` (the next round is gated regardless of outcome), returning the
/// previous digest hash for the delta gate. Returns `None` when not due. Does NOT
/// touch `in_flight` — the caller sets it after a Pending dispatch.
pub fn librarian_round_begin(st: &mut LibrarianAuto, now_ms: i64) -> Option<Option<String>> {
    if !due_for_round(st.dirty, st.boundary, st.last_change_ms, st.last_pass_ms, now_ms, st.fail_streak) {
        return None;
    }
    st.dirty = false;
    st.boundary = false;
    st.last_pass_ms = now_ms; // gate the next round regardless of outcome
    Some(st.digest_hash.clone())
}

/// One Librarian sweep (driven by the periodic Tick): for each project that changed
/// since its last round and is past the round interval, dispatch ONE delta-gated
/// reflect. End-to-end safe: [reflect::reflect_prepare] no-ops (Disabled) without a
/// budget ceiling and skips the paid call ($0, Unchanged) when the digest is
/// byte-identical to the last round.
///
/// LIB-DESIGN-01: the provider call is OFFLOADED to a helper thread rather than run
/// inline. `reflect_prepare` does the fast, index-touching work on this worker thread
/// (digest read + delta gate + durable budget reserve); when a call is warranted the
/// resulting [reflect::ReflectPending] is moved to a short-lived thread that performs
/// only the network round-trip and posts the result back as
/// [BrainEvent::LibrarianDone]. The worker returns to the event loop immediately, so
/// incremental indexing (`Fs`) is never stalled for the full provider-call duration.
/// Every index WRITE still happens on this single worker thread (prepare here, finish
/// on the `LibrarianDone` handler) — the single-writer invariant is preserved.
fn run_librarian_rounds(
    app: &AppHandle,
    index: &SqliteIndex,
    state: &mut std::collections::HashMap<String, LibrarianAuto>,
    tx: &mpsc::Sender<BrainEvent>,
) {
    let now_ms = now_epoch_ms();
    // Real current date (UTC) so date-dependent findings (stale_revalidate) are
    // visible to autonomous rounds, not only to manual clicks (ADR-010 cluster 5).
    let today = utc_date_ymd(now_ms);
    for (project_id, st) in state.iter_mut() {
        // A round already dispatched for this project is still running on a helper
        // thread; skip until its LibrarianDone lands (no overlap, no double reserve).
        if st.in_flight {
            continue;
        }
        let Some(prev) = librarian_round_begin(st, now_ms) else {
            continue;
        };
        match reflect::reflect_prepare(app, index, project_id, Some(&today), now_ms, prev.as_deref()) {
            // No provider call needed — the outcome is already final (Unchanged /
            // Disabled / NoKey / OverBudget / EmptyCorpus). Fold + log inline; this
            // path never touched the network, so it stays on the worker thread.
            reflect::ReflectDispatch::Ready(outcome, digest_hash) => {
                apply_round_outcome(st, &outcome.reason, digest_hash);
                persist_lib_pin(index, project_id, st, now_ms);
                log_round_outcome(project_id, &outcome, st);
            }
            // A provider call is required — offload it. The worker keeps serving Fs.
            reflect::ReflectDispatch::Pending(pending) => {
                let tx = tx.clone();
                let pid = project_id.clone();
                match std::thread::Builder::new()
                    .name("koden-brain-librarian".into())
                    .spawn(move || {
                        let (result, finish) = pending.call();
                        // The worker owns the store; hand the raw result back to it
                        // to reconcile + enqueue on the single writer thread.
                        let _ = tx.send(BrainEvent::LibrarianDone { project: pid, finish, result });
                    }) {
                    Ok(_) => {
                        st.in_flight = true;
                        // Snapshot the pin at dispatch so LibrarianDone can detect a
                        // mid-flight manual re-pin and not clobber it. [LIB-DESIGN-01 miss1]
                        st.in_flight_from = prev.clone();
                    }
                    Err(e) => {
                        // Could not spawn the helper: don't wedge the project in_flight.
                        // Re-arm so a later tick retries; the reservation just placed is
                        // folded by the boot sweep if this attempt never completes (it
                        // still counts against the ceiling meanwhile, never under-charges).
                        log::warn!("brain: librarian offload spawn failed ({e}); round deferred");
                        st.dirty = true;
                    }
                }
            }
        }
    }
}

/// Log one autonomous round's outcome with the paid-retry stance — shared by the
/// inline `Ready` path and the offloaded `LibrarianDone` path.
fn log_round_outcome(project_id: &str, outcome: &reflect::ReflectOutcome, st: &LibrarianAuto) {
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
        // ADR-018: autonomous mode lands the seeded findings right away. The
        // returned pin is dropped — this runs before `lib_state` exists, and
        // `lib_entry` hydrates from the DURABLE pin the sweep already wrote on the
        // first sighting of the project this boot. [LIB-SPEND-01]
        let _ = auto_apply_sweep(app, index, &proj.id, now_ms);
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
/// boundary on an `exited` signal. ADR-020: `started`/`exited` also land as
/// crash-safe `start`/`end` rows in the activity trail (single writer — this runs
/// on the worker thread).
fn handle_agent(
    app: &AppHandle,
    index: &SqliteIndex,
    tx: &mpsc::Sender<BrainEvent>,
    pty_id: u32,
    kind: &str,
    agent: Option<String>,
) -> Option<String> {
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
    // ADR-021: a cwd resolving nowhere registers its nearest qualifying ancestor
    // under the workspace root and retries, so this very signal is not lost.
    let project = cwd.as_deref().and_then(|c| resolve_or_register_cwd(app, tx, c)).map(|p| p.id);
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

    // ADR-020: session boundaries land in the activity trail (incremental,
    // crash-safe — an app crash leaves the `start` row as the trail's evidence).
    if let Some(p) = &resolved {
        record_session_boundary(index, p, kind, effective_agent.as_deref(), pty_id, now_epoch_ms());
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

    /// ADR-020 fixup: the files-touched trail records REAL workspace changes
    /// only — the brain's own derived artifact and denylisted paths ride the
    /// same coalesced watcher batch as genuine edits (an apply writes notes,
    /// then refreshes the artifact) and must never appear as "activity".
    ///
    /// These gates now live UPSTREAM: the payload is built from the paths
    /// `index_changed_accepted` returned, so the exclusion is structural. The
    /// end-to-end version of this assertion is
    /// `activity_trail_matches_what_the_indexer_accepted` below.
    #[test]
    fn files_activity_payload_keeps_accepted_rels() {
        let payload = files_activity_payload(&["src/main.rs".to_string()]);
        assert!(payload.contains("main.rs"), "real edit kept: {payload}");
    }

    /// The bug this pairs with: `files_activity_payload` used to re-derive its
    /// own gate set from the RAW watcher batch and drifted from the indexer's,
    /// missing `rel_under_skip_dir` (which holds `.git`) and the gitignore gate.
    /// A `git commit` then fanned `.git/index.lock` into the injected gist, so
    /// "what did we last work on?" answered with git plumbing.
    ///
    /// Locks the invariant structurally: the trail names a path IFF the indexer
    /// accepted it. Fails on the old behavior, which listed all six.
    #[test]
    fn activity_trail_matches_what_the_indexer_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "out/\n").unwrap();
        for d in ["src", "out", ".git", "node_modules/pkg"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("src/main.rs"), b"fn main() {}").unwrap();
        std::fs::write(root.join("out/bundle.js"), b"x").unwrap();
        std::fs::write(root.join(".git/index.lock"), b"").unwrap();
        std::fs::write(root.join(".git/ORIG_HEAD"), b"deadbeef").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), b"x").unwrap();

        let index = SqliteIndex::open(&dir.path().join("i.sqlite")).unwrap();
        let changed: Vec<std::path::PathBuf> = [
            "src/main.rs",
            "out/bundle.js",
            ".git/index.lock",
            ".git/ORIG_HEAD",
            "node_modules/pkg/index.js",
        ]
        .iter()
        .map(|p| root.join(p))
        .collect();

        let (_stats, accepted) = index_changed_accepted(&index, "p", root, &changed);
        let payload = files_activity_payload(&accepted);

        assert!(payload.contains("src/main.rs"), "real edit kept: {payload}");
        assert!(!payload.contains(".git"), "git internals excluded: {payload}");
        assert!(
            !payload.contains("node_modules"),
            "skip-dir excluded: {payload}"
        );
        assert!(
            !payload.contains("bundle.js"),
            "gitignored excluded: {payload}"
        );
    }

    /// Observed live: atomic writers leave `x.md.tmp.<pid>.<hash>` droppings that
    /// the watcher batches alongside the real rename target — the trail must
    /// list the file, never its transient spelling.
    #[test]
    fn files_activity_payload_excludes_transient_writes() {
        let accepted: Vec<String> = [
            "docs/ADR-021.md",
            "docs/ADR-021.md.tmp.41168.983e1e3a",
            "notes/draft.md~",
            "notes/.#lock.md",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let payload = files_activity_payload(&accepted);
        assert!(payload.contains("ADR-021.md"), "real file kept: {payload}");
        assert!(!payload.contains(".tmp."), "atomic temp excluded: {payload}");
        assert!(!payload.contains("~"), "editor backup excluded: {payload}");
        assert!(!payload.contains(".#"), "editor lock excluded: {payload}");
    }

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

    /// LIB-DESIGN-01 miss1: the offloaded LibrarianDone fold must NOT clobber a pin a
    /// manual Reflect set while the round's provider call was in flight. `expected_pin`
    /// = the pin snapshotted at dispatch; if `digest_hash` moved off it meanwhile, a
    /// mid-flight manual round already paid+pinned a newer digest — keep it.
    #[test]
    fn fold_offloaded_outcome_cas_guards_mid_flight_manual_pin() {
        use reflect::ReflectReason as R;

        // No mid-flight re-pin (the common case): behaves exactly like apply_round_outcome.
        // Dispatched with pin = None (never reflected); round reflected on hA.
        let mut st = LibrarianAuto { digest_hash: None, ..Default::default() };
        fold_offloaded_outcome(&mut st, None, &R::Ok, Some("hA".into()));
        assert_eq!(st.digest_hash.as_deref(), Some("hA"), "no re-pin ⇒ this round's hash folds in");
        assert_eq!(st.fail_streak, 0);

        // Mid-flight manual re-pin: dispatched with pin = None, but a manual Reflect
        // pinned hB before LibrarianDone landed. Folding this round's (older) hA must
        // NOT clobber hB — otherwise the next auto round re-pays the already-paid hB.
        let mut st = LibrarianAuto { digest_hash: Some("hB".into()), fail_streak: 3, ..Default::default() };
        fold_offloaded_outcome(&mut st, None, &R::Ok, Some("hA".into()));
        assert_eq!(st.digest_hash.as_deref(), Some("hB"), "manual pin hB preserved, NOT clobbered by hA");
        assert_eq!(st.fail_streak, 0, "the paid round still succeeded ⇒ streak resets");

        // Re-pin with a PAID-but-rejected outcome (InvalidOutput) likewise keeps hB
        // and still counts the failure — only the pin write is guarded.
        let mut st = LibrarianAuto { digest_hash: Some("hB".into()), ..Default::default() };
        fold_offloaded_outcome(&mut st, None, &R::InvalidOutput, Some("hA".into()));
        assert_eq!(st.digest_hash.as_deref(), Some("hB"), "InvalidOutput must not clobber the manual pin either");
        assert_eq!(st.fail_streak, 1);

        // CallFailed never writes the pin at all; the guard is a no-op on it.
        let mut st = LibrarianAuto { digest_hash: Some("hB".into()), ..Default::default() };
        fold_offloaded_outcome(&mut st, Some("hB".into()), &R::CallFailed("x".into()), None);
        assert_eq!(st.digest_hash.as_deref(), Some("hB"));
        assert_eq!(st.fail_streak, 1);
    }

    /// LIB-SPEND-01: the delta-gate pin must be durable. `lib_entry` hydrates a
    /// fresh in-memory entry from the persisted store (so a restart doesn't re-pay a
    /// byte-identical digest), and `persist_lib_pin` writes a settled pin through —
    /// but never persists a still-`None` pin.
    #[test]
    fn lib_entry_hydrates_persisted_pin_and_persist_roundtrips() {
        let dir = std::env::temp_dir().join(format!("koden-libpin-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = dir.join("index.sqlite");
        let idx = SqliteIndex::open_with_recovery(&db).expect("open store");

        // Fresh project: lib_entry hydrates to None (this project has never reflected).
        let mut state: std::collections::HashMap<String, LibrarianAuto> = std::collections::HashMap::new();
        assert_eq!(lib_entry(&mut state, &idx, "p").digest_hash, None, "no persisted pin yet");

        // A round settles on a digest; persist_lib_pin writes it through to the store.
        state.get_mut("p").unwrap().digest_hash = Some("deadbeef".into());
        persist_lib_pin(&idx, "p", state.get("p").unwrap(), 42);
        assert_eq!(idx.librarian_pin("p").as_deref(), Some("deadbeef"), "pin persisted");

        // Simulate a restart: drop the map; a new entry must hydrate from the store.
        let mut restarted: std::collections::HashMap<String, LibrarianAuto> = std::collections::HashMap::new();
        assert_eq!(
            lib_entry(&mut restarted, &idx, "p").digest_hash.as_deref(),
            Some("deadbeef"),
            "the pin survives the in-memory reset (the whole point)"
        );

        // A still-None pin (a project seen but not yet reflected) is never written.
        persist_lib_pin(&idx, "q", &LibrarianAuto::default(), 43);
        assert_eq!(idx.librarian_pin("q"), None, "a None pin is not persisted");

        let _ = std::fs::remove_dir_all(&dir);
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

        index_changed(&index, "p", root, std::slice::from_ref(&sub));
        let paths = index.existing_paths("p").unwrap();
        assert!(paths.contains(&"sub/kept.txt".to_string()), "non-ignored child is indexed");
        assert!(
            !paths.contains(&"sub/zz.txt".to_string()),
            ".kodenignore'd child must not enter the index even when the subtree walk over-yields it"
        );
    }

    /// Parallel first-index determinism: indexing the SAME fixture into two
    /// fresh stores yields identical derived content — fingerprint (path+hash
    /// set), node keys, resolved edges, and search hit order — regardless of
    /// worker completion order (the sequence-numbered apply is the guarantee).
    #[test]
    fn parallel_index_dir_is_deterministic() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path();
        // Enough files to actually fan out over every worker, with real parse
        // targets and an in-project import chain so nodes + edges are exercised.
        // Deliberately > 2×PAR_MAX_LEAD so compute workers cross the lead cap
        // and the wait/notify handshake is exercised (deadlock regression).
        for i in 0..80usize {
            let dir = root.join(format!("m{}", i % 5));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(format!("f{i:02}.ts")),
                format!(
                    "import {{ f{prev:02} }} from './f{prev:02}';\n\
                     export function f{i:02}() {{ return {i}; }}\n",
                    prev = i.saturating_sub(1)
                ),
            )
            .unwrap();
        }
        std::fs::write(root.join("lib.rs"), "pub fn rustSide() -> u8 { 7 }").unwrap();

        let s1 = tempfile::tempdir().unwrap();
        let s2 = tempfile::tempdir().unwrap();
        let i1 = SqliteIndex::open(&s1.path().join("a.sqlite")).unwrap();
        let i2 = SqliteIndex::open(&s2.path().join("b.sqlite")).unwrap();
        let st1 = index_dir(&i1, "p", root);
        let st2 = index_dir(&i2, "p", root);
        assert_eq!(st1.indexed, 81);
        assert_eq!(st2.indexed, 81);
        assert_eq!(
            i1.project_fingerprint("p").unwrap(),
            i2.project_fingerprint("p").unwrap(),
            "two first-index runs over the same tree must fingerprint identically"
        );
        assert_eq!(i1.project_node_keys("p").unwrap(), i2.project_node_keys("p").unwrap());
        assert_eq!(i1.project_edges("p").unwrap(), i2.project_edges("p").unwrap());
        use crate::modules::brain::store::SearchIndex as _;
        let hits = |idx: &SqliteIndex, q: &str| -> Vec<String> {
            idx.search(Some("p"), q, 10).unwrap().into_iter().map(|h| h.path).collect()
        };
        for q in ["f07", "rustside", "return"] {
            assert_eq!(hits(&i1, q), hits(&i2, q), "hit ORDER must match for '{q}'");
        }
        // And a warm second pass over unchanged content is a full no-op set:
        // same fingerprint, nothing pruned (the Unchanged fast-path kept them).
        let warm = index_dir(&i1, "p", root);
        assert_eq!(warm.pruned, 0);
        assert_eq!(
            i1.project_fingerprint("p").unwrap(),
            i2.project_fingerprint("p").unwrap(),
            "warm pass must not change derived state"
        );
    }

    /// One bad file in a parallel batch (vanished before read / binary) must not
    /// poison its neighbors: every readable text file still indexes, the binary
    /// is excluded, and only positive absence feeds reconcile.
    #[test]
    fn error_file_in_batch_does_not_poison_others() {
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        let work = tempfile::tempdir().unwrap();
        let root = work.path();
        for i in 0..10usize {
            std::fs::write(root.join(format!("ok{i}.ts")), format!("export const k{i} = {i};"))
                .unwrap();
        }
        std::fs::write(root.join("blob.bin"), b"bin\x00ary").unwrap();
        // A path the walk "saw" that is gone by read time → open() NotFound →
        // Absent — positive evidence, safe to (not) index; must not poison others.
        let ghost = root.join("ghost.ts");
        let walked = walk::Walked {
            files: {
                let mut v: Vec<std::path::PathBuf> = (0..10)
                    .map(|i| root.join(format!("ok{i}.ts")))
                    .collect();
                v.push(root.join("blob.bin"));
                v.push(ghost.clone());
                v
            },
            complete: true,
        };
        let stats = index_walked(&index, "p", root, walked);
        assert_eq!(stats.indexed, 10, "all 10 good files index despite bad batch-mates");
        let paths = index.existing_paths("p").unwrap();
        assert_eq!(paths.len(), 10);
        assert!(!paths.contains(&"blob.bin".to_string()), "binary excluded");
        assert!(!paths.contains(&"ghost.ts".to_string()), "absent file never indexed");
    }

    /// Synthetic ~2k-file first-index wall-clock probe (debug build; relative
    /// numbers only — the release SLA re-measure lives in the sweep step). Run:
    /// `cargo test --lib bench_first_index_2k -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_first_index_2k() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path();
        for i in 0..2000usize {
            let dir = root.join(format!("mod{:02}", i % 40));
            std::fs::create_dir_all(&dir).unwrap();
            // Realistic-ish TS: defs to parse, an in-project import, ~1 KB body.
            let body = format!(
                "import {{ helper{prev} }} from './file{prev:04}';\n\
                 export interface Shape{i} {{ id: number; label: string; }}\n\
                 export function helper{i}(input: Shape{i}): string {{\n\
                     const normalized = input.label.trim().toLowerCase();\n\
                     return `${{input.id}}:${{normalized}}`;\n\
                 }}\n\
                 export class Service{i} {{\n\
                     private cache = new Map<number, string>();\n\
                     resolve(shape: Shape{i}): string {{\n\
                         const hit = this.cache.get(shape.id);\n\
                         if (hit !== undefined) return hit;\n\
                         const out = helper{i}(shape);\n\
                         this.cache.set(shape.id, out);\n\
                         return out;\n\
                     }}\n\
                 }}\n\
                 // filler: {filler}\n",
                prev = i.saturating_sub(1),
                i = i,
                filler = "lorem ipsum dolor sit amet consectetur adipiscing elit ".repeat(8),
            );
            std::fs::write(dir.join(format!("file{i:04}.ts")), body).unwrap();
        }
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        let t0 = std::time::Instant::now();
        let stats = index_dir(&index, "bench", root);
        let elapsed = t0.elapsed();
        println!("bench_first_index_2k: indexed {} files in {elapsed:?}", stats.indexed);
        assert_eq!(stats.indexed, 2000);
    }

    /// ADR-020 ingest gate: trivial turns drop (empty / one-char / slash-command-
    /// only), long prompts truncate on a char boundary, and secret-shaped content
    /// is REDACTED at ingest — the stored payload never carries the raw token.
    #[test]
    fn clean_turn_text_filters_truncates_and_redacts() {
        // Drops: empty, whitespace, single keystroke, slash-command-only.
        for junk in ["", "   ", "y", "/clear", "/model opus", "  /compact  "] {
            assert_eq!(clean_turn_text(junk), None, "must drop {junk:?}");
        }
        // Kept: a real prompt; a multi-line prompt that merely starts with '/'.
        assert_eq!(clean_turn_text("fix the login bug").as_deref(), Some("fix the login bug"));
        assert!(clean_turn_text("/path/to/file is broken\nfix it").is_some());
        // Truncation: bounded at TURN_MAX_CHARS chars (multi-byte safe).
        let long = "é".repeat(TURN_MAX_CHARS + 100);
        let cut = clean_turn_text(&long).unwrap();
        assert_eq!(cut.chars().count(), TURN_MAX_CHARS);
        // Redaction property: an API-key-shaped token and a PEM header never
        // reach the stored payload.
        let probe = "sk-ProbeEcho991Zx8Kt5Rm7Vb4Np2Cj6L";
        let stored = clean_turn_text(&format!("use {probe} for the call")).unwrap();
        assert!(!stored.contains(probe), "API key leaked into activity: {stored}");
        assert!(stored.contains("REDACTED"), "redaction marker missing: {stored}");
        let pem = "here is the key\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA7\n-----END RSA PRIVATE KEY-----";
        let stored = clean_turn_text(pem).unwrap();
        assert!(!stored.contains("MIIEowIBAAKCAQEA7"), "PEM body leaked: {stored}");
        // Deterministic (the reflect send path re-renders it).
        assert_eq!(clean_turn_text(pem), clean_turn_text(pem));
    }

    /// ADR-020 coalescing seam: one payload per batch with the batch COUNT —
    /// never per-proposal. No-op applied/reverted batches are suppressed; a
    /// completed reflect round is activity even at 0 proposals (it spent a call).
    #[test]
    fn build_activity_event_coalesces_batches() {
        let ev = build_activity_event("p", "Proj", "applied", 3, None)
            .expect("a 3-apply sweep emits exactly one event");
        assert_eq!((ev.kind.as_str(), ev.count), ("applied", 3));
        assert_eq!(ev.project_name, "Proj");
        assert!(ev.spent_usd.is_none());
        assert!(build_activity_event("p", "Proj", "applied", 0, None).is_none(), "no-op sweep is silent");
        assert!(build_activity_event("p", "Proj", "reverted", 0, None).is_none());
        let r = build_activity_event("p", "Proj", "reflected", 0, Some(0.0021)).expect("paid round");
        assert_eq!(r.count, 0);
        assert_eq!(r.spent_usd, Some(0.0021));
    }

    /// ADR-020 boundary rows: `started`/`exited` land as start/end activity rows
    /// (status-only kinds don't), and an exit enqueues a TARGETED rescan of the
    /// session's project.
    #[test]
    fn session_boundaries_write_rows_and_exit_enqueues_targeted_rescan() {
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        record_session_boundary(&index, "p", "started", Some("claude"), 7, 1_000);
        record_session_boundary(&index, "p", "working", Some("claude"), 7, 2_000); // status noise
        record_session_boundary(&index, "p", "exited", Some("claude"), 7, 3_000);
        let rows = index.recent_activity("p", 10).unwrap();
        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, vec!["end", "start"], "newest first; working never lands");
        assert!(rows.iter().all(|r| r.payload_redacted == "claude"));

        let (tx, rx) = mpsc::channel::<BrainEvent>();
        enqueue_exit_reconcile(&tx, "p");
        match rx.try_recv().expect("one event enqueued") {
            BrainEvent::Rescan { project } => assert_eq!(project.as_deref(), Some("p")),
            other => panic!("expected a targeted Rescan, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "exactly one event per exit");
    }

    /// ADR-020 retention: the per-project cap drops oldest rows, the TTL drops
    /// aged rows, and other projects' rows are untouched.
    #[test]
    fn prune_activity_caps_and_ttls_per_project() {
        let store = tempfile::tempdir().unwrap();
        let index = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
        for i in 0..10i64 {
            index.record_activity("p", None, "turn", &format!("t{i}"), 1_000 + i).unwrap();
        }
        index.record_activity("q", None, "turn", "other-project", 1_000).unwrap();
        // Cap: keep the newest 4 of p's 10.
        let dropped = index.prune_activity("p", 4, i64::MAX, 2_000).unwrap();
        assert_eq!(dropped, 6);
        let rows = index.recent_activity("p", 50).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.payload_redacted.as_str()).collect::<Vec<_>>(),
            vec!["t9", "t8", "t7", "t6"],
            "oldest dropped, newest kept"
        );
        // TTL: everything older than (now - ttl) goes; t9 (ts 1009) survives a
        // cutoff of 1009, the rest don't.
        let dropped = index.prune_activity("p", 500, 991, 2_000).unwrap();
        assert_eq!(dropped, 3);
        let rows = index.recent_activity("p", 50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].payload_redacted, "t9");
        // The sibling project is untouched by p's prunes.
        assert_eq!(index.recent_activity("q", 50).unwrap().len(), 1);
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

    /// ADR-021: the single-dir qualification shared by discovery and the
    /// first-use walk — a git repo or a manifest qualifies; a plain dir, a
    /// file, and an ignored-named dir (even one CARRYING a manifest, the
    /// node_modules shape) do not.
    #[test]
    fn qualifies_as_project_markers_and_ignored_names() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path();

        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(!qualifies_as_project(&plain), "no marker → not a project");

        let git = root.join("gitproj");
        std::fs::create_dir_all(git.join(".git")).unwrap();
        assert!(qualifies_as_project(&git), "a git repo qualifies");

        let manifest = root.join("nodeproj");
        std::fs::create_dir_all(&manifest).unwrap();
        std::fs::write(manifest.join("package.json"), "{}").unwrap();
        assert!(qualifies_as_project(&manifest), "a manifest qualifies");

        let nm = root.join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("package.json"), "{}").unwrap();
        assert!(!qualifies_as_project(&nm), "an ignored-named dir never qualifies");

        let file = root.join("gitproj").join("README.md");
        std::fs::write(&file, "x").unwrap();
        assert!(!qualifies_as_project(&file), "a file never qualifies");
    }

    /// ADR-021 nearest-ancestor rule, defined precisely: walking UP from cwd,
    /// the FIRST qualifying dir STRICTLY below the workspace root wins — a
    /// nested-git-in-git cwd picks the INNER repo; the root itself is never a
    /// candidate; a cwd outside the root yields nothing.
    #[test]
    fn first_use_candidate_picks_nearest_qualifying_ancestor_below_root() {
        let work = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(work.path()).unwrap();
        let outer = root.join("outer");
        let inner = outer.join("inner");
        let src = inner.join("src");
        std::fs::create_dir_all(outer.join(".git")).unwrap();
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        std::fs::create_dir_all(&src).unwrap();

        // Nested git-in-git: the dir nearest to cwd wins.
        assert_eq!(first_use_candidate(&root, &src).as_deref(), Some(inner.as_path()));
        // cwd above the inner repo resolves to the outer one.
        let outer_docs = outer.join("docs");
        std::fs::create_dir_all(&outer_docs).unwrap();
        assert_eq!(first_use_candidate(&root, &outer_docs).as_deref(), Some(outer.as_path()));
        // cwd AT a project root registers that root.
        assert_eq!(first_use_candidate(&root, &inner).as_deref(), Some(inner.as_path()));
        // The workspace root itself is NEVER a candidate…
        assert_eq!(first_use_candidate(&root, &root), None);
        // …even when the root itself is a git repo and cwd is a plain child.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(first_use_candidate(&root, &plain), None, "no ancestor strictly below root");
        // A cwd OUTSIDE the workspace root yields nothing.
        let elsewhere = tempfile::tempdir().unwrap();
        let far = std::fs::canonicalize(elsewhere.path()).unwrap().join("x");
        std::fs::create_dir_all(far.join(".git")).unwrap();
        assert_eq!(first_use_candidate(&root, &far), None);
    }

    /// ADR-021: every npm dependency carries a package.json — a cwd inside
    /// node_modules must attribute to the REAL project above it, never register
    /// the dependency dir.
    #[test]
    fn first_use_candidate_skips_ignored_subtrees() {
        let work = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(work.path()).unwrap();
        let proj = root.join("proj");
        let dep = proj.join("node_modules").join("dep");
        std::fs::create_dir_all(proj.join(".git")).unwrap();
        std::fs::create_dir_all(&dep).unwrap();
        std::fs::write(dep.join("package.json"), "{}").unwrap();
        assert_eq!(first_use_candidate(&root, &dep).as_deref(), Some(proj.as_path()));
    }

    /// ADR-021 first-use path: an unresolvable cwd under the workspace root
    /// registers its project ONCE and the retried resolution succeeds — the
    /// triggering signal/turn is not lost; the second trigger resolves normally
    /// without a second registration; the workspace root itself never registers;
    /// no workspace root → no registration at all.
    #[test]
    fn resolve_or_register_registers_once_and_retries_resolution() {
        let work = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(work.path()).unwrap();
        let proj = root.join("proj");
        let src = proj.join("src");
        std::fs::create_dir_all(proj.join(".git")).unwrap();
        std::fs::create_dir_all(&src).unwrap();

        // No workspace root configured → a miss stays a miss.
        let reg = KodenBrainRegistry::default();
        let cwd = src.to_string_lossy().to_string();
        assert!(resolve_or_register_project(&reg, &cwd).is_none());
        assert!(reg.projects().is_empty());

        // Workspace root set: the first trigger registers AND resolves.
        reg.set_workspace_root(Some(to_canon(&root).trim_end_matches('/').to_string()));
        let (p1, newly) = resolve_or_register_project(&reg, &cwd).expect("registers + resolves");
        assert!(newly, "first trigger performs the registration");
        assert_eq!(p1.name, "proj");
        assert_eq!(reg.projects().len(), 1);

        // Debounce: the second trigger (started signal + first turn arrive close
        // together) resolves normally — same project, no second registration.
        let (p2, newly2) = resolve_or_register_project(&reg, &cwd).expect("resolves");
        assert!(!newly2, "second trigger must not re-register");
        assert_eq!(p2.id, p1.id);
        assert_eq!(reg.projects().len(), 1);

        // The workspace root itself is never registered, even as a git repo.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let root_cwd = root.to_string_lossy().to_string();
        assert!(resolve_or_register_project(&reg, &root_cwd).is_none());
        assert_eq!(reg.projects().len(), 1, "root trigger must not add a project");
    }

    /// ADR-021 boot re-discovery: the shared brain_set_workspace loop registers
    /// only qualifying children, is idempotent across a double boot, and reports
    /// how many were NEW (the boot log gate).
    #[test]
    fn register_workspace_children_is_idempotent_across_boots() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path();
        std::fs::create_dir_all(root.join("a").join(".git")).unwrap();
        let b = root.join("b");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("Cargo.toml"), "[package]").unwrap();
        std::fs::create_dir_all(root.join("plain")).unwrap(); // no marker

        let reg = KodenBrainRegistry::default();
        let (children, added) = register_workspace_children(&reg, root);
        assert_eq!(added, 2, "both qualifying children are NEW on first boot");
        assert_eq!(children.len(), 2);
        assert_eq!(reg.projects().len(), 2);

        // Second boot: same children returned, nothing newly added.
        let (children2, added2) = register_workspace_children(&reg, root);
        assert_eq!(added2, 0, "double boot registers nothing new");
        assert_eq!(children2.len(), 2);
        assert_eq!(reg.projects().len(), 2);
    }

    /// A user's confirmed remove must survive the auto paths: boot re-discovery
    /// (the shared child scan) and first-use registration both skip a tombstoned
    /// project even though its dir still qualifies on disk; the explicit
    /// `brain_add_project` path (`add_root`) is the opt-back-in.
    #[test]
    fn removed_project_survives_boot_rediscovery_and_first_use() {
        let work = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(work.path()).unwrap();
        let proj = root.join("proj");
        let src = proj.join("src");
        std::fs::create_dir_all(proj.join(".git")).unwrap();
        std::fs::create_dir_all(&src).unwrap();

        let reg = KodenBrainRegistry::default();
        reg.set_workspace_root(Some(to_canon(&root).trim_end_matches('/').to_string()));
        let (_, added) = register_workspace_children(&reg, &root);
        assert_eq!(added, 1);
        let id = reg.projects()[0].id.clone();

        // The user removes it (brain_remove_project → registry.remove).
        reg.remove(&id).expect("removed");

        // Boot re-discovery (next launch, .git still on disk): stays removed.
        let (children, added) = register_workspace_children(&reg, &root);
        assert_eq!(added, 0, "boot re-discovery must not resurrect a removed project");
        assert!(children.is_empty());
        assert!(reg.projects().is_empty());

        // First-use registration (agent signal/turn in that dir): stays removed.
        let cwd = src.to_string_lossy().to_string();
        assert!(
            resolve_or_register_project(&reg, &cwd).is_none(),
            "first-use must not resurrect a removed project"
        );
        assert!(reg.projects().is_empty());

        // Explicit re-add opts back in; discovery works normally again.
        reg.add_root(&proj).expect("explicit re-add");
        assert_eq!(reg.projects().len(), 1);
        let (_, newly) = resolve_or_register_project(&reg, &cwd).expect("resolves again");
        assert!(!newly);
    }
}
