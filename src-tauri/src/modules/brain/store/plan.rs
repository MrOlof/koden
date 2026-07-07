//! `plan_context` — one read-only bundle for agent planning: task-text search
//! hits + git-diff affected files + (when a target symbol is given) upstream
//! impact. Pure COMPOSITION over the existing readers (`search_readonly`,
//! `detect_changes_readonly`, `code_impact_readonly`) — no new query logic,
//! no writes, deterministic because every leg is.
//!
//! Per-subtool failure isolation: a leg that cannot run becomes an
//! `advisories[]` entry (`{ subtool, reason }`) and the rest of the bundle
//! still returns — fail-open, like every other command reader. Advisory
//! `reason` vocabulary is closed: "index-unavailable" (a leg's read failed),
//! "no-target" (impact skipped — no symbol given), "index-not-ready" (command
//! layer, before the DB exists), or a `DetectedChanges::skipped_reason` class
//! mirrored from the changes leg ("not-a-git-repo" | "git-not-available" |
//! "git-error").

use std::path::Path;

use crate::modules::brain::ast::{Impact, ImpactDirection};
use crate::modules::brain::Hit;

use super::changes::{detect_changes_readonly, DetectMode, DetectedChanges};

// ponytail: composition ceilings, not tunables — a planning bundle is a
// briefing, not a report. 15 hits ≈ one screen; 150 impact rows is already
// past what a planner reads row-by-row (`truncated` flags the cut).
const PLAN_SEARCH_CAP: usize = 15;
const PLAN_IMPACT_MAX_RESULTS: usize = 150;
const PLAN_IMPACT_DEPTH: usize = 5;

/// One isolated subtool failure/skip: which leg, and the reason class.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PlanAdvisory {
    pub subtool: String,
    pub reason: String,
}

impl PlanAdvisory {
    fn new(subtool: &str, reason: &str) -> Self {
        Self { subtool: subtool.to_string(), reason: reason.to_string() }
    }
}

/// `brain_plan_context` result: `{ task, target?, search_hits, changes,
/// impact?, advisories }`. `impact` is present only when a target symbol was
/// given AND its read succeeded; every absence is explained in `advisories`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlanContext {
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub search_hits: Vec<Hit>,
    pub changes: DetectedChanges,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<Impact>,
    pub advisories: Vec<PlanAdvisory>,
}

impl PlanContext {
    /// Command-layer fail-open shape when the index doesn't exist yet: nothing
    /// is attempted, every leg is advised (fixed order: search, changes, impact).
    pub fn skipped(task: String, target: Option<String>, reason: &str) -> Self {
        let impact_reason = if target.is_some() { reason } else { "no-target" };
        Self {
            task,
            target,
            search_hits: Vec::new(),
            changes: DetectedChanges::skipped(DetectMode::Both, reason),
            impact: None,
            advisories: vec![
                PlanAdvisory::new("search", reason),
                PlanAdvisory::new("changes", reason),
                PlanAdvisory::new("impact", impact_reason),
            ],
        }
    }
}

/// Compose the planning bundle: (a) lexical search for the task text (cap 15),
/// (b) git-diff affected files (both surfaces), (c) upstream impact of the
/// target symbol (max 150 rows, tests excluded) when one is given. Read-only
/// end to end; infallible — failures land in `advisories` (fixed leg order:
/// search, changes, impact).
pub fn plan_context_readonly(
    db_path: &Path,
    project: &str,
    project_root: &Path,
    task: &str,
    target: Option<String>,
) -> PlanContext {
    let mut advisories = Vec::new();

    // (a) Task-text search, capped.
    let search_hits =
        match super::sqlite::search_readonly(db_path, Some(project), task, PLAN_SEARCH_CAP) {
            Ok(hits) => hits,
            Err(e) => {
                log::debug!("plan_context search leg soft error: {e}");
                advisories.push(PlanAdvisory::new("search", "index-unavailable"));
                Vec::new()
            }
        };

    // (b) Diff mapping, both surfaces. A soft skip (non-git root, no git) is
    // already encoded in `skipped_reason`; mirror it into `advisories` so a
    // planner reads ONE list for "what's missing" — the bundle keeps the
    // skipped `changes` shape too (`mode` intact, `affected` empty).
    let changes = detect_changes_readonly(db_path, project, project_root, DetectMode::Both)
        .unwrap_or_else(|e| {
            log::debug!("plan_context changes leg soft error: {e}");
            DetectedChanges::skipped(DetectMode::Both, "index-unavailable")
        });
    if let Some(reason) = changes.skipped_reason.as_deref() {
        advisories.push(PlanAdvisory::new("changes", reason));
    }

    // (c) Upstream impact of the target — only when a target symbol is given.
    let impact = match target.as_deref() {
        None => {
            advisories.push(PlanAdvisory::new("impact", "no-target"));
            None
        }
        Some(symbol) => match super::sqlite::code_impact_readonly(
            db_path,
            project,
            symbol,
            PLAN_IMPACT_DEPTH,
            ImpactDirection::Upstream,
            PLAN_IMPACT_MAX_RESULTS,
            true, // exclude_tests: planning wants the production blast radius
        ) {
            Ok(impact) => Some(impact),
            Err(e) => {
                log::debug!("plan_context impact leg soft error: {e}");
                advisories.push(PlanAdvisory::new("impact", "index-unavailable"));
                None
            }
        },
    };

    PlanContext { task: task.to_string(), target, search_hits, changes, impact, advisories }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_advises_every_leg_and_no_target_wins_for_impact() {
        let s = PlanContext::skipped("t".into(), None, "index-not-ready");
        assert_eq!(
            s.advisories,
            vec![
                PlanAdvisory::new("search", "index-not-ready"),
                PlanAdvisory::new("changes", "index-not-ready"),
                PlanAdvisory::new("impact", "no-target"),
            ]
        );
        assert!(s.search_hits.is_empty() && s.impact.is_none());
        assert_eq!(s.changes.mode, "both");
        assert_eq!(s.changes.skipped_reason.as_deref(), Some("index-not-ready"));

        // With a target the impact leg was genuinely blocked by the index.
        let s2 = PlanContext::skipped("t".into(), Some("sym".into()), "index-not-ready");
        assert_eq!(s2.advisories[2], PlanAdvisory::new("impact", "index-not-ready"));
    }
}
