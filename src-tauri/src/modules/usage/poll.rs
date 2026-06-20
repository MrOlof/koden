// Async usage poller + OAuth token refresh + restart-surviving time window.
//
// Driven from a plain std::thread (NOT tokio time): the thread sleeps the
// adaptive cadence with std::thread::sleep and runs each reqwest call via
// tauri::async_runtime::block_on, so we never enable tokio's "time" feature.
//
// SECURITY: access/refresh tokens are NEVER logged. Only HTTP status codes and
// derived percentages are ever emitted to logs or the frontend.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager};

use super::{
    read_creds, time_estimate, write_creds, OauthCreds, UsageEvent, UsageSnapshot, UsageState,
    WindowStamp, FALLBACK_CLI_VERSION, OAUTH_CLIENT_ID,
};

pub const USAGE_EVENT: &str = "koden:usage-signal";

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const OAUTH_BETA: &str = "oauth-2025-04-20";
// Refresh proactively when the token is within this of expiry.
const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;
// After this many consecutive failed polls, flag telemetry_lost once.
const TELEMETRY_LOST_AFTER: u32 = 3;

fn now_epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Base URL for the usage endpoint, honoring KODEN_USAGE_ENDPOINT so the
/// sandbox can point us at a local stub. Trailing slash trimmed.
fn base_url() -> String {
    std::env::var("KODEN_USAGE_ENDPOINT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// Mandatory User-Agent: `claude-code/<version>`. Reads the installed package
/// version at runtime, falling back to a pinned const. Without it the endpoint
/// returns a persistent 429.
fn user_agent() -> String {
    format!("claude-code/{}", installed_cli_version())
}

fn installed_cli_version() -> String {
    for path in cli_package_json_candidates() {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            #[derive(Deserialize)]
            struct Pkg {
                version: String,
            }
            if let Ok(pkg) = serde_json::from_str::<Pkg>(&raw) {
                if !pkg.version.trim().is_empty() {
                    return pkg.version;
                }
            }
        }
    }
    FALLBACK_CLI_VERSION.to_string()
}

// Likely install locations of @anthropic-ai/claude-code's package.json across
// the common global-install layouts (npm global, the local-bin shim dir).
fn cli_package_json_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let pkg = ["@anthropic-ai", "claude-code", "package.json"];
    let join = |base: PathBuf, parts: &[&str]| -> PathBuf {
        let mut p = base;
        for part in parts {
            p = p.join(part);
        }
        p
    };
    if let Some(home) = dirs::home_dir() {
        // npm global (Windows: %APPDATA%/npm/node_modules; unix: lib/node_modules)
        if let Some(appdata) = dirs::data_dir() {
            out.push(join(appdata.join("npm").join("node_modules"), &pkg));
        }
        out.push(join(
            home.join(".local").join("share").join("npm").join("lib").join("node_modules"),
            &pkg,
        ));
        out.push(join(home.join(".npm-global").join("lib").join("node_modules"), &pkg));
        out.push(join(home.join(".local").join("lib").join("node_modules"), &pkg));
    }
    out
}

/// Build a rustls-backed async client. The production endpoint is public-only;
/// the override may reach a local sandbox.
fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
struct UsageBody {
    five_hour: Option<FiveHour>,
}

#[derive(Debug, Deserialize)]
struct FiveHour {
    utilization: Option<f64>,
    resets_at: Option<serde_json::Value>,
}

fn normalize_pct(raw: f64) -> f64 {
    // Endpoint may report 0..1 or 0..100; treat <= 1.0 as a fraction.
    let pct = if raw <= 1.0 { raw * 100.0 } else { raw };
    pct.clamp(0.0, 100.0)
}

/// Convert the endpoint's reset value (ISO8601 string or epoch number) to epoch
/// ms. Returns None when neither form parses.
fn reset_to_epoch_ms(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Number(n) => {
            let f = n.as_f64()?;
            // Heuristic: seconds vs ms. Anything below year-2300-in-seconds is
            // treated as seconds.
            if f > 1e12 {
                Some(f as i64)
            } else {
                Some((f * 1000.0) as i64)
            }
        }
        serde_json::Value::String(s) => parse_iso8601_ms(s),
        _ => None,
    }
}

