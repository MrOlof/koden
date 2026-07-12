//! P3 — cache-stable gist injection (the payoff). Synthesize a token-bounded,
//! fingerprint-keyed context bundle from the index and inject it into agents via
//! the existing `--append-system-prompt` channel.
//!
//! NON-NEGOTIABLE (the P3 gate): an unchanged relaunch yields a BYTE-IDENTICAL
//! gist. The gist sits in the cacheable prompt prefix; a per-launch-mutating gist
//! busts the agent's prompt cache (~90% input-cost penalty). We guarantee this by
//! deriving the gist purely from deterministic index state (search is now fully
//! ordered, notes/symbols are sorted), read over ONE pinned snapshot so the key
//! and the body never tear apart, and keying it by
//! `blake3(project_fingerprint ‖ temporal_digest ‖ overdue_digest ‖ intent ‖
//! budget ‖ schema_version)` (CONCEPT §6 Flow C, [DP-21]/[DP-22]).
//!
//! Byte-identity is a *property of the deterministic build*, not a stored
//! key→bytes cache. CONCEPT Flow C step 5 ("emit the byte-identical prior gist")
//! is satisfied by re-deriving the same bytes, not by memoizing a blob — there is
//! deliberately no on-disk gist cache (nothing to invalidate, nothing to stale).
//!
//! Secret-safe: the gist draws only from the index — FTS content is pre-redacted,
//! note titles are redacted at scan, symbols/paths are identifiers. No raw source
//! is re-read here. Snippet-text enrichment is a P3 refinement.
//!
//! Layered + fail-open: the freshness line is always kept; a known-unknowns
//! section (ADR-011 — empty retrieval legs stated explicitly, only over a ready
//! non-empty index) sits right after it (trimmed last-but-one); relevant files +
//! their top symbols, then top memory notes — each carrying a per-claim
//! freshness label (current / possibly-stale / historical(superseded)) — are
//! added while the char budget allows.

pub mod artifact;
pub mod synth;

use std::path::Path;

use rusqlite::Connection;

use crate::modules::brain::memory::doctor::path_anchor;
use crate::modules::brain::store;
use crate::modules::brain::store::schema::SCHEMA_VERSION;

const MAX_FILES: usize = 12;
const MAX_SYMS_PER_FILE: usize = 6;
const MAX_NOTES: usize = 8;
/// ADR-020 "Recent activity" bounds: day buckets rendered / agents per day /
/// files per day / rows read (covers the whole per-project retention cap, so
/// the derived set only moves when the STORED set does — not on a read window).
const ACTIVITY_MAX_DAYS: usize = 3;
const ACTIVITY_MAX_AGENTS: usize = 3;
const ACTIVITY_MAX_FILES: usize = 6;
const ACTIVITY_READ_ROWS: usize = 500;
/// chars-per-token heuristic ([DP-21]) — no exact cross-vendor tokenizer.
const CHARS_PER_TOKEN: usize = 4;
/// Char bound for the intent excerpt in the known-unknowns line (ADR-011).
const EXCERPT_CHARS: usize = 64;
const DAY_MS: i64 = 86_400_000;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Gist {
    pub bytes: String,
    /// Cache key — `blake3(fingerprint ‖ temporal_digest ‖ overdue_digest ‖
    /// intent ‖ budget ‖ schema_version)`. An unchanged relaunch reproduces
    /// this key (and byte-identical `bytes`).
    pub fingerprint: String,
    pub sources: Vec<String>,
}

/// Build the gist for `intent` in `project_id` under a `budget_tokens` ceiling.
/// Zero tokens to build (pure index reads); deterministic → byte-stable.
///
/// Every read runs over ONE pinned WAL snapshot ([open_readonly_snapshot]), so
/// the cache key (fingerprint) and the rendered body cannot be torn across a
/// concurrent worker commit — the P3 byte-identity gate. Fail-open: if the index
/// isn't ready, the snapshot is `None` and a freshness-only gist is returned.
pub fn build_gist(
    db_path: &Path,
    project_id: &str,
    project_name: &str,
    intent: &str,
    budget_tokens: usize,
) -> Gist {
    let conn = store::open_readonly_snapshot(db_path).ok();
    build_gist_on_conn(conn.as_ref(), project_id, project_name, intent, budget_tokens)
}

