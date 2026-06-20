// Proactive Claude usage poller. Reactive retry detection (pty/retry_detect)
// only fires AFTER a limit banner is already on screen; this module polls the
// OAuth usage endpoint ahead of time so the UI can warn/pause before the agent
// burns the last of the window. Fail-open by design: a poll that errors keeps
// the last-good snapshot and never crashes the host.
//
// SECURITY: the OAuth access/refresh tokens read here are NEVER logged. Only
// derived, non-secret values (percent, reset time, source) leave this module.

pub mod poll;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The default usage window length Claude enforces (the "5-hour" bucket).
pub const WINDOW_MS: i64 = 5 * 60 * 60 * 1000;

/// Fallback CLI version for the mandatory User-Agent when the installed
/// @anthropic-ai/claude-code package.json can't be located at runtime. Without
/// a `claude-code/<version>` UA the endpoint returns a persistent 429.
pub const FALLBACK_CLI_VERSION: &str = "2.1.168";

/// OAuth client id used by the public Claude Code token-refresh flow. Not a
/// secret (it ships in the CLI); the secret is the refresh token, never logged.
pub const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// One read of the usage picture. `percent` is 0..100. `source` distinguishes a
/// live endpoint read ("endpoint") from a time-based estimate ("time").
#[derive(Clone, Debug, Default)]
pub struct UsageSnapshot {
    pub percent: Option<f64>,
    pub reset_epoch_ms: Option<i64>,
    pub telemetry_lost: bool,
    pub source: &'static str,
}

/// Serialized to the frontend over `koden:usage-signal`. camelCase to match the
/// TS lane's event shape.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEvent {
    pub percent_used: Option<f64>,
    pub reset_epoch_ms: Option<i64>,
    /// "warn" or "pause" when a threshold was just crossed, else None.
    pub threshold_crossed: Option<String>,
    pub source: String,
    pub telemetry_lost: bool,
}

impl UsageEvent {
    pub fn from_snapshot(snap: &UsageSnapshot, threshold_crossed: Option<String>) -> Self {
        Self {
            percent_used: snap.percent,
            reset_epoch_ms: snap.reset_epoch_ms,
            threshold_crossed,
            source: snap.source.to_string(),
            telemetry_lost: snap.telemetry_lost,
        }
    }
}

/// OAuth credentials as stored in `~/.claude/.credentials.json`. Only the three
/// fields the refresh flow needs are modeled; the rest is preserved verbatim on
/// write so we never drop scopes/subscriptionType the CLI relies on.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OauthCreds {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(rename = "refreshToken")]
    pub refresh_token: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
}

/// Absolute path of the Claude credentials file. Uses the home dir resolver
/// (dirs crate) rather than a raw env read for cross-platform correctness.
pub fn credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join(".credentials.json"))
}

/// Read and parse the OAuth creds. Returns None when the file is absent or
/// unparseable. NEVER logs token values; on parse failure logs only the error
/// kind, not the contents.
pub fn read_creds() -> Option<OauthCreds> {
    let path = credentials_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(rename = "claudeAiOauth")]
        claude_ai_oauth: OauthCreds,
    }
    match serde_json::from_str::<Wrapper>(&raw) {
        Ok(w) => Some(w.claude_ai_oauth),
        Err(_) => {
            // Do not surface the file contents; a malformed creds file is the
            // caller's problem to fix, and logging it could leak the token.
            log::debug!("usage: credentials.json present but not parseable");
            None
        }
    }
}

/// Persist refreshed tokens back to `.credentials.json` atomically (temp +
/// rename) while preserving every other field already in the file. NEVER logs
/// the values written.
pub fn write_creds(updated: &OauthCreds) -> Result<(), String> {
    let path = credentials_path().ok_or_else(|| "no home dir".to_string())?;
    // Re-read the full object so we only overwrite the three token fields and
    // keep scopes / subscriptionType / rateLimitTier intact.
    let mut root: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let oauth = obj
        .entry("claudeAiOauth")
        .or_insert_with(|| serde_json::json!({}));
    if !oauth.is_object() {
        *oauth = serde_json::json!({});
    }
    let oauth = oauth.as_object_mut().unwrap();
    oauth.insert(
        "accessToken".into(),
        serde_json::Value::String(updated.access_token.clone()),
    );
    oauth.insert(
        "refreshToken".into(),
        serde_json::Value::String(updated.refresh_token.clone()),
    );
    oauth.insert(
        "expiresAt".into(),
        serde_json::Value::Number(updated.expires_at.into()),
    );

    let serialized = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.koden-tmp");
    std::fs::write(&tmp, serialized).map_err(|e| format!("write creds tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename creds: {e}")
    })?;
    Ok(())
}