// Minimal ISO8601 -> epoch ms without chrono. Handles
// "YYYY-MM-DDTHH:MM:SS[.fff][Z|+HH:MM]". Good enough for the endpoint's format.
fn parse_iso8601_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<i64> { s.get(a..b)?.parse::<i64>().ok() };
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let min = num(14, 16)?;
    let sec = num(17, 19)?;
    // Optional timezone after the seconds (and optional fractional part).
    let mut tz_off_secs: i64 = 0;
    let rest = &s[19..];
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(stripped) = rest.strip_prefix(['+', '-']) {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let h: i64 = stripped.get(0..2)?.parse().ok()?;
        let m: i64 = stripped
            .get(3..5)
            .or_else(|| stripped.get(2..4))
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        tz_off_secs = sign * (h * 3600 + m * 60);
    }
    let days = days_from_civil(year, month, day);
    let utc_secs = days * 86_400 + hour * 3600 + min * 60 + sec - tz_off_secs;
    Some(utc_secs * 1000)
}

// Days since 1970-01-01 (Howard Hinnant's civil algorithm), no chrono.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Ensure the access token is fresh enough to use; refresh when within the skew
/// window. Returns the (possibly refreshed) creds. NEVER logs token values.
async fn ensure_fresh(client: &reqwest::Client, creds: OauthCreds) -> Result<OauthCreds, String> {
    if creds.expires_at - now_epoch_ms() > REFRESH_SKEW_MS {
        return Ok(creds);
    }
    refresh(client, &creds).await
}

async fn refresh(client: &reqwest::Client, creds: &OauthCreds) -> Result<OauthCreds, String> {
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": creds.refresh_token,
        "client_id": OAUTH_CLIENT_ID,
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/json")
        .header("User-Agent", user_agent())
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("token refresh http {}", status.as_u16()));
    }
    #[derive(Deserialize)]
    struct TokenResp {
        access_token: String,
        refresh_token: Option<String>,
        // seconds until expiry
        expires_in: Option<i64>,
        // some flows return an absolute ms timestamp instead
        #[serde(rename = "expiresAt")]
        expires_at: Option<i64>,
    }
    let raw = resp.bytes().await.map_err(|e| e.to_string())?;
    let tok: TokenResp = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let expires_at = tok
        .expires_at
        .or_else(|| tok.expires_in.map(|s| now_epoch_ms() + s * 1000))
        .unwrap_or_else(|| now_epoch_ms() + 3_600_000);
    let updated = OauthCreds {
        access_token: tok.access_token,
        refresh_token: tok.refresh_token.unwrap_or_else(|| creds.refresh_token.clone()),
        expires_at,
    };
    // Compare-and-swap: only commit if nobody (e.g. the Claude Code CLI) rotated
    // the on-disk refresh_token while our network round trip was in flight.
    // Refresh tokens are single-use, so an unconditional last-writer-wins could
    // clobber a newer grant the CLI just obtained and invalidate it. If it
    // changed, adopt the on-disk tokens instead.
    if let Some(on_disk) = read_creds() {
        if on_disk.refresh_token != creds.refresh_token {
            log::debug!("usage: creds rotated concurrently; adopting on-disk tokens");
            return Ok(on_disk);
        }
    }
    write_creds(&updated)?;
    log::debug!("usage: oauth token refreshed");
    Ok(updated)
}

/// Perform one usage fetch. On 401 it refreshes once and retries. Returns a live
/// snapshot, or an error string (kept generic, never includes token material).
async fn fetch_once(client: &reqwest::Client) -> Result<UsageSnapshot, String> {
    let creds = read_creds().ok_or_else(|| "no credentials".to_string())?;
    let creds = ensure_fresh(client, creds).await?;
    match request_usage(client, &creds).await {
        Ok(snap) => Ok(snap),
        Err(FetchErr::Unauthorized) => {
            // Token may have been revoked server-side; force a refresh + retry.
            let refreshed = refresh(client, &creds).await?;
            request_usage(client, &refreshed)
                .await
                .map_err(|e| e.into_string())
        }
        Err(e) => Err(e.into_string()),
    }
}

enum FetchErr {
    Unauthorized,
    Other(String),
}

impl FetchErr {
    fn into_string(self) -> String {
        match self {
            FetchErr::Unauthorized => "http 401".to_string(),
            FetchErr::Other(s) => s,
        }
    }
}

