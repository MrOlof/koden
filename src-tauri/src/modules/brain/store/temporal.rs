//! `temporal` — git-backed read-only temporal queries (ADR-013's recommended
//! first step: shell out to git at query time, store NOTHING). Two surfaces:
//! `hotspots` (churn ranking — distinct commits touching each indexed path)
//! and `changed_between` (paths touched between two anchors, mapped onto the
//! index). Stored bitemporal capture stays the upgrade path if a query ever
//! needs GRAPH state (not file state) at a past commit.
//!
//! Same soft-skip contract as `detect_changes`: a non-git root — or ANY git
//! failure — is a SOFT result (`skipped_reason` set, empty rows), never a
//! hard error.
//!
//! Anchor/`--since` hygiene: caller-supplied values are passed to git as
//! plain args (no shell), behind a loose shape gate that runs BEFORE git:
//! leading '-' (flag injection: `-oProxyCommand=...`), empty, over-length,
//! or control chars are rejected outright. Well-shaped-but-unknown anchors
//! are git's to judge → "unknown-anchor".

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::modules::brain::registry::fold_case;

use super::changes::{indexed_paths_folded, run_git_readonly};

// ponytail: hard ceilings, not tunables. 2000 commits is a churn window, not
// an archaeology dig — and it bounds the buffered `git log` output. 128 chars
// covers any sane rev/refname/approxidate; longer is garbage or an attack.
const HOTSPOT_MAX_COMMITS: usize = 2000;
const MAX_GIT_ARG_LEN: usize = 128;
const HOTSPOT_DEFAULT_LIMIT: usize = 25;
const HOTSPOT_MAX_LIMIT: usize = 200;

/// Loose shape gate for a caller-supplied git argument value (revision anchor
/// or `--since` date). NOT git-syntax validation — only what keeps the value a
/// plain argument: non-empty, bounded length, no leading '-' (would parse as a
/// flag), no control chars. Everything else git judges (soft-skip on failure).
fn is_safe_git_arg(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_GIT_ARG_LEN
        && !s.starts_with('-')
        && !s.chars().any(char::is_control)
}

/// Byte prefixing each `%H` in the hotspots log (`--pretty=format:%x01%H`).
/// `--name-only` emits paths on bare lines, so a tracked file named exactly
/// like a full hex object id would be indistinguishable from a commit line by
/// shape alone. The sentinel is unambiguous: git C-quotes control bytes in
/// path output regardless of `core.quotepath`, so a raw `\x01` can only ever
/// start a commit line.
const COMMIT_SENTINEL: char = '\x01';

/// One churn-ranked indexed path.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HotspotRow {
    /// The index's STORED path spelling (joinable across brain surfaces).
    pub path: String,
    /// Distinct commits touching the path within the window.
    pub commits: u32,
}

/// `brain_hotspots` result: `{ rows, skipped_reason? }`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Hotspots {
    /// Deterministic: commits desc, then path asc; capped at the limit.
    /// Only paths present in the index appear (churn of unindexed files is
    /// invisible to ranking by design — the brain ranks what it knows).
    pub rows: Vec<HotspotRow>,
    /// Set (with empty `rows`) when the log could not be taken:
    /// "invalid-since" (shape gate, git never ran) | "not-a-git-repo" |
    /// "git-not-available" | "git-error" — or, from the command layer,
    /// "index-not-ready" | "index-unavailable".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

impl Hotspots {
    pub fn skipped(reason: &str) -> Self {
        Self { rows: Vec::new(), skipped_reason: Some(reason.to_string()) }
    }
}

/// One diff-touched path between two anchors.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ChangedFile {
    /// The index's stored path when `in_index`, else the normalized diff path.
    pub path: String,
    pub in_index: bool,
}

/// `brain_changed_between` result: `{ from, to, rows, skipped_reason? }`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ChangedBetween {
    pub from: String,
    /// Resolved anchor ("HEAD" when the caller omitted it).
    pub to: String,
    /// Deterministic: path asc, deduped.
    pub rows: Vec<ChangedFile>,
    /// Set (with empty `rows`) when the diff could not be taken:
    /// "invalid-anchor" (shape gate, git never ran) | "unknown-anchor" |
    /// "not-a-git-repo" | "git-not-available" | "git-error" — or, from the
    /// command layer, "index-not-ready" | "index-unavailable".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

impl ChangedBetween {
    pub fn skipped(from: &str, to: &str, reason: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            rows: Vec::new(),
            skipped_reason: Some(reason.to_string()),
        }
    }
}

