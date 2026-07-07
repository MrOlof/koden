//! `temporal` integration — REAL temp git repos (via `GitRepoFixture`) mapped
//! against a real SQLite index: churn ranking across several commits,
//! since-window filtering, limit clamping, changed_between across anchors
//! (incl. a file not in the index), invalid/unknown anchors (soft skip),
//! flag-injection rejection BEFORE git runs, and non-git roots.

mod common;

use std::path::PathBuf;

use common::{git_available, GitRepoFixture};
use koden_lib::modules::brain::store::{
    changed_between_readonly, hotspots_readonly, ChangedFile, HotspotRow, SqliteIndex,
};

const PID: &str = "tmp";

fn skip_if_no_git() -> bool {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return true;
    }
    false
}

/// Commit with a PINNED author+committer date so `--since` windows are
/// deterministic (`--since` filters on committer date).
fn commit_at(fx: &GitRepoFixture, date: &str, msg: &str) {
    let out = std::process::Command::new("git")
        .args(["commit", "-q", "-m", msg])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(&fx.repo_path)
        .output()
        .expect("git on PATH");
    assert!(out.status.success(), "git commit failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// Three date-pinned commits over four indexed files + one committed-but-not-
/// indexed file. Churn: a.ts in all 3 commits, b.ts in 2, c.ts/d.ts in 1
/// (the seed). notes.md churns in every commit but is NEVER indexed.
///
///   2020-01-01  seed: a, b, c, d, notes
///   2021-01-01  touch a, b, notes
///   2022-01-01  touch a, notes
fn seeded() -> (GitRepoFixture, tempfile::TempDir, PathBuf) {
    let fx = GitRepoFixture::new();
    let files: &[(&str, &str)] = &[
        ("src/a.ts", "export const a = 0;\n"),
        ("src/b.ts", "export const b = 0;\n"),
        ("src/c.ts", "export const c = 0;\n"),
        ("src/d.ts", "export const d = 0;\n"),
    ];
    for (p, c) in files {
        fx.write_file(p, c);
    }
    fx.write_file("notes.md", "# notes v0\n");
    fx.run_git(&["add", "."]);
    commit_at(&fx, "2020-01-01T10:00:00 +0000", "seed");

    fx.write_file("src/a.ts", "export const a = 1;\n");
    fx.write_file("src/b.ts", "export const b = 1;\n");
    fx.write_file("notes.md", "# notes v1\n");
    fx.run_git(&["add", "."]);
    commit_at(&fx, "2021-01-01T10:00:00 +0000", "touch a b");

    fx.write_file("src/a.ts", "export const a = 2;\n");
    fx.write_file("notes.md", "# notes v2\n");
    fx.run_git(&["add", "."]);
    commit_at(&fx, "2022-01-01T10:00:00 +0000", "touch a");

    let store = tempfile::tempdir().expect("store tempdir");
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).expect("open index");
    for (p, c) in files {
        idx.index_file(PID, p, c, p, 10).expect("index_file");
    }
    (fx, store, db)
}

fn hotspot(path: &str, commits: u32) -> HotspotRow {
    HotspotRow { path: path.to_string(), commits }
}

fn changed(path: &str, in_index: bool) -> ChangedFile {
    ChangedFile { path: path.to_string(), in_index }
}

#[test]
fn hotspots_rank_by_churn_desc_then_path_asc_index_only() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();

    let got = hotspots_readonly(&db, PID, &fx.repo_path, None, None).unwrap();
    assert_eq!(got.skipped_reason, None);
    // notes.md churns most (3 commits) but is NOT indexed → absent. Tie at
    // 1 commit (c vs d) breaks path asc.
    assert_eq!(
        got.rows,
        vec![
            hotspot("src/a.ts", 3),
            hotspot("src/b.ts", 2),
            hotspot("src/c.ts", 1),
            hotspot("src/d.ts", 1),
        ]
    );

    // Deterministic across runs (byte-identical result).
    let again = hotspots_readonly(&db, PID, &fx.repo_path, None, None).unwrap();
    assert_eq!(got, again);
}