async fn request_usage(
    client: &reqwest::Client,
    creds: &OauthCreds,
) -> Result<UsageSnapshot, FetchErr> {
    let url = format!("{}/api/oauth/usage", base_url());
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", creds.access_token))
        .header("anthropic-beta", OAUTH_BETA)
        .header("User-Agent", user_agent())
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| FetchErr::Other(e.to_string()))?;
    let status = resp.status();
    if status.as_u16() == 401 {
        return Err(FetchErr::Unauthorized);
    }
    if !status.is_success() {
        return Err(FetchErr::Other(format!("http {}", status.as_u16())));
    }
    let raw = resp
        .bytes()
        .await
        .map_err(|e| FetchErr::Other(e.to_string()))?;
    let body: UsageBody =
        serde_json::from_slice(&raw).map_err(|e| FetchErr::Other(e.to_string()))?;
    let five = body
        .five_hour
        .ok_or_else(|| FetchErr::Other("missing five_hour".to_string()))?;
    let percent = five.utilization.map(normalize_pct);
    let reset_epoch_ms = five.resets_at.as_ref().and_then(reset_to_epoch_ms);
    Ok(UsageSnapshot {
        percent,
        reset_epoch_ms,
        telemetry_lost: false,
        source: "endpoint",
    })
}

/// Adaptive cadence keyed on the last known percent. More usage => poll faster.
fn cadence_for(percent: Option<f64>) -> Duration {
    match percent {
        Some(p) if p >= 90.0 => Duration::from_secs(60),
        Some(p) if p >= 85.0 => Duration::from_secs(5 * 60),
        Some(p) if p >= 70.0 => Duration::from_secs(10 * 60),
        Some(p) if p >= 50.0 => Duration::from_secs(15 * 60),
        _ => Duration::from_secs(30 * 60),
    }
}

// --- restart-surviving time window ---

fn window_stamp_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("usage-window.json"))
}

fn read_window_stamp(app: &AppHandle) -> Option<WindowStamp> {
    let path = window_stamp_path(app).ok()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str::<WindowStamp>(&raw).ok()
}