/// Build the gist, synthesizing a cold-start intent when `intent` is blank. The
/// synthesis and the build share one snapshot, so the synthesized intent and the
/// body it drives observe the same index state.
pub fn build_gist_auto(
    db_path: &Path,
    project_id: &str,
    project_name: &str,
    intent: &str,
    budget_tokens: usize,
) -> Gist {
    let conn = store::open_readonly_snapshot(db_path).ok();
    let query = if intent.trim().is_empty() {
        synth::synthesize_intent_on_conn(conn.as_ref(), project_id, project_name)
    } else {
        intent.to_string()
    };
    build_gist_on_conn(conn.as_ref(), project_id, project_name, &query, budget_tokens)
}

/// Render the gist over a single pinned snapshot (`None` → freshness-only,
/// fail-open). All reads go through `*_with_conn` so they share `conn`'s state.
fn build_gist_on_conn(
    conn: Option<&Connection>,
    project_id: &str,
    project_name: &str,
    intent: &str,
    budget_tokens: usize,
) -> Gist {
    let fp = conn
        .and_then(|c| store::project_fingerprint_with_conn(c, project_id).ok())
        .unwrap_or_default();
    // The temporal re-rank ([DP-12]) boost shapes the "Relevant files" order, so the
    // cache key must cover the temporal state too — otherwise two index histories that
    // converge to the same (path,hash) set (same fp) but different access counts would
    // share a key yet produce different bytes (cache poisoning). Folded SEPARATELY so
    // the content fingerprint stays portable.
    let temporal = conn
        .and_then(|c| store::project_temporal_digest_with_conn(c, project_id).ok())
        .unwrap_or_default();
    // Notes are fetched BEFORE the key: the overdue-revalidate set (the one
    // wall-clock-dependent label input, gauntlet S7 `stale-note-labeled-current`)
    // must be folded into the key so a note crossing its `revalidate_after`
    // boundary rotates the key instead of re-rendering different bytes under an
    // unchanged key. Day-granular and derived from the pinned snapshot's notes,
    // so the key only moves when the overdue SET changes (a note crosses its
    // boundary / note files change) — not once per day.
    let notes = conn
        .and_then(|c| store::gist_notes_with_conn(c, project_id).ok())
        .unwrap_or_default();
    let overdue = overdue_note_ids(&notes, today_utc_ms());
    let overdue_digest = overdue.join("\u{1}");
    // ADR-020 "Recent activity": derived off the SAME pinned snapshot and folded
    // into the key as a SET digest (the overdue-digest pattern) — day-bucketed,
    // count-free lines, so a turn that adds nothing new to a day's (agent, file)
    // sets renders identical bytes under an identical key; only a genuine set
    // change (new day / new agent / new file entry) rotates the key, exactly once.
    let activity = conn
        .and_then(|c| store::recent_activity_with_conn(c, project_id, ACTIVITY_READ_ROWS).ok())
        .unwrap_or_default();
    let activity_lines = activity_day_lines(&activity);
    let activity_digest = activity_lines.join("\u{1}");
    let key = blake3::hash(
        format!(
            "{fp}\u{0}{temporal}\u{0}{overdue_digest}\u{0}{activity_digest}\u{0}{intent}\u{0}{budget_tokens}\u{0}{SCHEMA_VERSION}"
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    let file_count = conn
        .and_then(|c| store::file_count_with_conn(c, project_id).ok())
        .unwrap_or(0);
    let fp_short = fp.get(..12).unwrap_or(&fp);

    // Always-kept freshness line (never trimmed).
    let freshness = format!("# Koden Brain · {project_name} · {file_count} files · fp:{fp_short}");
    let max_chars = budget_tokens
        .saturating_mul(CHARS_PER_TOKEN)
        .max(freshness.len() + 1);
    let mut out = freshness;
    let mut sources: Vec<String> = Vec::new();

    // Both retrieval legs run up-front so the known-unknowns section can name
    // exactly which came back empty (ADR-011). (`notes` already fetched above —
    // the overdue-set digest in the cache key derives from it.)
    // Test paths are excluded (gauntlet S2 `no-test-exclusion-in-gist-search`):
    // the gist's MAX_FILES budget is agent-facing PRODUCTION context — the
    // plan.rs `exclude_tests` rationale ("planning wants the production blast
    // radius") applied to the file list, where tests/fixtures otherwise
    // lexically outrank the code they exercise and spend the whole budget.
    let hits = conn
        .and_then(|c| {
            store::search_excluding_tests_with_conn(c, Some(project_id), intent, MAX_FILES).ok()
        })
        .unwrap_or_default();

    // Known-unknowns (ADR-011): an empty retrieval leg is stated explicitly so an
    // agent can tell "the Brain looked and found nothing" from "the Brain wasn't
    // consulted". Gated on a ready, NON-EMPTY index — an unready or empty index
    // still yields the freshness-only gist, exactly as before (thin over wrong,
    // [DP-22]: an absence claim is only honest when retrieval actually ran over
    // real state). Rendered right after the always-kept freshness line, which in
    // this sequential renderer makes it trimmed last-but-one (only the freshness
    // line outranks it); pushed as ONE block so a tight budget can't strand a
    // dangling header. Derives only from key-covered state (hits/notes off the
    // pinned snapshot + the intent, all folded into the cache key) → byte-stable.
    if conn.is_some() && file_count > 0 {
        let mut unknowns: Vec<String> = Vec::new();
        if hits.is_empty() && !intent.trim().is_empty() {
            unknowns.push(format!("- No code hits for \"{}\".", intent_excerpt(intent)));
        }
        if notes.is_empty() {
            unknowns.push("- No memory notes in this project.".to_string());
        }
        if !unknowns.is_empty() {
            push_line(&mut out, max_chars, &format!("## Known unknowns\n{}", unknowns.join("\n")));
        }
    }

    // Code layer: relevant files + their top symbols.
    if !hits.is_empty() && push_line(&mut out, max_chars, "## Relevant files") {
        for h in hits.iter().take(MAX_FILES) {
            let syms = conn
                .and_then(|c| {
                    store::symbols_for_path_with_conn(c, project_id, &h.path, MAX_SYMS_PER_FILE).ok()
                })
                .unwrap_or_default();
            let line = if syms.is_empty() {
                format!("- {}", h.path)
            } else {
                format!("- {} [{}]", h.path, syms.join(", "))
            };
            if push_line(&mut out, max_chars, &line) {
                sources.push(h.path.clone());
            } else {
                break;
            }
        }
    }

    // Memory layer: top notes (titles already redacted at scan), each carrying a
    // per-claim freshness label (ADR-011) so a possibly-outdated note is visibly
    // hedged instead of only silently downranked.
    if !notes.is_empty() && push_line(&mut out, max_chars, "## Memory") {
        // A note is superseded via its own back-edge OR another note's forward edge.
        let superseding: std::collections::HashSet<&str> = notes
            .iter()
            .filter_map(|n| n.supersedes.as_deref())
            .filter(|s| !s.is_empty())
            .collect();
        let overdue_set: std::collections::HashSet<&str> = overdue.iter().copied().collect();
        for n in notes.iter().take(MAX_NOTES) {
            let label = note_freshness_label(conn, project_id, n, &superseding, &overdue_set);
            let line = match &n.note_type {
                Some(t) => format!("- {} ({t}) [{label}]", n.title),
                None => format!("- {} [{label}]", n.title),
            };
            if !push_line(&mut out, max_chars, &line) {
                break;
            }
        }
    }

    // Recent activity (ADR-020): the session trail, day-bucketed. Rendered LAST
    // (trimmed first under a tight budget) and pushed as ONE block so a cut can't
    // strand a dangling header. Derives only from key-covered state (the
    // activity-set digest above), so one key always renders the same bytes.
    if !activity_lines.is_empty() {
        push_line(
            &mut out,
            max_chars,
            &format!("## Recent activity\n{}", activity_lines.join("\n")),
        );
    }

    Gist { bytes: out, fingerprint: key, sources }
}

/// Day-bucketed, COUNT-FREE lines from newest-first activity rows (ADR-020).
/// Per UTC day (derived from each row's stored `ts_ms` — no wall clock): the
/// sorted set of agents seen on session boundaries and the sorted set of files
/// touched, both capped. Deterministic given the row set; a turn on an
/// already-covered day changes NOTHING here (the gist-key stability contract),
/// while a new day / agent / file entry moves exactly one line.
fn activity_day_lines(rows: &[store::ActivityRow]) -> Vec<String> {
    use std::collections::BTreeSet;
    // Rows arrive newest-first; BTreeMap orders days ascending — take the LAST
    // (= most recent) ACTIVITY_MAX_DAYS buckets, rendered newest first.
    let mut days: std::collections::BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> =
        std::collections::BTreeMap::new();
    for r in rows {
        let day = epoch_ms_to_utc_date(r.ts_ms);
        let bucket = days.entry(day).or_default();
        match r.kind.as_str() {
            "start" | "end" => {
                let agent = r.payload_redacted.trim();
                if !agent.is_empty() {
                    bucket.0.insert(agent.to_string());
                }
            }
            "files" => {
                let files: Vec<String> =
                    serde_json::from_str(&r.payload_redacted).unwrap_or_default();
                for f in files {
                    if !f.trim().is_empty() {
                        bucket.1.insert(f);
                    }
                }
            }
            _ => {} // turn — day presence only
        }
    }
    days.iter()
        .rev()
        .take(ACTIVITY_MAX_DAYS)
        .map(|(day, (agents, files))| {
            let mut parts: Vec<String> = vec![format!("- {day}")];
            if !agents.is_empty() {
                parts.push(
                    agents.iter().take(ACTIVITY_MAX_AGENTS).cloned().collect::<Vec<_>>().join(", "),
                );
            }
            if !files.is_empty() {
                parts.push(
                    files.iter().take(ACTIVITY_MAX_FILES).cloned().collect::<Vec<_>>().join(", "),
                );
            }
            if parts.len() == 1 {
                parts.push("session activity".to_string());
            }
            parts.join(" · ")
        })
        .collect()
}

/// UTC calendar date of an epoch-ms instant (pure — the worker's `utc_date_ymd`
/// algorithm over a stored timestamp; NOT a wall-clock read).
fn epoch_ms_to_utc_date(epoch_ms: i64) -> String {
    let days = epoch_ms.div_euclid(DAY_MS);
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

/// Per-claim freshness label for one memory note (ADR-011): `current` /
/// `possibly-stale` / `historical(superseded)`. CACHE-STABILITY IS THE HARD
/// CONSTRAINT: labels derive ONLY from key-covered state — supersession edges,
/// `created` and `revalidate_after` live in the note files (indexed → the
/// content fingerprint), the anchors' presence/touch state lives in `files`
/// (fingerprint + temporal digest), and the wall-clock-dependent OVERDUE
/// outcome is folded into the key as the overdue-set digest — so one cache key
/// always renders the same bytes (the P3 byte-identity gate).
///
/// possibly-stale (gauntlet S7 `stale-note-labeled-current` — every staleness
/// signal the doctor knows also hedges the agent-facing label) = any of:
/// - `overdue`: the note's `revalidate_after` day has passed (mirror of the
///   doctor's `stale_revalidate`; membership is key-covered via the digest);
/// - a path anchor's target is ABSENT from the index — moved or deleted, so the
///   note describes gone state (mirror of the doctor's `broken_anchor`; the
///   path set is fingerprint-covered);
/// - some path anchor's file content-changed on a LATER day than the note's
///   `created` date, observed by a live reindex (`accessed_count >= 2`: the
///   FIRST stamp is the initial index walk, which timestamps indexing — not
///   the code's last change — so counting it would mark every pre-Brain note
///   stale).
// ponytail: one deliberate ceiling remains — a schema-bump rebuild resets
// `accessed_count` to 1, forgetting prior EDIT staleness until the next real
// change — fails toward `current`, never a false stale claim (thin over wrong).
// (The former `revalidate_after` exclusion is lifted: instead of folding raw
// `today(day)` into the key — a daily cache bust — only the overdue SET is
// folded, so the key moves exactly when a label would.)
fn note_freshness_label(
    conn: Option<&Connection>,
    project_id: &str,
    n: &store::GistNote,
    superseding: &std::collections::HashSet<&str>,
    overdue: &std::collections::HashSet<&str>,
) -> &'static str {
    if matches!(n.superseded_by.as_deref(), Some(s) if !s.is_empty())
        || superseding.contains(n.id.as_str())
    {
        return "historical(superseded)";
    }
    if overdue.contains(n.id.as_str()) {
        return "possibly-stale";
    }
    let created_ms = n.created.as_deref().and_then(iso_date_to_epoch_ms);
    if let Some(c) = conn {
        for a in &n.anchors {
            let Some(p) = path_anchor(a) else { continue };
            match store::file_touch_with_conn(c, project_id, &p) {
                // Anchor target not in the index (moved/deleted → pruned): the
                // note is about gone state — hedge it. Independent of `created`
                // (S7a notes need no created date to have a broken anchor).
                Ok(None) => return "possibly-stale",
                Ok(Some((touch_ms, count))) => {
                    // Strictly after the created DAY — frontmatter dates are
                    // day-granular, so a same-day touch is not evidence the
                    // code moved past the note.
                    if let Some(created_ms) = created_ms {
                        if count >= 2 && touch_ms >= created_ms + DAY_MS {
                            return "possibly-stale";
                        }
                    }
                }
                Err(_) => {} // read error → fail toward current (thin over wrong)
            }
        }
    }
    "current"
}

/// IDs of notes whose `revalidate_after` day is strictly BEFORE `today_ms`
/// (UTC-midnight epoch ms) — the doctor's `stale_revalidate` comparison
/// (`rv < today`, day-granular) over parseable ISO dates; unparseable dates
/// fail toward current, mirroring the label policy. Deterministic given
/// (`notes`, `today_ms`): notes arrive ORDER BY id, so the digest built from
/// this is byte-stable — it is folded into the gist cache key so an overdue
/// transition rotates the key instead of tearing key↔bytes.
fn overdue_note_ids(notes: &[store::GistNote], today_ms: i64) -> Vec<&str> {
    notes
        .iter()
        .filter(|n| {
            n.revalidate_after
                .as_deref()
                .and_then(iso_date_to_epoch_ms)
                .is_some_and(|rv_ms| rv_ms < today_ms)
        })
        .map(|n| n.id.as_str())
        .collect()
}

/// Today as UTC-midnight epoch ms (day granularity — the only wall-clock read
/// in the gist; its effect on bytes is key-covered via the overdue-set digest).
fn today_utc_ms() -> i64 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    ms.div_euclid(DAY_MS) * DAY_MS
}

/// Parse the `YYYY-MM-DD` (day-granular) prefix of an ISO date/datetime to epoch
/// milliseconds (UTC midnight). Pure integer civil-date arithmetic
/// (days-from-civil) — deterministic, no wall clock, no date dependency.
///
/// `pub(crate)`: this is the ONE date parser for staleness decisions — the
/// doctor's `stale_revalidate` (memory/doctor.rs) uses it too, so a note the
/// doctor flags as overdue can never render `[current]` here (and vice versa),
/// including for malformed frontmatter dates, which fail toward current on
/// BOTH sides.
pub(crate) fn iso_date_to_epoch_ms(s: &str) -> Option<i64> {
    let date = s.trim().split(['T', ' ']).next().unwrap_or("");
    let mut it = date.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * DAY_MS)
}

