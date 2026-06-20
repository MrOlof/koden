// Ports claude-auto-retry's screen-scrape logic to Koden's PTY byte stream.
// Upstream capture-panes a tmux pane every ~5s and regex-matches Claude's
// usage-limit banner; we instead inspect the same bytes the agent detector
// sees and emit one retry signal carrying the parsed reset time. The JS side
// (RetryBridge) owns the wait + re-submit, so this stays a pure detector.

const RETRY_MARGIN_MS: i64 = 60_000;

// Reset clauses can wrap or repaint across chunks, so hold a small rolling
// window of recent (ANSI-stripped, lowercased) text and match against it. A
// full Claude banner with the reset clause is well under this.
const WINDOW_MAX: usize = 4096;

/// One parsed usage-limit hit. `reset_epoch_ms` is when Claude's window frees
/// up (already includes the safety margin); the bridge schedules a resubmit
/// for then.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RetrySignal {
    pub reset_epoch_ms: i64,
}

#[derive(Clone, serde::Serialize)]
pub struct RetryEvent {
    pub id: u32,
    #[serde(rename = "resetEpochMs")]
    pub reset_epoch_ms: i64,
}

impl RetrySignal {
    pub fn into_event(self, id: u32) -> RetryEvent {
        RetryEvent { id, reset_epoch_ms: self.reset_epoch_ms }
    }
}

pub struct RetryDetector {
    /// True only while an armed claude session is running (driven by session.rs
    /// from the AgentDetector transitions). Scoped so unrelated terminal output
    /// can never trigger a retry.
    active: bool,
    /// One-shot per arming: once a limit banner fires we latch until a
    /// working/exited transition re-arms, so the banner lingering on screen
    /// (Claude keeps it painted) never re-emits every chunk.
    latched: bool,
    window: String,
}

impl Default for RetryDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl RetryDetector {
    pub fn new() -> Self {
        Self { active: false, latched: false, window: String::new() }
    }

    /// Begin (or restart) watching: a claude command armed, or it resumed work.
    /// Clears the latch so the next limit banner can fire again.
    pub fn arm(&mut self) {
        self.active = true;
        self.latched = false;
        self.window.clear();
    }

    /// Stop watching (the agent exited). Also re-arms the latch so a later
    /// re-run starts clean.
    pub fn disarm(&mut self) {
        self.active = false;
        self.latched = false;
        self.window.clear();
    }

    /// Feed a chunk of raw PTY output. `now_ms` is the current wall clock in
    /// epoch ms (injected so the parse stays pure and testable).
    pub fn process<F: FnMut(RetrySignal)>(&mut self, input: &[u8], now_ms: i64, mut emit: F) {
        if !self.active || self.latched {
            return;
        }
        // Cheap early-out: the limit banners always contain one of these. The
        // modern banner's reset clause ("(MIDDLEDOT) resets <time>") can wrap
        // into a chunk that carries only "resets", so admit that word too.
        if !contains_ci(input, b"limit")
            && !contains_ci(input, b"usage")
            && !contains_ci(input, b"resets")
        {
            return;
        }
        append_stripped_lower(&mut self.window, input);
        if let Some(reset) = parse_reset(&self.window, now_ms) {
            self.latched = true;
            self.window.clear();
            emit(RetrySignal { reset_epoch_ms: reset + RETRY_MARGIN_MS });
        }
    }
}

fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| {
        w.iter()
            .zip(needle)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

// Strip CSI / OSC / common escapes, lowercase, and append to the rolling
// window. Keeps only the tail so the window can't grow unbounded on a long
// agent run that happens to mention "limit" in normal output.
fn append_stripped_lower(window: &mut String, input: &[u8]) {
    let text = String::from_utf8_lossy(input);
    let stripped = strip_ansi(&text);
    for ch in stripped.chars() {
        if ch == '\r' {
            continue;
        }
        window.push(ch.to_ascii_lowercase());
    }
    if window.len() > WINDOW_MAX {
        // Retain the tail on a char boundary.
        let cut = window.len() - WINDOW_MAX;
        let mut idx = cut;
        while idx < window.len() && !window.is_char_boundary(idx) {
            idx += 1;
        }
        *window = window[idx..].to_string();
    }
}

fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    // CSI: ESC [ ... <final 0x40..=0x7e>
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1;
                }
                b']' => {
                    // OSC: ESC ] ... (BEL | ESC \)
                    i += 1;
                    while i < bytes.len() && bytes[i] != 0x07 {
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == 0x07 {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
            continue;
        }
        // Push as a char; non-ASCII bytes are part of multi-byte UTF-8 in the
        // original &str, so copy through the str slice to stay valid.
        let ch_len = utf8_len(b);
        if i + ch_len <= bytes.len() {
            if let Ok(part) = std::str::from_utf8(&bytes[i..i + ch_len]) {
                out.push_str(part);
            }
            i += ch_len;
        } else {
            i += 1;
        }
    }
    out
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

// Recognized phrasings (already lowercased). The modern CLI (v2.1.x) renders:
//   "you've hit your <session|weekly|opus|sonnet> limit (MIDDLEDOT) resets <time> (<iana tz>)"
// where MIDDLEDOT is U+00B7. The legacy "5-hour limit reached" string is gone
// from the binary, so the modern triggers come first and the old ones stay as
// a fallback:
//   "you've hit your"  / "you're out of extra usage"  / "now using extra usage"
//   "usage limit reached"  + "resets <time>" / "resets at <time>"
//   "5-hour limit reached - resets <time>"   (legacy)
//   "please try again in N hours"  /  "... in N minutes"
//   "out of extra usage"  (no time → default backoff)
// Returns the reset wall-clock in epoch ms (before the safety margin), or None
// when no limit phrasing with a derivable time is present.
fn parse_reset(window: &str, now_ms: i64) -> Option<i64> {
    // The real banner is contiguous: "<limit phrase> (MIDDLEDOT) resets <time>"
    // (or "<limit phrase> ... please try again in N"). Require the reset/time
    // clause to be ADJACENT to the limit phrase, not merely co-present somewhere
    // in the 4096-char window — otherwise the model echoing a limit phrase in
    // one sentence and a time in another (e.g. when it explains rate limits, or
    // when this very feature is being built) would spuriously schedule a retry.
    const LIMIT_PHRASES: &[&str] = &[
        "you've hit your",
        "you're out of extra usage",
        "now using extra usage",
        "usage limit reached",
        "limit reached",
        "out of extra usage",
    ];
    // Anchor on the freshest (last) limit phrase in the window.
    let limit_end = LIMIT_PHRASES
        .iter()
        .filter_map(|p| window.rfind(p).map(|i| i + p.len()))
        .max()?;
    // Only the text just after the limit phrase counts as the banner's clause.
    // Clamp the end to a UTF-8 char boundary so slicing across the middle-dot
    // (U+00B7) can never panic the PTY reader thread.
    const MAX_GAP: usize = 80;
    let mut tail_end = (limit_end + MAX_GAP).min(window.len());
    while tail_end > limit_end && !window.is_char_boundary(tail_end) {
        tail_end -= 1;
    }
    let tail = &window[limit_end..tail_end];

    if let Some(ms) = parse_try_again_in(tail, now_ms) {
        return Some(ms);
    }
    if let Some(ms) = parse_resets_at(tail, now_ms) {
        return Some(ms);
    }
    // An extra-usage banner with no parseable clock nearby: back off a default
    // 1h, like the upstream fallback.
    if window.contains("out of extra usage") {
        return Some(now_ms + 3_600_000);
    }
    None
}

// "please try again in 3 hours" / "try again in 45 minutes" / "in 2h"
fn parse_try_again_in(window: &str, now_ms: i64) -> Option<i64> {
    let idx = window.find("try again in")?;
    let rest = &window[idx + "try again in".len()..];
    let (num, unit) = first_number_unit(rest)?;
    let secs = match unit.as_str() {
        "hour" | "hours" | "h" => num * 3600.0,
        "minute" | "minutes" | "min" | "mins" | "m" => num * 60.0,
        "second" | "seconds" | "sec" | "secs" | "s" => num,
        _ => return None,
    };
    Some(now_ms + (secs * 1000.0) as i64)
}

// "resets at 3pm" / "resets at 15:30" / "resets 3:00am" — interpreted as the
// next occurrence of that local clock time after now.
fn parse_resets_at(window: &str, now_ms: i64) -> Option<i64> {
    let idx = window.find("resets")?;
    let rest = &window[idx + "resets".len()..];
    let rest = rest.strip_prefix(" at").unwrap_or(rest);
    let (hour, minute) = parse_clock(rest)?;
    Some(next_local_occurrence(hour, minute, now_ms))
}

// Pull the first "<number><unit>" pair out of a fragment, tolerating a unit
// either glued ("2h") or spaced ("2 hours").
fn first_number_unit(s: &str) -> Option<(f64, String)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let num: f64 = s[start..i].parse().ok()?;
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            let ustart = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            return Some((num, s[ustart..i].to_string()));
        }
        i += 1;
    }
    None
}