/// Rank indexed paths by churn: DISTINCT commits touching each path within a
/// bounded `git log --name-only` window (ceiling 2000 commits), optionally
/// narrowed by `--since=<since>`. Deterministic (commits desc, path asc);
/// soft-skips when git can't run. `limit` defaults to 25, clamped 1..200.
pub fn hotspots_readonly(
    db_path: &Path,
    project: &str,
    project_root: &Path,
    since: Option<&str>,
    limit: Option<usize>,
) -> rusqlite::Result<Hotspots> {
    let limit = limit.unwrap_or(HOTSPOT_DEFAULT_LIMIT).clamp(1, HOTSPOT_MAX_LIMIT);
    if let Some(s) = since {
        // Shape gate FIRST — a bad `since` never reaches git (flag injection).
        if !is_safe_git_arg(s) {
            return Ok(Hotspots::skipped("invalid-since"));
        }
    }
    let max_count = format!("--max-count={HOTSPOT_MAX_COMMITS}");
    // `core.quotepath=false`: raw UTF-8 paths instead of octal-escaped quoting
    //   (log has no `-z`-clean equivalent of the diff probe's NUL framing).
    // `--no-renames`/`--relative`: same rationale as the diff probe — both
    //   rename sides listed, paths root-relative like the index.
    let mut args: Vec<&str> = vec![
        "-c",
        "core.quotepath=false",
        "log",
        &max_count,
        "--name-only",
        "--no-renames",
        "--relative",
        "--pretty=format:%x01%H",
    ];
    let since_arg;
    if let Some(s) = since {
        since_arg = format!("--since={s}");
        args.push(&since_arg);
    }
    let stdout = match run_git_readonly(project_root, &args) {
        Ok(s) => s,
        Err(reason) => return Ok(Hotspots::skipped(&reason)),
    };

    let conn = super::sqlite::open_readonly(db_path)?;
    let indexed = indexed_paths_folded(&conn, project)?;

    // Stream the log: a sentinel-prefixed line anchors the current commit,
    // every other non-empty line is a path. Distinct-commit counting
    // via a (stored path, commit) set — two casings of one indexed path in the
    // same commit can't double-count, and two spellings across commits merge
    // onto the stored spelling.
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut current: Option<&str> = None;
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(hash) = line.strip_prefix(COMMIT_SENTINEL) {
            current = Some(hash);
            continue;
        }
        let Some(commit) = current else { continue };
        let path = line.replace('\\', "/");
        if let Some(stored) = indexed.get(&fold_case(&path)) {
            seen.insert((stored.clone(), commit.to_string()));
        }
    }
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for (path, _) in seen {
        *counts.entry(path).or_insert(0) += 1;
    }
    let mut rows: Vec<HotspotRow> = counts
        .into_iter()
        .map(|(path, commits)| HotspotRow { path, commits })
        .collect();
    // BTreeMap iteration is path-asc, and the sort is stable → the tie-break
    // (commits desc, then path asc) holds by construction.
    rows.sort_by_key(|r| std::cmp::Reverse(r.commits));
    rows.truncate(limit);
    Ok(Hotspots { rows, skipped_reason: None })
}

/// Paths touched between two anchors (`git diff --name-only <from>..<to>`,
/// `to` defaulting to HEAD), each mapped onto the index. Deterministic (path
/// asc); soft-skips on bad/unknown anchors or when git can't run.
pub fn changed_between_readonly(
    db_path: &Path,
    project: &str,
    project_root: &Path,
    from: &str,
    to: Option<&str>,
) -> rusqlite::Result<ChangedBetween> {
    let to = to.unwrap_or("HEAD");
    // Shape gate FIRST — a bad anchor never reaches git (flag injection).
    if !is_safe_git_arg(from) || !is_safe_git_arg(to) {
        return Ok(ChangedBetween::skipped(from, to, "invalid-anchor"));
    }
    // One `A..B` arg: both anchors are gated above, and concatenation with
    // literal dots cannot re-create a leading '-'.
    let range = format!("{from}..{to}");
    // Same probe shape as `detect_changes`' diff legs (`-z` NUL framing).
    let args =
        ["diff", "--name-only", "-z", "--no-renames", "--relative", range.as_str()];
    let stdout = match run_git_readonly(project_root, &args) {
        Ok(s) => s,
        Err(reason) => return Ok(ChangedBetween::skipped(from, to, &reason)),
    };

    let conn = super::sqlite::open_readonly(db_path)?;
    let indexed = indexed_paths_folded(&conn, project)?;

    // Keyed by OUTPUT path: two diff spellings resolving to one indexed row
    // dedupe here, and BTreeMap gives the path-asc output order for free.
    let mut rows: BTreeMap<String, bool> = BTreeMap::new();
    for raw in stdout.split('\0').filter(|s| !s.is_empty()) {
        let path = raw.replace('\\', "/");
        match indexed.get(&fold_case(&path)) {
            Some(stored) => {
                rows.insert(stored.clone(), true);
            }
            None => {
                rows.entry(path).or_insert(false);
            }
        }
    }
    Ok(ChangedBetween {
        from: from.to_string(),
        to: to.to_string(),
        rows: rows
            .into_iter()
            .map(|(path, in_index)| ChangedFile { path, in_index })
            .collect(),
        skipped_reason: None,
    })
}
