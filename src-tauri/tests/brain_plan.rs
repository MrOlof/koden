//! `plan_context` composition — the one-call planning bundle over a real index
//! (+ a real temp git repo for the happy path): leg caps flow through (search
//! 15 / impact 150 + exclude_tests), per-leg advisories with the OTHER legs
//! intact (missing target, non-git root), a zero-advisory git-backed path, and
//! run-to-run determinism. Composition only — each leg's own behavior is
//! covered by its own suite (brain_changes, sqlite impact/search tests).

mod common;

use std::path::PathBuf;

use common::{git_available, GitRepoFixture};
use koden_lib::modules::brain::store::{plan_context_readonly, PlanAdvisory, SqliteIndex};

const PID: &str = "plan";

fn skip_if_no_git() -> bool {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return true;
    }
    false
}

fn adv(subtool: &str, reason: &str) -> PlanAdvisory {
    PlanAdvisory { subtool: subtool.to_string(), reason: reason.to_string() }
}

/// Index-only fixture (no git): 20 files matching the task text "planner"
/// (> the 15-hit search cap), `targetSym` in src/dep.ts with 160 direct
/// importers (> the 150-row impact cap) + one test-convention importer that
/// `exclude_tests` must drop.
fn seeded_index() -> (tempfile::TempDir, PathBuf) {
    let store = tempfile::tempdir().expect("store tempdir");
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).expect("open index");
    let put = |p: &str, c: &str| {
        idx.index_file(PID, p, c, p, c.len() as i64).expect("index_file");
    };
    for i in 0..20 {
        put(
            &format!("src/planner_{i:02}.ts"),
            &format!("export const planner{i} = 1; // planner notes\n"),
        );
    }
    put("src/dep.ts", "export function targetSym() {}\n");
    for i in 0..160 {
        put(&format!("src/imp_{i:03}.ts"), "import './dep';\n");
    }
    put("src/dep.test.ts", "import './dep';\n");
    idx.rebuild_edges(PID).expect("rebuild_edges");
    (store, db)
}

#[test]
fn caps_flow_through_search_and_impact_legs() {
    if skip_if_no_git() {
        return;
    }
    let (_store, db) = seeded_index();
    let plain = tempfile::tempdir().expect("plain root");

    let got = plan_context_readonly(&db, PID, plain.path(), "planner", Some("targetSym".into()));
    assert_eq!(got.task, "planner");
    assert_eq!(got.target.as_deref(), Some("targetSym"));
    assert_eq!(got.search_hits.len(), 15, "search leg capped at 15");

    let imp = got.impact.expect("impact present when a target is given");
    assert_eq!(imp.direction, "upstream");
    assert_eq!(imp.defined_in, vec!["src/dep.ts"]);
    assert_eq!(imp.rows.len(), 150, "impact leg capped at 150");
    assert!(imp.truncated);
    assert_eq!(imp.result_total, 160, "test-convention importer excluded BEFORE the cap");
    assert!(
        imp.rows.iter().all(|r| r.path != "src/dep.test.ts"),
        "exclude_tests wired through the bundle"
    );

    // Failure isolation: the non-git root soft-skips ONLY the changes leg —
    // one advisory, and the two legs above still produced results.
    assert_eq!(got.advisories, vec![adv("changes", "not-a-git-repo")]);
    assert_eq!(got.changes.mode, "both");
    assert!(got.changes.affected.is_empty());
    assert_eq!(got.changes.skipped_reason.as_deref(), Some("not-a-git-repo"));
}

#[test]
fn missing_target_advises_impact_and_keeps_other_legs() {
    if skip_if_no_git() {
        return;
    }
    let (_store, db) = seeded_index();
    let plain = tempfile::tempdir().expect("plain root");

    let got = plan_context_readonly(&db, PID, plain.path(), "planner", None);
    assert_eq!(got.target, None);
    assert!(got.impact.is_none(), "no target → impact skipped, not attempted");
    // Fixed leg order: search (ok), changes (non-git skip), impact (no target).
    assert_eq!(
        got.advisories,
        vec![adv("changes", "not-a-git-repo"), adv("impact", "no-target")]
    );
    assert!(!got.search_hits.is_empty(), "search leg unaffected by the other legs' skips");
}

#[test]
fn git_backed_happy_path_has_no_advisories() {
    if skip_if_no_git() {
        return;
    }
    let fx = GitRepoFixture::new();
    let files: &[(&str, &str)] = &[
        ("src/dep.ts", "export function targetSym() {}\n"),
        ("src/a.ts", "import './dep';\nexport const planner = 1;\n"),
    ];
    for (p, c) in files {
        fx.write_file(p, c);
    }
    fx.run_git(&["add", "."]);
    fx.run_git(&["commit", "-q", "-m", "seed"]);

    let store = tempfile::tempdir().expect("store tempdir");
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).expect("open index");
    for (p, c) in files {
        idx.index_file(PID, p, c, p, c.len() as i64).expect("index_file");
    }
    idx.rebuild_edges(PID).expect("rebuild_edges");

    // Working-tree edit → the changes leg maps dep + its first-degree dependent.
    fx.write_file("src/dep.ts", "export function targetSym() { return 1; }\n");

    let got =
        plan_context_readonly(&db, PID, &fx.repo_path, "planner", Some("targetSym".into()));
    assert_eq!(got.advisories, Vec::new(), "all three legs ran");
    assert_eq!(got.changes.mode, "both");
    assert_eq!(got.changes.affected.len(), 1);
    assert_eq!(got.changes.affected[0].path, "src/dep.ts");
    assert!(got.changes.affected[0].in_index);
    assert_eq!(got.changes.affected[0].dependents, vec!["src/a.ts"]);
    let imp = got.impact.expect("impact present");
    assert_eq!(imp.defined_in, vec!["src/dep.ts"]);
    assert!(!got.search_hits.is_empty(), "'planner' matches indexed content");
}

#[test]
fn bundle_is_deterministic_across_runs() {
    if skip_if_no_git() {
        return;
    }
    let (_store, db) = seeded_index();
    let plain = tempfile::tempdir().expect("plain root");
    let go = || {
        serde_json::to_string(&plan_context_readonly(
            &db,
            PID,
            plain.path(),
            "planner",
            Some("targetSym".into()),
        ))
        .expect("serialize bundle")
    };
    assert_eq!(go(), go(), "byte-identical bundle across runs");
}
