//! `detect_changes` — map the project's git diff onto the index: which indexed
//! files changed, and who imports them (first-degree `code_edges` dependents).
//! File-granular NorrGit adoption (CONCEPT §4.1b sibling of `code_impact`).
//!
//! Read-only end to end: git is invoked as a plain `diff --name-only` probe
//! (`--no-pager`, no write flags) and the index is queried over a read-only
//! connection. A non-git root — or ANY git failure — is a SOFT result
//! (`skipped_reason` set, empty `affected`), never a hard error: the brain is
//! fail-open and a project without git history is a normal state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use crate::modules::brain::registry::fold_case;

/// Which git diff surface to inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectMode {
    /// Unstaged worktree changes (`git diff`).
    Working,
    /// Staged changes (`git diff --cached`).
    Staged,
    /// Union of both surfaces.
    Both,
}

impl DetectMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "working" => Some(Self::Working),
            "staged" => Some(Self::Staged),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Staged => "staged",
            Self::Both => "both",
        }
    }
}

/// One diff-touched file mapped against the index.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AffectedFile {
    /// The index's stored path when `in_index`, else the normalized diff path.
    pub path: String,
    pub in_index: bool,
    /// First-degree dependents (`code_edges` dst→src, depth 1), deduped,
    /// path asc. Always empty when `in_index` is false.
    pub dependents: Vec<String>,
}

/// `brain_detect_changes` result: `{ mode, affected, skipped_reason? }`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct DetectedChanges {
    pub mode: String,
    /// Deterministic: path asc, deduped across diff surfaces.
    pub affected: Vec<AffectedFile>,
    /// Set (with empty `affected`) when the diff could not be taken:
    /// "not-a-git-repo" | "git-not-available" | "git-error" — or, from the
    /// command layer, "index-not-ready" | "index-unavailable".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

impl DetectedChanges {
    pub fn skipped(mode: DetectMode, reason: &str) -> Self {
        Self {
            mode: mode.as_str().to_string(),
            affected: Vec::new(),
            skipped_reason: Some(reason.to_string()),
        }
    }
}

/// Run one read-only `git diff --name-only` probe and return the touched paths
/// (forward-slash normalized). `Err` carries the skipped-reason class.
fn git_diff_names(project_root: &Path, staged: bool) -> Result<Vec<String>, String> {
    let mut cmd = Command::new("git");
    cmd.arg("--no-pager").arg("diff");
    if staged {
        cmd.arg("--cached");
    }
    // `-z`: NUL-separated raw paths — no core.quotepath escaping to undo.
    // `--no-renames`: a rename decomposes into delete(old) + add(new) so BOTH
    //   sides appear in `--name-only` output (rename detection would list only
    //   the post-image name). Simplest correct rename handling for mapping a
    //   diff onto the index: the old path still has the row/edges, the new one
    //   correctly reports `in_index: false` until reindexed.
    // `--relative`: paths relative to the project root even when the root is a
    //   subdirectory of the git repo (changes outside the root are excluded) —
    //   matching the index's root-relative path representation.
    cmd.args(["--name-only", "-z", "--no-renames", "--relative"]);
    cmd.current_dir(project_root);
    let out = cmd.output().map_err(|_| "git-not-available".to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).to_ascii_lowercase();
        return Err(if err.contains("not a git repository") {
            "not-a-git-repo".to_string()
        } else {
            "git-error".to_string()
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.replace('\\', "/"))
        .collect())
}

/// Map the project's git diff to affected indexed files + their first-degree
/// dependents. Deterministic output (path asc); soft-skips when git can't run.
pub fn detect_changes_readonly(
    db_path: &Path,
    project: &str,
    project_root: &Path,
    mode: DetectMode,
) -> rusqlite::Result<DetectedChanges> {
    // ponytail: untracked files are NOT included in v1 — `git diff` only sees
    // tracked content, and a brand-new file usually has no edges to map yet.
    // If wanted later: union `git ls-files --others --exclude-standard`.
    let legs: &[bool] = match mode {
        DetectMode::Working => &[false],
        DetectMode::Staged => &[true],
        DetectMode::Both => &[false, true],
    };
    // BTreeSet: union across surfaces is deduped + path-asc by construction.
    let mut diff_paths: BTreeSet<String> = BTreeSet::new();
    for &staged in legs {
        match git_diff_names(project_root, staged) {
            Ok(paths) => diff_paths.extend(paths),
            Err(reason) => return Ok(DetectedChanges::skipped(mode, &reason)),
        }
    }

    let conn = super::sqlite::open_readonly(db_path)?;
    // Case-fold-keyed lookup (same fold as the registry's `resolve`, ADR-010
    // cluster 7: Windows only): git records the casing from `git add` time,
    // which can drift from the on-disk casing the index stored. Matching via
    // the fold, then emitting the STORED spelling, keeps output paths joinable
    // against every other brain surface.
    let mut indexed: BTreeMap<String, String> = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT path FROM files WHERE project_id=?1")?;
        let it = stmt.query_map([project], |r| r.get::<_, String>(0))?;
        for p in it {
            let p = p?;
            indexed.insert(fold_case(&p), p);
        }
    }
    let mut dep_stmt = conn.prepare(
        "SELECT DISTINCT src_path FROM code_edges WHERE project_id=?1 AND dst_path=?2 ORDER BY src_path",
    )?;

    // Keyed by OUTPUT path: two diff spellings resolving to one indexed row
    // dedupe here, and BTreeMap gives the path-asc output order for free.
    let mut affected: BTreeMap<String, AffectedFile> = BTreeMap::new();
    for diff_path in &diff_paths {
        let (path, in_index) = match indexed.get(&fold_case(diff_path)) {
            Some(stored) => (stored.clone(), true),
            None => (diff_path.clone(), false),
        };
        if affected.contains_key(&path) {
            continue;
        }
        // DISTINCT + ORDER BY in the statement = deduped, deterministic.
        let dependents = if in_index {
            let it = dep_stmt.query_map((project, path.as_str()), |r| r.get::<_, String>(0))?;
            let mut v = Vec::new();
            for x in it {
                v.push(x?);
            }
            v
        } else {
            Vec::new()
        };
        affected.insert(path.clone(), AffectedFile { path, in_index, dependents });
    }

    Ok(DetectedChanges {
        mode: mode.as_str().to_string(),
        affected: affected.into_values().collect(),
        skipped_reason: None,
    })
}
