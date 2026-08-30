//! `koden` CLI: a local socket the running instance serves, so a coding agent
//! inside a Koden terminal can read panes, type into them, open tabs and
//! notify the user. The client lives in this same executable as `koden cli`
//! (short-circuited in `main.rs`); no second `[[bin]]`.
//!
//! Flow: `koden cli` -> socket/pipe -> [`server`] thread -> [`bridge`] parks
//! the connection -> Tauri event `koden:cli-request` -> the webview's
//! CliBridge dispatches and invokes [`cli_reply`] -> the connection answers.

pub mod args;
pub mod bridge;
pub mod client;
pub mod protocol;
pub mod render;
pub mod server;
#[cfg(windows)]
pub mod win;

use std::sync::{Arc, OnceLock};

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use bridge::Pending;
use protocol::{Forwarded, Reply, REQUEST_EVENT};

pub use protocol::{ENV_ENDPOINT, ENV_EXE, ENV_TOKEN};

struct Planted {
    endpoint: String,
    token: String,
}

static PLANTED: OnceLock<Planted> = OnceLock::new();

/// Set only once the listener is up, so a PTY never carries a token that
/// points at nothing.
pub fn env_for_pty() -> Option<(&'static str, &'static str)> {
    PLANTED
        .get()
        .map(|p| (p.endpoint.as_str(), p.token.as_str()))
}

/// Path a shell function can exec to reach `koden cli`. Inside an AppImage the
/// outer runtime is re-run (it mounts and sets the library path); elsewhere
/// the current executable.
pub fn exe_for_pty() -> Option<String> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        let p = std::path::PathBuf::from(appimage);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

pub struct CliState {
    pending: Arc<Pending>,
}

/// Called from `lib.rs` setup. Always manages [`CliState`] so `cli_reply`
/// cannot panic on a missing state; the listener itself is fail-open.
pub fn start(app: &AppHandle) {
    let pending = Arc::new(Pending::default());
    app.manage(CliState {
        pending: pending.clone(),
    });
    let token = protocol::generate_token();
    let handle = app.clone();
    let forward: server::Forward = Box::new(move |f: Forwarded| {
        handle
            .emit_to("main", REQUEST_EVENT, f)
            .map_err(|e| e.to_string())
    });
    match server::start(token.clone(), pending, forward) {
        Ok(endpoint) => {
            log::info!("cli: listening on {endpoint}");
            let _ = PLANTED.set(Planted { endpoint, token });
        }
        Err(e) => log::warn!("cli: not available: {e}"),
    }
}

/// The webview's answer to a `koden:cli-request` event.
#[tauri::command]
pub fn cli_reply(
    state: tauri::State<'_, CliState>,
    id: String,
    ok: bool,
    result: Option<Value>,
    error: Option<String>,
) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        return Err("cli_reply: bad id".into());
    }
    let reply = if ok {
        Reply::ok(id, result.unwrap_or(Value::Null))
    } else {
        Reply::err(
            id,
            error
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| "unknown error".into()),
        )
    };
    if !state.pending.complete(reply) {
        log::debug!("cli_reply: no pending request for that id (timed out?)");
    }
    Ok(())
}