#[test]
fn hotspots_since_window_filters_commits() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();

    // Window opens after the seed commit: only the 2021 + 2022 commits count.
    let got = hotspots_readonly(&db, PID, &fx.repo_path, Some("2020-06-01"), None).unwrap();
    assert_eq!(got.skipped_reason, None);
    assert_eq!(got.rows, vec![hotspot("src/a.ts", 2), hotspot("src/b.ts", 1)]);

    // Window opens in the future: zero commits is an EMPTY result, not a skip.
    let empty = hotspots_readonly(&db, PID, &fx.repo_path, Some("2999-01-01"), None).unwrap();
    assert_eq!(empty.rows, Vec::new());
    assert_eq!(empty.skipped_reason, None);
}

#[test]
fn hotspots_limit_clamps_low_and_high() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();

    // 0 clamps up to 1 → exactly the top row.
    let one = hotspots_readonly(&db, PID, &fx.repo_path, None, Some(0)).unwrap();
    assert_eq!(one.rows, vec![hotspot("src/a.ts", 3)]);

    // Absurdly high clamps to the 200 ceiling — with 4 indexed files that is
    // simply "all rows", identical to the default.
    let all = hotspots_readonly(&db, PID, &fx.repo_path, None, Some(usize::MAX)).unwrap();
    assert_eq!(all.rows.len(), 4);

    let two = hotspots_readonly(&db, PID, &fx.repo_path, None, Some(2)).unwrap();
    assert_eq!(two.rows, vec![hotspot("src/a.ts", 3), hotspot("src/b.ts", 2)]);
}

#[test]
fn hotspots_hex_named_file_is_not_a_commit_anchor() {
    if skip_if_no_git() {
        return;
    }
    // Regression: `--name-only` emits paths on bare lines, and a tracked file
    // named exactly like a full hex object id used to be parsed as a commit
    // anchor — every path listed after it in the same commit got attributed
    // to one fake commit, collapsing distinct-commit counts (and the hex file
    // itself always counted 0). The `%x01` sentinel anchors commits by a byte
    // git always C-quotes in path output, so both files must count both
    // commits.
    let hex = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
    let fx = GitRepoFixture::new();
    fx.write_file(hex, "v0\n");
    fx.write_file("src/a.ts", "export const a = 0;\n");
    fx.run_git(&["add", "."]);
    commit_at(&fx, "2020-01-01T10:00:00 +0000", "seed");

    fx.write_file(hex, "v1\n");
    fx.write_file("src/a.ts", "export const a = 1;\n");
    fx.run_git(&["add", "."]);
    commit_at(&fx, "2021-01-01T10:00:00 +0000", "touch both");

    let store = tempfile::tempdir().expect("store tempdir");
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).expect("open index");
    idx.index_file(PID, hex, "v1\n", hex, 10).expect("index_file");
    idx.index_file(PID, "src/a.ts", "export const a = 1;\n", "src/a.ts", 10)
        .expect("index_file");

    let got = hotspots_readonly(&db, PID, &fx.repo_path, None, None).unwrap();
    assert_eq!(got.skipped_reason, None);
    // Tie at 2 commits breaks path asc ("da…" < "src/a.ts").
    assert_eq!(got.rows, vec![hotspot(hex, 2), hotspot("src/a.ts", 2)]);
}

#[test]
fn hotspots_rejects_flag_injection_since_before_git_runs() {
    if skip_if_no_git() {
        return;
    }
    let (_fx, _store, db) = seeded();
    let plain = tempfile::tempdir().expect("plain tempdir");

    // A NON-git root would soft-skip "not-a-git-repo" if git ran — getting
    // "invalid-since" proves the shape gate fired first.
    for bad in ["-oProxyCommand=calc", "--all", "", "a\nb", &"x".repeat(300)] {
        let got = hotspots_readonly(&db, PID, plain.path(), Some(bad), None).unwrap();
        assert_eq!(got.rows, Vec::new(), "since={bad:?}");
        assert_eq!(got.skipped_reason.as_deref(), Some("invalid-since"), "since={bad:?}");
    }
}