fn write_window_stamp(app: &AppHandle, stamp: &WindowStamp) -> Result<(), String> {
    let path = window_stamp_path(app)?;
    let serialized = serde_json::to_string(stamp).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.koden-tmp");
    std::fs::write(&tmp, serialized).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Called by the PTY session on the first claude activity: stamp a window start
/// of `now` (end = +5h) if no live window is already in progress. Idempotent
/// within a window so repeated agent transitions don't keep resetting it.
pub fn ensure_window_started(app: &AppHandle) {
    let now = now_epoch_ms();
    if let Some(existing) = read_window_stamp(app) {
        if now < existing.window_end_ms() {
            return;
        }
    }
    let stamp = WindowStamp { window_start_ms: now };
    if let Err(e) = write_window_stamp(app, &stamp) {
        log::debug!("usage: could not persist window stamp: {e}");
    }
}

/// Spawn the background poller. One in-flight poll at a time (this thread is the
/// only caller). Fail-open: errors keep the last-good snapshot and never panic.
pub fn spawn_poller(app: AppHandle) {
    std::thread::Builder::new()
        .name("koden-usage-poller".into())
        .spawn(move || poller_loop(app))
        .expect("spawn usage poller thread");
}

fn poller_loop(app: AppHandle) {
    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("usage: client build failed, poller disabled: {e}");
            return;
        }
    };
    let in_flight = Arc::new(AtomicBool::new(false));
    let mut consecutive_failures: u32 = 0;
    let mut telemetry_lost_notified = false;
    // First cadence is conservative until we know the percent.
    let mut next_delay = Duration::from_secs(30);

    loop {
        std::thread::sleep(next_delay);

        let state = app.state::<UsageState>();
        let (enabled, _warn, _pause) = state.config();
        if !enabled {
            // Guard off: idle-poll slowly so re-enabling picks up quickly.
            next_delay = Duration::from_secs(60);
            continue;
        }

        // One in-flight poll: skip if a previous block_on somehow overlapped
        // (defensive — this loop is single-threaded so it never should).
        if in_flight.swap(true, Ordering::AcqRel) {
            next_delay = Duration::from_secs(30);
            continue;
        }
        let result = tauri::async_runtime::block_on(fetch_once(&client));
        in_flight.store(false, Ordering::Release);

        let snapshot = match result {
            Ok(snap) => {
                consecutive_failures = 0;
                telemetry_lost_notified = false;
                Some(snap)
            }
            Err(e) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                log::debug!(
                    "usage: poll failed ({e}); consecutive={consecutive_failures}"
                );
                // Fall back to a time-based estimate from the persisted window.
                read_window_stamp(&app).and_then(|stamp| time_estimate(&stamp, now_epoch_ms()))
            }
        };

        let Some(mut snapshot) = snapshot else {
            // No live read and no usable time estimate. After enough failures,
            // notify telemetry_lost exactly once, then keep last-good.
            if consecutive_failures >= TELEMETRY_LOST_AFTER && !telemetry_lost_notified {
                telemetry_lost_notified = true;
                let mut lost = state.snapshot();
                lost.telemetry_lost = true;
                let ev = UsageEvent::from_snapshot(&lost, None);
                let _ = app.emit(USAGE_EVENT, ev);
            }
            next_delay = cadence_for(state.snapshot().percent);
            continue;
        };

        if consecutive_failures >= TELEMETRY_LOST_AFTER {
            snapshot.telemetry_lost = true;
        }

        // A live read realigns the persisted window so time-based fallback stays
        // consistent if the endpoint later goes dark.
        if snapshot.source == "endpoint" {
            if let Some(reset) = snapshot.reset_epoch_ms {
                let start = reset - super::WINDOW_MS;
                let _ = write_window_stamp(&app, &WindowStamp { window_start_ms: start });
            }
        }

        state.maybe_reset_window(snapshot.reset_epoch_ms);
        let crossed = state.record(snapshot.clone());
        let ev = UsageEvent::from_snapshot(&snapshot, crossed);
        let _ = app.emit(USAGE_EVENT, ev);

        next_delay = cadence_for(snapshot.percent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pct_handles_fraction_and_percent() {
        assert!((normalize_pct(0.42) - 42.0).abs() < 0.001);
        assert!((normalize_pct(42.0) - 42.0).abs() < 0.001);
        assert!((normalize_pct(1.0) - 100.0).abs() < 0.001);
        assert_eq!(normalize_pct(250.0), 100.0);
    }

    #[test]
    fn cadence_tightens_with_usage() {
        assert_eq!(cadence_for(Some(95.0)), Duration::from_secs(60));
        assert_eq!(cadence_for(Some(88.0)), Duration::from_secs(5 * 60));
        assert_eq!(cadence_for(Some(75.0)), Duration::from_secs(10 * 60));
        assert_eq!(cadence_for(Some(60.0)), Duration::from_secs(15 * 60));
        assert_eq!(cadence_for(Some(10.0)), Duration::from_secs(30 * 60));
        assert_eq!(cadence_for(None), Duration::from_secs(30 * 60));
    }

    #[test]
    fn reset_to_epoch_ms_from_number_seconds_and_ms() {
        // seconds
        let s = serde_json::json!(1_700_000_000i64);
        assert_eq!(reset_to_epoch_ms(&s), Some(1_700_000_000_000));
        // ms
        let m = serde_json::json!(1_700_000_000_000i64);
        assert_eq!(reset_to_epoch_ms(&m), Some(1_700_000_000_000));
    }

    #[test]
    fn parse_iso8601_utc_z() {
        // 2023-11-14T22:13:20Z == 1700000000 s
        let ms = parse_iso8601_ms("2023-11-14T22:13:20Z").unwrap();
        assert_eq!(ms, 1_700_000_000_000);
    }

    #[test]
    fn parse_iso8601_with_offset_and_fraction() {
        // +01:00 offset means the UTC instant is one hour earlier.
        let base = parse_iso8601_ms("2023-11-14T22:13:20Z").unwrap();
        let off = parse_iso8601_ms("2023-11-14T23:13:20.500+01:00").unwrap();
        assert_eq!(off, base);
    }

    #[test]
    fn days_from_civil_epoch_is_zero() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
    }

    #[test]
    fn base_url_defaults_to_anthropic() {
        // Note: relies on env not being set in the test runner.
        if std::env::var("KODEN_USAGE_ENDPOINT").is_err() {
            assert_eq!(base_url(), DEFAULT_BASE_URL);
        }
    }

    #[test]
    fn user_agent_is_claude_code_shaped() {
        let ua = user_agent();
        assert!(ua.starts_with("claude-code/"));
        // version segment is non-empty
        assert!(ua.len() > "claude-code/".len());
    }
}
