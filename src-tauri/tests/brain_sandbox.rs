//! Koden Brain — deterministic, offline, $0 sandbox (BUILD-PROMPT §6.5).
//!
//! These integration tests materialize real fixture repos in a scratch TempDir
//! and drive the **real** indexing pipeline (`brain::worker::index_dir` → walk →
//! binary-sniff → blake3 → secrets redact → FTS index → reconcile) — not unit
//! mocks. They prove, from an actual run: secret exclusion (denylist + redaction),
//! generated/vendored/binary/oversized exclusion, incremental == full rebuild, and
//! fingerprint determinism. Everything uses an explicit TempDir; nothing touches
//! the real `~/.koden`.

use std::path::Path;

use koden_lib::modules::brain::store::{SearchIndex, SqliteIndex};
use koden_lib::modules::brain::worker::index_dir;

const PID: &str = "fix";

fn write(root: &Path, rel: &str, content: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
    std::fs::write(p, content).expect("write");
}

fn open_index(dir: &Path) -> SqliteIndex {
    SqliteIndex::open(&dir.join("index.sqlite")).expect("open index")
}

fn hits(idx: &SqliteIndex, q: &str) -> usize {
    idx.search(Some(PID), q, 20).expect("search").len()
}

#[test]
fn indexes_real_source_excludes_generated_vendored_binary_oversized() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();

    // Real source (must index)
    write(root, "src/auth/login.ts", b"export function loginHandler() { return true; }");
    write(root, "src/util.rs", b"pub fn rustUtilHelper() -> bool { true }");
    // Excluded: vendored / build / generated dirs (pruned during walk)
    write(root, "node_modules/pkg/index.js", b"const vendoredSentinel = 1;");
    write(root, "dist/bundle.js", b"var distSentinel = 2;");
    write(root, "generated/gen.ts", b"const genSentinel = 3;");
    // Excluded: binary (NUL sniff)
    write(root, "assets/blob.dat", b"binarySentinel\x00\x00\x00more");
    // Excluded: oversized (>1MB)
    let mut big = b"bigFileSentinel ".to_vec();
    big.resize(1_200_000, b'a');
    write(root, "huge.txt", &big);

    let idx = open_index(store.path());
    let stats = index_dir(&idx, PID, root);

    assert_eq!(stats.indexed, 2, "only the 2 real source files index");
    assert_eq!(idx.file_count(PID).unwrap(), 2);

    // real source is searchable
    assert!(hits(&idx, "login") >= 1, "login.ts should be found");
    assert!(hits(&idx, "util") >= 1, "util.rs should be found");

    // every excluded artifact is absent from the index
    for sentinel in [
        "vendoredSentinel",
        "distSentinel",
        "genSentinel",
        "binarySentinel",
        "bigFileSentinel",
    ] {
        assert_eq!(hits(&idx, sentinel), 0, "excluded content leaked: {sentinel}");
    }
}

/// HARD GATE, real-run proof (BUILD-PROMPT §6.5): planted secrets in a real repo
/// are never retrievable from the index, while normal code remains searchable.
#[test]
fn secrets_gate_proven_from_real_walk() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();

    // Denylisted file — must never be read into the index.
    write(root, ".env", b"API_SECRET=supersecretvalue123\nDB_PASSWORD=hunter2pass\n");
    // Indexed file with an inline secret (redacted) + normal code (kept).
    write(
        root,
        "src/config.ts",
        b"const apiKey = \"sk-LIVE1234abcdEFGH5678ijklMNOP\";\n\
          export function parseConfig(raw: string) { return JSON.parse(raw); }\n",
    );

    let idx = open_index(store.path());
    index_dir(&idx, PID, root);

    // Nothing secret is retrievable.
    for leaked in [
        "supersecretvalue123", // from the denylisted .env (never read)
        "hunter2pass",
        "sk",                  // redacted inline key
        "live1234abcdefgh5678ijklmnop",
    ] {
        assert_eq!(hits(&idx, leaked), 0, "secret retrievable from index: {leaked}");
    }
    // Surrounding code survives — redaction is surgical, .env-skip is whole-file.
    assert!(hits(&idx, "parse config") >= 1, "non-secret code must remain searchable");
}

/// §12 gate + DoD property: an incrementally-reconciled index equals a full
/// rebuild over the same final on-disk state (same fingerprint, count, results).
#[test]
fn incremental_equals_full_rebuild() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();

    write(root, "a.ts", b"export const alphaToken = 1;");
    write(root, "b.ts", b"export const bravoToken = 2;");
    write(root, "c.ts", b"export const charlieToken = 3;");

    // Incremental index: build, then mutate on disk, then reconcile.
    let inc_store = tempfile::tempdir().unwrap();
    let inc = open_index(inc_store.path());
    index_dir(&inc, PID, root);
    std::fs::write(root.join("a.ts"), b"export const alphaTokenV2 = 11;").unwrap(); // modify
    std::fs::remove_file(root.join("b.ts")).unwrap(); // delete
    write(root, "d.ts", b"export const deltaToken = 4;"); // add
    let inc_stats = index_dir(&inc, PID, root);
    assert!(inc_stats.pruned >= 1, "reconcile must prune the deleted file");

    // Full rebuild: fresh index over the SAME final disk state.
    let full_store = tempfile::tempdir().unwrap();
    let full = open_index(full_store.path());
    index_dir(&full, PID, root);

    assert_eq!(
        inc.file_count(PID).unwrap(),
        full.file_count(PID).unwrap(),
        "file counts diverge"
    );
    assert_eq!(
        inc.project_fingerprint(PID).unwrap(),
        full.project_fingerprint(PID).unwrap(),
        "incremental fingerprint != full-rebuild fingerprint"
    );
    // The deleted file's content is gone from the incremental index.
    assert!(inc.search(Some(PID), "bravo", 10).unwrap().is_empty(), "deleted file still matches");
    // The modified + added files are present.
    assert!(!inc.search(Some(PID), "alphatokenv2", 10).unwrap().is_empty());
    assert!(!inc.search(Some(PID), "delta", 10).unwrap().is_empty());
}

/// Fingerprint is deterministic across rebuilds (a P3 cache-stability proxy).
#[test]
fn fingerprint_is_deterministic() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "x.rs", b"pub fn one() {}");
    write(root, "y.rs", b"pub fn two() {}");

    let s1 = tempfile::tempdir().unwrap();
    let i1 = open_index(s1.path());
    index_dir(&i1, PID, root);

    let s2 = tempfile::tempdir().unwrap();
    let i2 = open_index(s2.path());
    index_dir(&i2, PID, root);

    assert_eq!(
        i1.project_fingerprint(PID).unwrap(),
        i2.project_fingerprint(PID).unwrap(),
        "fingerprint must be identical for identical content"
    );
}