// Parse a clock fragment like " 3pm", " 15:30", " 3:00 am". Returns 24h
// (hour, minute). 12h with am/pm is normalized.
//
// Scans candidate numbers and accepts the first that actually looks like a
// clock time: either it carries a ":MM" part, or it is directly followed
// (optional space) by an am/pm suffix. This skips date-day numbers in banners
// like "resets jun 21, 3pm (utc)" where the bare "21" must not be read as the
// hour. A trailing " (america/new_york)" timezone paren after the suffix is
// ignored because the suffix anchors the parse before it.
fn parse_clock(s: &str) -> Option<(u32, u32)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    loop {
        while i < bytes.len() && !bytes[i].is_ascii_digit() {
            // A sentence terminator means we've wandered past the clause.
            if bytes[i] == b'.' || bytes[i] == b'\n' {
                return None;
            }
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        if let Some((hm, _next)) = try_clock_at(s, i) {
            return Some(hm);
        }
        // Not a clock at this digit run; skip it and keep scanning.
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
}

// Attempt to read a clock starting at byte index `start` (which must point at a
// digit). Returns ((hour24, minute), index-after-suffix) when the run qualifies
// as a clock time, else None.
fn try_clock_at(s: &str, start: usize) -> Option<((u32, u32), usize)> {
    let bytes = s.as_bytes();
    let mut i = start;
    let hstart = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let mut hour: u32 = s[hstart..i].parse().ok()?;
    let mut minute: u32 = 0;
    let mut had_minute = false;
    if i < bytes.len() && bytes[i] == b':' {
        i += 1;
        let mstart = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == mstart {
            return None;
        }
        minute = s[mstart..i].parse().ok()?;
        had_minute = true;
    }
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let suffix = &s[i..];
    let mut had_suffix = false;
    if suffix.starts_with("pm") {
        if hour < 12 {
            hour += 12;
        }
        had_suffix = true;
        i += 2;
    } else if suffix.starts_with("am") {
        if hour == 12 {
            hour = 0;
        }
        had_suffix = true;
        i += 2;
    }
    // A bare number with neither ":MM" nor an am/pm suffix is not a clock (it's
    // a date day, a token count, etc.) — reject so the scanner moves on.
    if !had_minute && !had_suffix {
        return None;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(((hour, minute), i))
}

// Next local-time occurrence of (hour:minute) strictly after now_ms. Derives
// the local UTC offset from the platform without pulling chrono: compute
// today's local midnight in epoch ms, add the target, roll forward a day if
// it already passed. Keeps the crate dependency-free.
fn next_local_occurrence(hour: u32, minute: u32, now_ms: i64) -> i64 {
    let offset_ms = local_offset_ms(now_ms);
    let local_now = now_ms + offset_ms;
    let day_ms = 86_400_000i64;
    let local_midnight = local_now.div_euclid(day_ms) * day_ms;
    let target_local = local_midnight + (hour as i64) * 3_600_000 + (minute as i64) * 60_000;
    let mut target_utc = target_local - offset_ms;
    if target_utc <= now_ms {
        target_utc += day_ms;
    }
    target_utc
}

#[cfg(test)]
fn local_offset_ms(_now_ms: i64) -> i64 {
    // Tests pin the offset to UTC so clock-time assertions are deterministic.
    0
}

// Best-effort local UTC offset in ms. Claude renders the reset time in the
// MACHINE-LOCAL timezone, so the machine-local offset is exactly what converts
// that clock to epoch. Tests pin it to UTC for deterministic clock assertions.
#[cfg(not(test))]
fn local_offset_ms(_now_ms: i64) -> i64 {
    platform_offset_ms().unwrap_or(0)
}

// Real platform offset, derived from the C runtime's local-vs-utc broken-down
// time gap for the same instant. DST-correct (the local conversion honors
// whatever rule is in effect). Compiled in all configs so the smoke test below
// can exercise the real path even though `local_offset_ms` is test-pinned.
#[cfg(unix)]
fn platform_offset_ms() -> Option<i64> {
    // SAFETY: localtime_r/gmtime_r are reentrant; we pass valid stack pointers
    // and a current time_t. No globals are mutated.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        let mut utc: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut local).is_null() {
            return None;
        }
        if libc::gmtime_r(&t, &mut utc).is_null() {
            return None;
        }
        Some((tm_to_secs(&local) - tm_to_secs(&utc)) * 1000)
    }
}