#[test]
fn changed_between_maps_anchor_ranges_onto_index() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();

    // `to` omitted → HEAD: the 2022 commit touched a.ts + notes.md, and
    // notes.md (committed, never indexed) reports in_index=false. Path asc.
    let got = changed_between_readonly(&db, PID, &fx.repo_path, "HEAD~1", None).unwrap();
    assert_eq!(got.skipped_reason, None);
    assert_eq!((got.from.as_str(), got.to.as_str()), ("HEAD~1", "HEAD"));
    assert_eq!(got.rows, vec![changed("notes.md", false), changed("src/a.ts", true)]);

    // Explicit anchors: HEAD~2..HEAD~1 is the 2021 commit (a, b, notes).
    let mid = changed_between_readonly(&db, PID, &fx.repo_path, "HEAD~2", Some("HEAD~1")).unwrap();
    assert_eq!(
        mid.rows,
        vec![changed("notes.md", false), changed("src/a.ts", true), changed("src/b.ts", true)]
    );

    // Empty range: identical anchors is an EMPTY result, not a skip.
    let same = changed_between_readonly(&db, PID, &fx.repo_path, "HEAD", Some("HEAD")).unwrap();
    assert_eq!(same.rows, Vec::new());
    assert_eq!(same.skipped_reason, None);
}

#[test]
fn changed_between_rejects_flag_injection_before_git_runs() {
    if skip_if_no_git() {
        return;
    }
    let (_fx, _store, db) = seeded();
    let plain = tempfile::tempdir().expect("plain tempdir");

    // Same negative control as hotspots: a non-git root would report
    // "not-a-git-repo" if git ran — "invalid-anchor" proves it never did.
    for bad in ["-oProxyCommand=calc", "--output=owned", "", &"x".repeat(300)] {
        let got = changed_between_readonly(&db, PID, plain.path(), bad, None).unwrap();
        assert_eq!(got.rows, Vec::new(), "from={bad:?}");
        assert_eq!(got.skipped_reason.as_deref(), Some("invalid-anchor"), "from={bad:?}");
    }
    // The `to` side is gated identically.
    let got = changed_between_readonly(&db, PID, plain.path(), "HEAD", Some("-R")).unwrap();
    assert_eq!(got.skipped_reason.as_deref(), Some("invalid-anchor"));
}

#[test]
fn changed_between_unknown_anchor_soft_skips_with_git_class() {
    if skip_if_no_git() {
        return;
    }
    let (fx, _store, db) = seeded();

    // Well-shaped but unresolvable: git judges it → its error class, no panic.
    let got =
        changed_between_readonly(&db, PID, &fx.repo_path, "no-such-ref-xyz", None).unwrap();
    assert_eq!(got.rows, Vec::new());
    assert_eq!(got.skipped_reason.as_deref(), Some("unknown-anchor"));
}

#[test]
fn non_git_root_soft_skips_never_errors() {
    if skip_if_no_git() {
        return;
    }
    let (_fx, _store, db) = seeded();
    let plain = tempfile::tempdir().expect("plain tempdir");

    let hs = hotspots_readonly(&db, PID, plain.path(), None, None).unwrap();
    assert_eq!(hs.rows, Vec::new());
    assert_eq!(hs.skipped_reason.as_deref(), Some("not-a-git-repo"));

    let cb = changed_between_readonly(&db, PID, plain.path(), "HEAD~1", None).unwrap();
    assert_eq!(cb.rows, Vec::new());
    assert_eq!(cb.skipped_reason.as_deref(), Some("not-a-git-repo"));
}
