mod agent_detect;
mod da_filter;
#[cfg(windows)]
mod job;
mod retry_detect;
mod session;
pub(crate) mod shell_init;
mod shell_ssh;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::TrySendError;
use std::sync::{Arc, RwLock};
use std::thread;

use portable_pty::PtySize;
use tauri::ipc::{Channel, Response};

use crate::modules::workspace::{authorize_user_spawn_cwd, WorkspaceEnv, WorkspaceRegistry};
use session::Session;

pub struct PtyState {
    sessions: RwLock<HashMap<u32, Arc<Session>>>,
    // Starts at 1 so freshly-handed-out ids are never 0, which the frontend
    // sometimes treats as "unset". Increments monotonically; never reused.
    next_id: AtomicU32,
}

impl Default for PtyState {
    fn default() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            next_id: AtomicU32::new(1),
        }
    }
}

impl PtyState {
    /// The spawn cwd of a live session, if known. The Brain uses this to resolve
    /// a pty leaf → project (longest-prefix match against registered roots).
    /// Returns `None` for unknown/closed sessions or sessions opened without a cwd.
    pub fn session_cwd(&self, id: u32) -> Option<String> {
        self.sessions.read().ok()?.get(&id).and_then(|s| s.cwd.clone())
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn pty_open(
    app: tauri::AppHandle,
    state: tauri::State<'_, PtyState>,
    registry: tauri::State<'_, WorkspaceRegistry>,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
    workspace: Option<WorkspaceEnv>,
    blocks: Option<bool>,
    ssh_tmux: Option<String>,
    ssh_tmux_window: Option<String>,
    on_data: Channel<Response>,
    on_exit: Channel<i32>,
) -> Result<u32, String> {
    let workspace = WorkspaceEnv::from_option(workspace);
    let blocks = blocks.unwrap_or(false);
    authorize_user_spawn_cwd(&registry, cwd.as_deref(), &workspace).map_err(|e| {
        log::warn!("pty_open: cwd rejected: {e}");
        e
    })?;
    let id = state.next_id.fetch_add(1, Ordering::Relaxed);
    let session = tauri::async_runtime::spawn_blocking(move || {
        session::spawn(
            id,
            app,
            cols,
            rows,
            cwd,
            workspace,
            blocks,
            ssh_tmux,
            ssh_tmux_window,
            on_data,
            on_exit,
        )
        .map(|(s, _)| s)
    })
    .await
    .map_err(|e| {
        log::error!("pty_open join failed: {e}");
        e.to_string()
    })?
    .map_err(|e| {
        log::error!("pty_open failed: {e}");
        e
    })?;
    state.sessions.write().unwrap().insert(id, session);
    log::info!("pty opened id={id} cols={cols} rows={rows}");
    Ok(id)
}

// Input is the latency-critical path: raw body + id header skips JSON
// serialization of every keystroke on both sides of the IPC boundary.
// Deliberately sync (ordered, no async dispatch) but non-blocking: the actual
// PTY write happens on the session's writer thread. Sync commands run on the
// main thread, and a `write_all` against a wedged conhost here used to freeze
// keyboard input app-wide (2026-08-31 ssh/ai-server incident).
#[tauri::command]
pub fn pty_write(
    state: tauri::State<PtyState>,
    request: tauri::ipc::Request,
) -> Result<(), String> {
    let id: u32 = request
        .headers()
        .get("x-pty-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "pty_write: missing x-pty-id header".to_string())?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("pty_write: expected raw body".to_string());
    };
    let session = state
        .sessions
        .read()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            log::warn!("pty_write: unknown id={id}");
            "no session".to_string()
        })?;
    match session.input_tx.try_send(bytes.to_vec()) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            // Only reachable when the writer thread is stuck (wedged conhost,
            // stalled ssh): drop the chunk instead of blocking the caller.
            log::warn!("pty_write id={id}: input queue full, dropped {} bytes", bytes.len());
            Err("pty input backlogged".to_string())
        }
        Err(TrySendError::Disconnected(_)) => {
            // Expected if the child already exited (the old EPIPE case).
            log::debug!("pty_write id={id}: writer thread gone");
            Err("pty writer closed".to_string())
        }
    }
}

