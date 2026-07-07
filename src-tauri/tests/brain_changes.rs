//! `detect_changes` integration — REAL temp git repos (via `GitRepoFixture`)
//! mapped against a real SQLite index: working/staged/both surfaces, rename
//! both-sides, not-in-index files, untracked exclusion (v1 ceiling), non-git
//! roots (soft skip), and run-to-run determinism.

mod common;

use std::path::PathBuf;

use common::{git_available, GitRepoFixture};
use koden_lib::modules::brain::store::{
    detect_changes_readonly, AffectedFile, DetectMode, SqliteIndex,
};

const PID: &str = "chg";

fn skip_if_no_git() -> bool {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return true;
    }
    false
}

/// Committed repo + matching index. Import chain: b.ts → a.ts → dep.ts, so
/// dep's first-degree dependents = [a.ts] (NOT b.ts — depth 1 only) and
/// a's = [b.ts]. `notes.md` is committed but NOT indexed.
fn seeded() -> (GitRepoFixture, tempfile::TempDir, PathBuf) {
    let fx = GitRepoFixture::new();
    let files: &[(&str, &str)] = &[
        ("src/dep.ts", "export function targetSym() {}\n"),
        ("src/a.ts", "import './dep';\nexport const a = 1;\n"),
        ("src/b.ts", "import './a';\nexport const b = 1;\n"),
    ];
    for (p, c) in files {
        fx.write_file(p, c);
    }
    fx.write_file("notes.md", "# notes\n");
    fx.run_git(&["add", "."]);
    fx.run_git(&["commit", "-q", "-m", "seed"]);

    let store = tempfile::tempdir().expect("store tempdir");
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).expect("open index");
    for (p, c) in files {
        idx.index_file(PID, p, c, p, 10).expect("index_file");
    }
    idx.rebuild_edges(PID).expect("rebuild_edges");
    (fx, store, db)
}

fn affected(path: &str, in_index: bool, dependents: &[&str]) -> AffectedFile {
    AffectedFile {
        path: path.to_string(),
        in_index,
        dependents: dependents.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn working_change_maps_to_first_degree_dependents_only() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();
    fx.write_file("src/dep.ts", "export function targetSym() { return 1; }\n");

    let got = detect_changes_readonly(&db, PID, &fx.repo_path, DetectMode::Working).unwrap();
    assert_eq!(got.mode, "working");
    assert_eq!(got.skipped_reason, None);
    // Depth 1: a.ts imports dep directly; b.ts (transitive) must NOT appear.
    assert_eq!(got.affected, vec![affected("src/dep.ts", true, &["src/a.ts"])]);

    // The staged surface is untouched: empty affected, NOT a skip.
    let staged = detect_changes_readonly(&db, PID, &fx.repo_path, DetectMode::Staged).unwrap();
    assert_eq!(staged.mode, "staged");
    assert_eq!(staged.affected, Vec::new());
    assert_eq!(staged.skipped_reason, None);
}

#[test]
fn staged_change_and_both_union_dedupes_deterministically() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();
    // Staged: a.ts. Working: dep.ts + a second (post-add) edit of a.ts — a.ts
    // is now on BOTH surfaces and must appear exactly once in `both`.
    fx.write_file("src/a.ts", "import './dep';\nexport const a = 2;\n");
    fx.run_git(&["add", "src/a.ts"]);
    fx.write_file("src/a.ts", "import './dep';\nexport const a = 3;\n");
    fx.write_file("src/dep.ts", "export function targetSym() { return 2; }\n");

    let staged = detect_changes_readonly(&db, PID, &fx.repo_path, DetectMode::Staged).unwrap();
    assert_eq!(staged.affected, vec![affected("src/a.ts", true, &["src/b.ts"])]);

    let both = detect_changes_readonly(&db, PID, &fx.repo_path, DetectMode::Both).unwrap();
    assert_eq!(both.mode, "both");
    // Union, deduped, path asc.
    assert_eq!(
        both.affected,
        vec![
            affected("src/a.ts", true, &["src/b.ts"]),
            affected("src/dep.ts", true, &["src/a.ts"]),
        ]
    );

    // Deterministic across runs (byte-identical result).
    let again = detect_changes_readonly(&db, PID, &fx.repo_path, DetectMode::Both).unwrap();
    assert_eq!(both, again);
}

#[test]
fn rename_reports_both_sides() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();
    // `git mv` stages a rename; `--no-renames` decomposes it so BOTH the old
    // (still-indexed) and new (not-yet-indexed) paths appear.
    fx.run_git(&["mv", "src/b.ts", "src/renamed.ts"]);

    let got = detect_changes_readonly(&db, PID, &fx.repo_path, DetectMode::Staged).unwrap();
    assert_eq!(got.skipped_reason, None);
    assert_eq!(
        got.affected,
        vec![
            affected("src/b.ts", true, &[]),        // nothing imports b
            affected("src/renamed.ts", false, &[]), // not indexed yet
        ]
    );
}

#[test]
fn diffed_file_missing_from_index_and_untracked_excluded() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();
    fx.write_file("notes.md", "# notes\nchanged\n"); // committed, never indexed
    fx.write_file("src/brand_new.ts", "export const n = 1;\n"); // untracked

    let got = detect_changes_readonly(&db, PID, &fx.repo_path, DetectMode::Both).unwrap();
    // Untracked file absent (v1 ceiling: `git diff` sees tracked content only).
    assert_eq!(got.affected, vec![affected("notes.md", false, &[])]);
    assert_eq!(got.skipped_reason, None);
}

#[test]
fn non_git_root_soft_skips_never_errors() {
    if skip_if_no_git() {
        return;
    }
    let (_fx, _store, db) = seeded();
    let plain = tempfile::tempdir().expect("plain tempdir");

    let got = detect_changes_readonly(&db, PID, plain.path(), DetectMode::Both).unwrap();
    assert_eq!(got.mode, "both");
    assert_eq!(got.affected, Vec::new());
    assert_eq!(got.skipped_reason.as_deref(), Some("not-a-git-repo"));
}

#[test]
fn mode_parse_accepts_exactly_the_three_surfaces() {
    assert_eq!(DetectMode::parse("working"), Some(DetectMode::Working));
    assert_eq!(DetectMode::parse("staged"), Some(DetectMode::Staged));
    assert_eq!(DetectMode::parse("both"), Some(DetectMode::Both));
    // Invalid strings are a caller error at the command layer.
    assert_eq!(DetectMode::parse("Working"), None);
    assert_eq!(DetectMode::parse("untracked"), None);
    assert_eq!(DetectMode::parse(""), None);
}