/// Shared, command-mutable poller config + last snapshot. The poller thread
/// reads `enabled` / thresholds each cycle; commands write them; the snapshot
/// command reads the last result.
pub struct UsageState {
    inner: Mutex<UsageStateInner>,
}

pub struct UsageStateInner {
    pub enabled: bool,
    pub warn_pct: f64,
    pub pause_pct: f64,
    pub last: UsageSnapshot,
    /// Highest threshold already signaled this window, for hysteresis so we
    /// don't re-emit "warn" every poll once crossed. Reset when usage drops
    /// back below the warn line (new window) or the window resets.
    pub signaled: Option<String>,
}

impl Default for UsageState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(UsageStateInner {
                enabled: false,
                warn_pct: 80.0,
                pause_pct: 95.0,
                last: UsageSnapshot::default(),
                signaled: None,
            }),
        }
    }
}

impl UsageState {
    pub fn set_guard(&self, enabled: bool, warn_pct: f64, pause_pct: f64) {
        let mut g = self.inner.lock().unwrap();
        g.enabled = enabled;
        // Clamp to a sane order so pause is never below warn.
        g.warn_pct = warn_pct.clamp(0.0, 100.0);
        g.pause_pct = pause_pct.clamp(g.warn_pct, 100.0);
    }

    pub fn config(&self) -> (bool, f64, f64) {
        let g = self.inner.lock().unwrap();
        (g.enabled, g.warn_pct, g.pause_pct)
    }

    pub fn snapshot(&self) -> UsageSnapshot {
        self.inner.lock().unwrap().last.clone()
    }

    /// Store a fresh snapshot and compute which threshold (if any) was newly
    /// crossed, applying hysteresis so each level fires once per window.
    pub fn record(&self, snap: UsageSnapshot) -> Option<String> {
        let mut g = self.inner.lock().unwrap();
        let crossed = match snap.percent {
            Some(pct) => {
                if pct >= g.pause_pct && g.signaled.as_deref() != Some("pause") {
                    g.signaled = Some("pause".into());
                    Some("pause".to_string())
                } else if pct >= g.warn_pct
                    && g.signaled.is_none()
                {
                    g.signaled = Some("warn".into());
                    Some("warn".to_string())
                } else {
                    // Dropped back below the warn line => new window, re-arm.
                    if pct < g.warn_pct {
                        g.signaled = None;
                    }
                    None
                }
            }
            None => None,
        };
        g.last = snap;
        crossed
    }

    /// On a fresh window (endpoint reports a later reset than we last saw), the
    /// hysteresis latch is cleared so warn/pause can fire again.
    pub fn maybe_reset_window(&self, new_reset_ms: Option<i64>) {
        let mut g = self.inner.lock().unwrap();
        if let (Some(new_r), Some(old_r)) = (new_reset_ms, g.last.reset_epoch_ms) {
            if new_r > old_r + 60_000 {
                g.signaled = None;
            }
        }
    }
}

/// Snapshot returned to the frontend by `usage_guard_snapshot`. camelCase so it
/// drops straight into the TS lane.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshotDto {
    pub enabled: bool,
    pub warn_pct: f64,
    pub pause_pct: f64,
    pub percent_used: Option<f64>,
    pub reset_epoch_ms: Option<i64>,
    pub telemetry_lost: bool,
    pub source: Option<String>,
}

/// Enable/disable the proactive guard and set the warn/pause thresholds. The
/// poller reads these each cycle.
#[tauri::command]
pub fn usage_guard_set(
    state: tauri::State<'_, UsageState>,
    enabled: bool,
    warn_pct: f64,
    pause_pct: f64,
) {
    state.set_guard(enabled, warn_pct, pause_pct);
}

/// Read the latest usage snapshot plus the current guard config. Cheap; no
/// network. Returns `source: None` before the first successful poll.
#[tauri::command]
pub fn usage_guard_snapshot(state: tauri::State<'_, UsageState>) -> UsageSnapshotDto {
    let (enabled, warn_pct, pause_pct) = state.config();
    let snap = state.snapshot();
    UsageSnapshotDto {
        enabled,
        warn_pct,
        pause_pct,
        percent_used: snap.percent,
        reset_epoch_ms: snap.reset_epoch_ms,
        telemetry_lost: snap.telemetry_lost,
        source: if snap.source.is_empty() {
            None
        } else {
            Some(snap.source.to_string())
        },
    }
}