// Windows MSVC: the CRT exposes the reentrant secure variants localtime_s /
// gmtime_s (errno_t, 0 == success) plus tzset(). Same diff-of-broken-down-time
// approach as the unix arm. This replaces the old UTC-only fallback so "resets
// <clock>" computes in the machine's real timezone.
#[cfg(windows)]
fn platform_offset_ms() -> Option<i64> {
    // SAFETY: localtime_s/gmtime_s are reentrant; we pass valid stack pointers
    // and a current time_t. tzset() initializes the CRT timezone state.
    unsafe {
        libc::tzset();
        let t = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        let mut utc: libc::tm = std::mem::zeroed();
        if libc::localtime_s(&mut local, &t) != 0 {
            return None;
        }
        if libc::gmtime_s(&mut utc, &t) != 0 {
            return None;
        }
        Some((tm_to_secs(&local) - tm_to_secs(&utc)) * 1000)
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_offset_ms() -> Option<i64> {
    None
}

#[cfg(any(unix, windows))]
fn tm_to_secs(tm: &libc::tm) -> i64 {
    // Relative seconds-of-era good enough for a local-minus-utc difference. Day
    // resolution (year*365 + yday) is enough because the two tms are at most a
    // day apart across the date line, and we only need their difference.
    let days = (tm.tm_year as i64) * 365 + (tm.tm_yday as i64);
    days * 86_400 + (tm.tm_hour as i64) * 3600 + (tm.tm_min as i64) * 60 + tm.tm_sec as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON_UTC: i64 = 1_700_000_000_000; // fixed reference "now"

    fn run(d: &mut RetryDetector, input: &[u8], now: i64) -> Vec<RetrySignal> {
        let mut out = Vec::new();
        d.process(input, now, |s| out.push(s));
        out
    }

    #[test]
    fn no_fire_when_inactive() {
        let mut d = RetryDetector::new();
        assert!(run(&mut d, b"usage limit reached, resets at 3pm", NOON_UTC).is_empty());
    }

    #[test]
    fn early_out_ignores_chunks_without_keywords() {
        let mut d = RetryDetector::new();
        d.arm();
        assert!(run(&mut d, b"compiling project, all good", NOON_UTC).is_empty());
    }

    #[test]
    fn fires_on_try_again_in_hours() {
        let mut d = RetryDetector::new();
        d.arm();
        let out = run(&mut d, b"usage limit reached. please try again in 3 hours.", NOON_UTC);
        assert_eq!(out.len(), 1);
        let expected = NOON_UTC + 3 * 3_600_000 + RETRY_MARGIN_MS;
        assert_eq!(out[0].reset_epoch_ms, expected);
    }

    #[test]
    fn fires_on_try_again_in_minutes() {
        let mut d = RetryDetector::new();
        d.arm();
        let out = run(&mut d, b"usage limit reached, try again in 45 minutes", NOON_UTC);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].reset_epoch_ms, NOON_UTC + 45 * 60_000 + RETRY_MARGIN_MS);
    }

    #[test]
    fn fires_on_resets_at_clock_24h() {
        // now is 1700000000000 = 2023-11-14 22:13:20 UTC. With UTC offset
        // pinned to 0 in tests, "resets at 23:00" is later same day.
        let mut d = RetryDetector::new();
        d.arm();
        let out = run(&mut d, b"5-hour limit reached - resets at 23:00", NOON_UTC);
        assert_eq!(out.len(), 1);
        assert!(out[0].reset_epoch_ms > NOON_UTC);
        // within the same UTC day, before +24h
        assert!(out[0].reset_epoch_ms < NOON_UTC + 86_400_000 + RETRY_MARGIN_MS + 1);
    }

    #[test]
    fn resets_at_clock_rolls_to_next_day_when_past() {
        // "resets at 01:00" is already past 22:13 today → next day.
        let mut d = RetryDetector::new();
        d.arm();
        let out = run(&mut d, b"usage limit reached - resets at 1:00am", NOON_UTC);
        assert_eq!(out.len(), 1);
        assert!(out[0].reset_epoch_ms > NOON_UTC);
        assert!(out[0].reset_epoch_ms > NOON_UTC + 60_000);
    }

    #[test]
    fn out_of_extra_usage_falls_back_to_one_hour() {
        let mut d = RetryDetector::new();
        d.arm();
        let out = run(&mut d, b"you are out of extra usage for now", NOON_UTC);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].reset_epoch_ms, NOON_UTC + 3_600_000 + RETRY_MARGIN_MS);
    }

    #[test]
    fn latches_so_lingering_banner_does_not_re_emit() {
        let mut d = RetryDetector::new();
        d.arm();
        let first = run(&mut d, b"usage limit reached, try again in 2 hours", NOON_UTC);
        assert_eq!(first.len(), 1);
        // Same banner repainted on screen must not fire again.
        assert!(run(&mut d, b"usage limit reached, try again in 2 hours", NOON_UTC).is_empty());
    }

    #[test]
    fn re_arm_clears_latch() {
        let mut d = RetryDetector::new();
        d.arm();
        assert_eq!(run(&mut d, b"usage limit reached, try again in 1 hour", NOON_UTC).len(), 1);
        d.arm();
        assert_eq!(run(&mut d, b"usage limit reached, try again in 1 hour", NOON_UTC).len(), 1);
    }

    #[test]
    fn disarm_stops_detection() {
        let mut d = RetryDetector::new();
        d.arm();
        d.disarm();
        assert!(run(&mut d, b"usage limit reached, try again in 1 hour", NOON_UTC).is_empty());
    }

    #[test]
    fn strips_ansi_before_matching() {
        let mut d = RetryDetector::new();
        d.arm();
        let input = b"\x1b[31musage limit reached\x1b[0m, try again in \x1b[1m2 hours\x1b[0m";
        let out = run(&mut d, input, NOON_UTC);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].reset_epoch_ms, NOON_UTC + 2 * 3_600_000 + RETRY_MARGIN_MS);
    }

    #[test]
    fn matches_across_chunk_boundary() {
        let mut d = RetryDetector::new();
        d.arm();
        // The keyword is in the first chunk so the early-out passes; the time
        // clause arrives in the second chunk (also containing "limit"-ish?).
        assert!(run(&mut d, b"usage limit reached.", NOON_UTC).is_empty());
        let out = run(&mut d, b" usage: please try again in 30 minutes", NOON_UTC);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].reset_epoch_ms, NOON_UTC + 30 * 60_000 + RETRY_MARGIN_MS);
    }

    #[test]
    fn ignores_unrelated_limit_word() {
        let mut d = RetryDetector::new();
        d.arm();
        assert!(run(&mut d, b"rate limit on the api is 100 usage units", NOON_UTC).is_empty());
    }

    #[test]
    fn into_event_carries_id_and_reset() {
        let sig = RetrySignal { reset_epoch_ms: 123 };
        let ev = sig.into_event(7);
        assert_eq!(ev.id, 7);
        assert_eq!(ev.reset_epoch_ms, 123);
    }

    // --- Modern CLI (v2.1.x) banner: "·" is U+00B7, legacy strings gone. ---

    #[test]
    fn fires_on_modern_session_limit_banner_with_middledot_and_tz() {
        let mut d = RetryDetector::new();
        d.arm();
        // Exact modern phrasing: middledot separator + a trailing IANA tz paren.
        let banner = "You've hit your session limit \u{00b7} resets 3:05pm (America/New_York)";
        let out = run(&mut d, banner.as_bytes(), NOON_UTC);
        assert_eq!(out.len(), 1, "modern session-limit banner must fire");
        // Offset pinned to 0 in tests, so 3:05pm == 15:05 UTC, next occurrence.
        let r = out[0].reset_epoch_ms - RETRY_MARGIN_MS;
        let day_ms = 86_400_000i64;
        let minute_of_day = r.rem_euclid(day_ms) / 60_000;
        assert_eq!(minute_of_day, 15 * 60 + 5, "reset clock should be 15:05 UTC");
        assert!(out[0].reset_epoch_ms > NOON_UTC);
    }

    #[test]
    fn fires_on_modern_weekly_limit_banner_with_date_and_tz() {
        let mut d = RetryDetector::new();
        d.arm();
        // Weekly banner carries a date ("Jun 21") before the clock; the bare
        // day number "21" must not be mistaken for the hour.
        let banner = "You've hit your weekly limit \u{00b7} resets Jun 21, 3pm (UTC)";
        let out = run(&mut d, banner.as_bytes(), NOON_UTC);
        assert_eq!(out.len(), 1, "modern weekly-limit banner must fire");
        let r = out[0].reset_epoch_ms - RETRY_MARGIN_MS;
        let minute_of_day = r.rem_euclid(86_400_000) / 60_000;
        assert_eq!(minute_of_day, 15 * 60, "reset clock should be 15:00, not 21:00");
    }

    #[test]
    fn fires_on_now_using_extra_usage_banner() {
        let mut d = RetryDetector::new();
        d.arm();
        let banner = "You're now using extra usage \u{00b7} resets 9:30am (UTC)";
        let out = run(&mut d, banner.as_bytes(), NOON_UTC);
        assert_eq!(out.len(), 1);
        let minute_of_day =
            (out[0].reset_epoch_ms - RETRY_MARGIN_MS).rem_euclid(86_400_000) / 60_000;
        assert_eq!(minute_of_day, 9 * 60 + 30);
    }

    #[test]
    fn fires_when_resets_clause_arrives_in_its_own_chunk() {
        // The early-out must admit a chunk that carries only "resets ..." after
        // the limit header landed in a prior chunk.
        let mut d = RetryDetector::new();
        d.arm();
        assert!(run(&mut d, "You've hit your session limit".as_bytes(), NOON_UTC).is_empty());
        let out = run(&mut d, " \u{00b7} resets 3pm (UTC)".as_bytes(), NOON_UTC);
        assert_eq!(out.len(), 1, "second chunk carrying only the resets clause must fire");
    }

    // Regression: the OLD has_limit (usage-limit-reached / limit-reached /
    // out-of-extra-usage only) would have MISSED the modern banner entirely.
    #[test]
    fn old_has_limit_logic_would_have_missed_modern_banner() {
        let banner = "you've hit your session limit \u{00b7} resets 3:05pm (america/new_york)";
        // Reconstruct the pre-fix predicate verbatim.
        let old_has_limit = banner.contains("usage limit reached")
            || banner.contains("limit reached")
            || banner.contains("out of extra usage");
        assert!(!old_has_limit, "old predicate must NOT match the modern banner");
        // The fix's predicate does match it.
        assert!(parse_reset(banner, NOON_UTC).is_some(), "new parse_reset must match");
    }

    #[test]
    fn trailing_timezone_paren_does_not_derail_minute_parse() {
        // A "(America/New_York)" suffix after the am/pm must not bleed into the
        // minute. 11:45pm => 23:45.
        assert_eq!(parse_clock(" 11:45pm (America/New_York)"), Some((23, 45)));
        assert_eq!(parse_clock(" 3pm (UTC)"), Some((15, 0)));
        // 24h with tz paren, no am/pm.
        assert_eq!(parse_clock(" 15:30 (Europe/Stockholm)"), Some((15, 30)));
    }

    #[test]
    fn parse_clock_skips_date_day_number() {
        // "jun 21, 3pm" — the day "21" is not the hour.
        assert_eq!(parse_clock(" jun 21, 3pm"), Some((15, 0)));
        // A bare number with no clock markers is rejected.
        assert_eq!(parse_clock(" 42 tokens left"), None);
    }

    // Smoke: the real platform offset (not the test-pinned zero) must resolve
    // to a sane wall-clock offset within +/- 14h on unix and windows hosts.
    #[cfg(any(unix, windows))]
    #[test]
    fn platform_offset_is_some_and_within_14h() {
        let off = platform_offset_ms();
        assert!(off.is_some(), "platform offset must resolve on this host");
        assert!(
            off.unwrap().abs() <= 14 * 3_600_000,
            "offset {:?} out of +/-14h bounds",
            off
        );
    }
}