/// Deterministic bounded excerpt of the intent for the known-unknowns line —
/// synthesized cold-start intents (project name + note titles) can run long.
///
/// Secret-safe (the module contract above): the intent is the ONE gist input
/// that is not index-derived, so it must be redacted at RENDER time — a pasted
/// secret never matches the (pre-redacted) index, which makes it reliably take
/// exactly this no-hits path and, unredacted, land verbatim in the persisted
/// agent prompt file. Redaction runs BEFORE truncation so a cut can't split a
/// token past detector recognition. `secrets::redact` is a deterministic pure
/// function, so byte-identity holds; the cache key still folds the RAW intent
/// (key/byte consistency is preserved — same raw intent → same key → same
/// redacted bytes).
fn intent_excerpt(intent: &str) -> String {
    let (redacted, _) = crate::modules::brain::secrets::redact(intent);
    let t = redacted.trim();
    if t.chars().count() <= EXCERPT_CHARS {
        return t.to_string();
    }
    let cut: String = t.chars().take(EXCERPT_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// Build the gist (cold-start-synthesized if `intent` is blank) and write its
/// bytes to `out_path` — the existing `--append-system-prompt` channel
/// (`~/.koden/agent-<id>.txt`). Returns the gist for the injection toast.
pub fn write_gist(
    db_path: &Path,
    project_id: &str,
    project_name: &str,
    intent: &str,
    budget_tokens: usize,
    out_path: &Path,
) -> std::io::Result<Gist> {
    let g = build_gist_auto(db_path, project_id, project_name, intent, budget_tokens);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, g.bytes.as_bytes())?;
    Ok(g)
}

/// Append `line` (with a newline) iff it fits the char budget. Returns whether it
/// was added — callers stop a layer once a line is rejected (proportional trim).
fn push_line(out: &mut String, max_chars: usize, line: &str) -> bool {
    if out.len() + 1 + line.len() <= max_chars {
        out.push('\n');
        out.push_str(line);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{
        activity_day_lines, build_gist, intent_excerpt, iso_date_to_epoch_ms, overdue_note_ids,
        DAY_MS,
    };
    use crate::modules::brain::store::{ActivityRow, SqliteIndex};

    /// The label comparison hinges on this pure date math — pin its anchor points
    /// (epoch, day step, leap-day step, datetime tolerance, garbage → None).
    #[test]
    fn iso_date_math_is_correct_and_tolerant() {
        assert_eq!(iso_date_to_epoch_ms("1970-01-01"), Some(0));
        assert_eq!(iso_date_to_epoch_ms("1970-01-02"), Some(DAY_MS));
        // Consecutive days differ by exactly one day, across a leap day.
        assert_eq!(
            iso_date_to_epoch_ms("2000-03-01").unwrap() - iso_date_to_epoch_ms("2000-02-29").unwrap(),
            DAY_MS
        );
        // A full ISO datetime parses by its date prefix (day granularity).
        assert_eq!(
            iso_date_to_epoch_ms("2026-06-20T12:34:56Z"),
            iso_date_to_epoch_ms("2026-06-20")
        );
        // Ordering sanity for the actual comparison the label uses.
        assert!(iso_date_to_epoch_ms("2026-06-21") > iso_date_to_epoch_ms("2026-06-20"));
        for bad in ["", "yesterday", "2026-13-01", "2026-00-10", "2026-01-32", "20260620"] {
            assert_eq!(iso_date_to_epoch_ms(bad), None, "must reject {bad:?}");
        }
    }

    /// Regression (gauntlet S7 `stale-note-labeled-current`, manifestation b):
    /// the overdue-revalidate set that drives both the `possibly-stale` label
    /// and the cache-key digest. Doctor-mirroring boundary semantics: overdue
    /// iff `revalidate_after < today` at DAY granularity (same-day = not yet
    /// overdue); unparseable/absent dates fail toward current; output order is
    /// the input (ORDER BY id) order → digest-deterministic.
    #[test]
    fn overdue_note_ids_day_boundary_and_garbage() {
        let note = |id: &str, rv: Option<&str>| crate::modules::brain::store::GistNote {
            id: id.into(),
            title: id.into(),
            note_type: None,
            created: None,
            revalidate_after: rv.map(Into::into),
            supersedes: None,
            superseded_by: None,
            anchors: vec![],
        };
        let today = iso_date_to_epoch_ms("2026-07-07").unwrap();
        let notes = vec![
            note("a-past", Some("2026-07-06")),      // yesterday → overdue
            note("b-today", Some("2026-07-07")),     // same day → NOT overdue (doctor `<`)
            note("c-future", Some("2026-07-08")),    // tomorrow → not
            note("d-garbage", Some("next spring")),  // unparseable → fails toward current
            note("e-none", None),                    // absent → not
            note("f-old", Some("2020-01-01T09:00")), // datetime prefix parses → overdue
        ];
        assert_eq!(overdue_note_ids(&notes, today), vec!["a-past", "f-old"]);
        assert_eq!(
            overdue_note_ids(&notes, today),
            overdue_note_ids(&notes, today),
            "deterministic (feeds the cache-key digest)"
        );
    }

    #[test]
    fn intent_excerpt_is_bounded_and_stable() {
        assert_eq!(intent_excerpt("  login flow  "), "login flow");
        let long = "x".repeat(200);
        let e = intent_excerpt(&long);
        assert!(e.chars().count() <= super::EXCERPT_CHARS + 1, "bounded: {}", e.len());
        assert!(e.ends_with('…'));
        assert_eq!(intent_excerpt(&long), e, "deterministic");
    }

    /// ADR-020: the day-line derivation is a COUNT-FREE set fold — turns only
    /// mark day presence, agents/files render as sorted capped sets, buckets are
    /// newest-first and capped, and the output is deterministic (it feeds the
    /// cache-key digest).
    #[test]
    fn activity_day_lines_are_set_folded_and_deterministic() {
        let day1 = iso_date_to_epoch_ms("2026-07-01").unwrap();
        let day2 = iso_date_to_epoch_ms("2026-07-02").unwrap();
        let row = |ts: i64, kind: &str, payload: &str| ActivityRow {
            ts_ms: ts,
            kind: kind.into(),
            payload_redacted: payload.into(),
        };
        let rows = vec![
            // newest-first, as the store returns them
            row(day2 + 500, "files", r#"["src/b.rs","src/a.rs"]"#),
            row(day2 + 400, "turn", "fix the login bug"),
            row(day2 + 300, "start", "claude"),
            row(day1 + 100, "turn", "older day presence only"),
        ];
        let lines = activity_day_lines(&rows);
        assert_eq!(
            lines,
            vec![
                "- 2026-07-02 · claude · src/a.rs, src/b.rs".to_string(),
                "- 2026-07-01 · session activity".to_string(),
            ]
        );
        // Turns are count-free: 50 more same-day turns change NOTHING.
        let mut noisy = rows.clone();
        for i in 0..50 {
            noisy.insert(0, row(day2 + 600 + i, "turn", &format!("turn {i}")));
        }
        assert_eq!(activity_day_lines(&noisy), lines, "turns must not move the set");
        assert_eq!(activity_day_lines(&rows), lines, "deterministic");
    }

    /// ADR-020 gist-key stability (the load-bearing cache contract): with a day
    /// bucket established, ingesting 50 more turns renders BYTE-IDENTICAL gist
    /// bytes under the SAME key; a NEW file-set entry rotates the key exactly
    /// once (a repeat of the same set does not rotate it again).
    #[test]
    fn gist_key_stable_over_turns_and_rotates_once_on_new_file_entry() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("i.sqlite");
        let idx = SqliteIndex::open(&db).unwrap();
        idx.index_file("p", "src/login.rs", "pub fn login() {}", "h1", 17).unwrap();
        let ts = iso_date_to_epoch_ms("2026-07-02").unwrap();

        // Establish the day bucket, then snapshot.
        idx.record_activity("p", Some(1), "turn", "first turn", ts + 1).unwrap();
        let g1 = build_gist(&db, "p", "proj", "login", 800);
        assert!(g1.bytes.contains("## Recent activity"), "section renders: {}", g1.bytes);
        assert!(g1.bytes.contains("2026-07-02"), "day bucket named: {}", g1.bytes);

        // 50 more turns on the covered day: same bytes, same key.
        for i in 0..50 {
            idx.record_activity("p", Some(1), "turn", &format!("turn {i}"), ts + 2 + i).unwrap();
        }
        let g2 = build_gist(&db, "p", "proj", "login", 800);
        assert_eq!(g2.bytes, g1.bytes, "turns that change no rendered content keep the bytes");
        assert_eq!(g2.fingerprint, g1.fingerprint, "…and must NOT rotate the key");

        // A NEW file-set entry rotates the key once…
        idx.record_activity("p", None, "files", r#"["src/login.rs"]"#, ts + 100).unwrap();
        let g3 = build_gist(&db, "p", "proj", "login", 800);
        assert_ne!(g3.fingerprint, g2.fingerprint, "a new set entry rotates the key");
        assert!(g3.bytes.contains("src/login.rs"), "{}", g3.bytes);
        // …and an identical repeat of that entry does not rotate it again.
        idx.record_activity("p", None, "files", r#"["src/login.rs"]"#, ts + 200).unwrap();
        let g4 = build_gist(&db, "p", "proj", "login", 800);
        assert_eq!(g4.fingerprint, g3.fingerprint, "set semantics: repeats don't rotate");
        assert_eq!(g4.bytes, g3.bytes);
    }

    /// Regression (gauntlet S9 `secret-intent-echoed-to-gist`): a secret-shaped
    /// intent must NOT survive into the rendered excerpt — it reliably takes the
    /// known-unknowns no-hits path and would otherwise persist verbatim to the
    /// agent prompt file. Redaction must run BEFORE truncation (a probe placed
    /// past the cut must still be caught by whole-intent redaction), and benign
    /// intents must pass through untouched (no over-redaction).
    #[test]
    fn intent_excerpt_redacts_secret_shaped_intent() {
        let probe = "sk-ProbeEcho991Zx8Kt5Rm7Vb4Np2Cj6L";
        let e = intent_excerpt(&format!("why does auth fail with {probe}?"));
        assert!(!e.contains(probe), "secret echoed into excerpt: {e}");
        assert!(e.contains("REDACTED"), "redaction marker missing: {e}");
        // Redact-before-truncate: the secret STRADDLES the EXCERPT_CHARS cut. A
        // truncate-first implementation would clip the token mid-shape, leaving a
        // partial prefix too short for the detector — and leak it. Whole-intent
        // redaction (then truncation) never renders any byte of the token.
        let pad = "p".repeat(super::EXCERPT_CHARS - 10);
        let straddling = format!("{pad} {probe}");
        let e2 = intent_excerpt(&straddling);
        assert!(!e2.contains("sk-Probe"), "truncation leaked a secret prefix: {e2}");
        // Deterministic (byte-identity gate) and no over-redaction of benign text.
        assert_eq!(intent_excerpt(&straddling), e2);
        assert_eq!(intent_excerpt("stripe checkout flow"), "stripe checkout flow");
    }
}