/// Serializable, restart-surviving record of the current usage window's start.
/// Persisted under the app data dir so a time-based estimate stays consistent
/// across restarts when the endpoint is unreadable.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WindowStamp {
    pub window_start_ms: i64,
}

impl WindowStamp {
    pub fn window_end_ms(&self) -> i64 {
        self.window_start_ms + WINDOW_MS
    }
}

/// Time-based estimate when the endpoint is unreadable: percent is elapsed
/// fraction of the window, reset is the window end. Returns None once the
/// window has fully elapsed (stale stamp — no useful estimate).
pub fn time_estimate(stamp: &WindowStamp, now_ms: i64) -> Option<UsageSnapshot> {
    let end = stamp.window_end_ms();
    if now_ms >= end {
        return None;
    }
    let elapsed = (now_ms - stamp.window_start_ms).max(0) as f64;
    let pct = (elapsed / WINDOW_MS as f64 * 100.0).clamp(0.0, 100.0);
    Some(UsageSnapshot {
        percent: Some(pct),
        reset_epoch_ms: Some(end),
        telemetry_lost: true,
        source: "time",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_estimate_midwindow_is_half() {
        let start = 1_000_000_000_000;
        let stamp = WindowStamp { window_start_ms: start };
        let mid = start + WINDOW_MS / 2;
        let est = time_estimate(&stamp, mid).unwrap();
        assert!((est.percent.unwrap() - 50.0).abs() < 0.001);
        assert_eq!(est.reset_epoch_ms, Some(start + WINDOW_MS));
        assert_eq!(est.source, "time");
        assert!(est.telemetry_lost);
    }

    #[test]
    fn time_estimate_after_window_is_none() {
        let start = 1_000_000_000_000;
        let stamp = WindowStamp { window_start_ms: start };
        assert!(time_estimate(&stamp, start + WINDOW_MS + 1).is_none());
    }

    #[test]
    fn guard_clamps_pause_above_warn() {
        let st = UsageState::default();
        st.set_guard(true, 90.0, 50.0);
        let (enabled, warn, pause) = st.config();
        assert!(enabled);
        assert_eq!(warn, 90.0);
        assert_eq!(pause, 90.0, "pause floored at warn");
    }

    #[test]
    fn record_fires_warn_then_pause_once_each() {
        let st = UsageState::default();
        st.set_guard(true, 80.0, 95.0);
        let warn_snap = UsageSnapshot { percent: Some(82.0), source: "endpoint", ..Default::default() };
        assert_eq!(st.record(warn_snap.clone()), Some("warn".to_string()));
        // Same level again: no re-fire.
        assert_eq!(st.record(warn_snap), None);
        let pause_snap = UsageSnapshot { percent: Some(96.0), source: "endpoint", ..Default::default() };
        assert_eq!(st.record(pause_snap.clone()), Some("pause".to_string()));
        assert_eq!(st.record(pause_snap), None);
    }

    #[test]
    fn record_rearms_after_drop_below_warn() {
        let st = UsageState::default();
        st.set_guard(true, 80.0, 95.0);
        st.record(UsageSnapshot { percent: Some(85.0), source: "endpoint", ..Default::default() });
        // New window: usage drops, latch clears.
        st.record(UsageSnapshot { percent: Some(5.0), source: "endpoint", ..Default::default() });
        let warn_again = UsageSnapshot { percent: Some(81.0), source: "endpoint", ..Default::default() };
        assert_eq!(st.record(warn_again), Some("warn".to_string()));
    }

    #[test]
    fn event_shape_is_camel_case() {
        let snap = UsageSnapshot {
            percent: Some(42.5),
            reset_epoch_ms: Some(1_700_000_000_000),
            telemetry_lost: false,
            source: "endpoint",
        };
        let ev = UsageEvent::from_snapshot(&snap, Some("warn".into()));
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"percentUsed\":42.5"));
        assert!(json.contains("\"resetEpochMs\":1700000000000"));
        assert!(json.contains("\"thresholdCrossed\":\"warn\""));
        assert!(json.contains("\"telemetryLost\":false"));
        assert!(json.contains("\"source\":\"endpoint\""));
    }
}