// Async: ResizePseudoConsole is a blocking RPC into conhost and must never
// run on the main thread — a wedged conhost would take all IPC down with it.
#[tauri::command]
pub async fn pty_resize(
    state: tauri::State<'_, PtyState>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let session = state
        .sessions
        .read()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            log::warn!("pty_resize: unknown id={id}");
            "no session".to_string()
        })?;
    tauri::async_runtime::spawn_blocking(move || {
        session
            .master
            .lock()
            .unwrap()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| {
                log::warn!("pty_resize id={id} failed: {e}");
                e.to_string()
            })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn pty_close(state: tauri::State<PtyState>, id: u32) -> Result<(), String> {
    let session = state.sessions.write().unwrap().remove(&id);
    if let Some(s) = session {
        // Detached: kill and ClosePseudoConsole can both block on a wedged
        // conhost, and this sync command runs on the main thread.
        thread::Builder::new()
            .name(format!("koden-pty-drop-{id}"))
            .spawn(move || {
                if let Err(e) = s.killer.lock().unwrap().kill() {
                    // Non-fatal: the child may already have exited on its own
                    // (e.g. the user ran `exit`). Log so this isn't invisible.
                    log::debug!("pty_close: kill id={id} returned {e}");
                }
                log::info!("pty closed id={id}");
                let t0 = std::time::Instant::now();
                session::drop_session(s);
                log::info!(
                    "pty session id={id} dropped in {}ms",
                    t0.elapsed().as_millis()
                );
            })
            .expect("spawn pty drop thread");
    } else {
        log::debug!("pty_close: unknown id={id}");
    }
    Ok(())
}

// Async: pgrep (unix) / a Toolhelp snapshot (Windows) are syscalls that don't
// belong on the main thread.
#[tauri::command]
pub async fn pty_has_foreground_process(
    state: tauri::State<'_, PtyState>,
    id: u32,
) -> Result<bool, String> {
    let shell_pid = {
        let sessions = state.sessions.read().unwrap();
        let session = sessions.get(&id).ok_or_else(|| {
            log::warn!("pty_has_foreground_process: unknown session id={id}");
            "no session".to_string()
        })?;
        session.shell_pid
    };
    if shell_pid == 0 {
        return Ok(false);
    }
    tauri::async_runtime::spawn_blocking(move || shell_has_children(shell_pid))
        .await
        .map_err(|e| e.to_string())
}

// Foreground-only check for the renderer hibernation path: true while a job
// owns the tty (tcgetpgrp != shell pgid). Stricter and cheaper than
// pty_has_foreground_process, which counts background children too.
// Async: the unix branch takes `master.lock()`, which a stuck resize against
// a wedged pty could hold — never contend for it on the main thread.
#[tauri::command]
pub async fn pty_has_foreground_job(
    state: tauri::State<'_, PtyState>,
    id: u32,
) -> Result<bool, String> {
    let session = state
        .sessions
        .read()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            log::warn!("pty_has_foreground_job: unknown session id={id}");
            "no session".to_string()
        })?;
    let shell_pid = session.shell_pid;
    if shell_pid == 0 {
        return Ok(false);
    }
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(unix)]
        {
            let leader = session.master.lock().unwrap().process_group_leader();
            matches!(leader, Some(pid) if pid > 0 && pid as u32 != shell_pid)
        }
        #[cfg(windows)]
        {
            let _ = &session;
            shell_has_children(shell_pid)
        }
    })
    .await
    .map_err(|e| e.to_string())
}

// pgrep -P exits 0 when shell_pid has at least one child, 1 when none.
#[cfg(unix)]
fn shell_has_children(shell_pid: u32) -> bool {
    std::process::Command::new("pgrep")
        .args(["-P", &shell_pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn shell_has_children(shell_pid: u32) -> bool {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32 = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32>() as u32;
        let mut found = false;
        if Process32First(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ParentProcessID == shell_pid {
                    found = true;
                    break;
                }
                if Process32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        found
    }
}

// A fresh webview load orphans the previous frontend's sessions in this still
// running process; reap them on boot before any new tab spawns.
#[tauri::command]
pub fn pty_close_all(state: tauri::State<PtyState>) -> Result<usize, String> {
    let drained: Vec<(u32, Arc<Session>)> = {
        let mut sessions = state.sessions.write().unwrap();
        sessions.drain().collect()
    };
    let count = drained.len();
    for (id, s) in drained {
        thread::Builder::new()
            .name(format!("koden-pty-drop-{id}"))
            .spawn(move || {
                if let Err(e) = s.killer.lock().unwrap().kill() {
                    log::debug!("pty_close_all: kill id={id} returned {e}");
                }
                session::drop_session(s)
            })
            .expect("spawn pty drop thread");
    }
    if count > 0 {
        log::info!("pty_close_all: reaped {count} orphaned session(s)");
    }
    Ok(count)
}

#[tauri::command]
pub fn pty_shell_name() -> String {
    shell_init::detect_shell_name()
}
