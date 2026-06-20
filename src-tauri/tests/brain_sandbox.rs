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

use koden_lib::modules::brain::memory::doctor::run_doctor;
use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::store::{
    list_notes_readonly, list_proposals_readonly, SearchIndex, SqliteIndex,
};
use koden_lib::modules::brain::worker::{index_changed, index_dir};

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

/// P1 freshness gate: an out-of-band edit reindexes only the changed paths
/// (modified re-indexed, deleted pruned, added inserted) — untouched files stay.
#[test]
fn incremental_reindex_touches_only_changed_paths() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "a.ts", b"export const alphaToken = 1;");
    write(root, "b.ts", b"export const bravoToken = 2;");
    write(root, "c.ts", b"export const charlieToken = 3;");

    let idx = open_index(store.path());
    index_dir(&idx, PID, root);
    assert_eq!(idx.file_count(PID).unwrap(), 3);

    // Out-of-band: modify a, delete b, add d. c is NOT in the change set.
    std::fs::write(root.join("a.ts"), b"export const alphaTokenV2 = 11;").unwrap();
    std::fs::remove_file(root.join("b.ts")).unwrap();
    write(root, "d.ts", b"export const deltaToken = 4;");
    let changed = vec![root.join("a.ts"), root.join("b.ts"), root.join("d.ts")];
    let stats = index_changed(&idx, PID, root, &changed);

    assert_eq!(stats.pruned, 1, "deleted file pruned");
    assert!(stats.indexed >= 2, "modified + added reindexed");
    assert_eq!(idx.file_count(PID).unwrap(), 3, "a + c + d");
    assert!(!idx.search(Some(PID), "alphatokenv2", 10).unwrap().is_empty());
    assert!(idx.search(Some(PID), "bravo", 10).unwrap().is_empty(), "deleted pruned");
    assert!(!idx.search(Some(PID), "delta", 10).unwrap().is_empty());
    // c was never in the change set and remains searchable.
    assert!(!idx.search(Some(PID), "charlie", 10).unwrap().is_empty());
}

/// P1 memory: a `.koden-memory/*.md` note is parsed into the structured store,
/// listable for cards, AND lexically searchable through the same query path.
#[test]
fn memory_notes_parsed_stored_and_searchable() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/app.ts", b"export function startApp() {}");
    write(
        root,
        ".koden-memory/adr-sqlite.md",
        b"---\nid: adr-sqlite\ntype: decision\nstatus: accepted\n---\n\
          # Use SQLite for the index\n\nWe store the brain index in one SQLite file.\n",
    );

    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);
    assert_eq!(scan_project_memory(&idx, PID, root), 1, "one note scanned");
    assert_eq!(idx.note_count(PID).unwrap(), 1);

    // structured listing (cards / review inbox)
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, "adr-sqlite");
    assert_eq!(notes[0].note_type.as_deref(), Some("decision"));
    assert_eq!(notes[0].title, "Use SQLite for the index");

    // searchable zero-token via the same path the code uses (walk indexed the .md)
    assert!(
        !idx.search(Some(PID), "sqlite index", 10).unwrap().is_empty(),
        "seeded note must be searchable"
    );
}

/// P1 safety: a secret-shaped memory-note title is redacted before it reaches the
/// notes table (the table is a form of indexing — CONCEPT §7.1).
#[test]
fn memory_note_title_secret_is_redacted() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(
        root,
        ".koden-memory/leak.md",
        b"---\nid: leak\ntitle: token sk-ABCD1234efgh5678IJKL9012mnop\n---\nbody\n",
    );
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    scan_project_memory(&idx, PID, root);
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    assert_eq!(notes.len(), 1);
    assert!(!notes[0].title.contains("sk-ABCD"), "secret leaked in title: {}", notes[0].title);
    assert!(notes[0].title.contains("REDACTED"));
}

/// P1: a deleted memory note is pruned from the structured store on the next scan.
#[test]
fn deleted_note_is_pruned() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, ".koden-memory/n1.md", b"---\nid: n1\ntitle: One\n---\nx\n");
    write(root, ".koden-memory/n2.md", b"---\nid: n2\ntitle: Two\n---\ny\n");
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    scan_project_memory(&idx, PID, root);
    assert_eq!(idx.note_count(PID).unwrap(), 2);
    std::fs::remove_file(root.join(".koden-memory/n2.md")).unwrap();
    scan_project_memory(&idx, PID, root);
    assert_eq!(idx.note_count(PID).unwrap(), 1, "deleted note pruned");
    assert!(list_notes_readonly(&db, Some(PID)).unwrap().iter().all(|n| n.id != "n2"));
}

/// P1 gate: a doctor finding becomes a proposal the user can approve/reject, and
/// a rejected proposal does not reappear on the next doctor pass.
#[test]
fn doctor_queues_proposals_and_reject_sticks() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/here.rs", b"pub fn here() {}");
    // A note with NO type (→ missing_type) and a path anchor that isn't indexed
    // (→ broken_anchor).
    write(
        root,
        ".koden-memory/n1.md",
        b"---\nid: n1\nanchors:\n  - src/gone.rs\n---\n# Note one\nbody\n",
    );
    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);
    scan_project_memory(&idx, PID, root);

    assert!(run_doctor(&idx, PID, Some("2026-06-20"), 1000) >= 1, "doctor queues proposals");
    let pending = list_proposals_readonly(&db, Some(PID)).unwrap();
    assert!(pending.iter().any(|p| p.title.contains("no type")), "missing_type proposal");
    let anchor = pending
        .iter()
        .find(|p| p.title.contains("anchor not found"))
        .expect("broken_anchor proposal");

    // Reject the broken-anchor proposal → it must not return.
    assert!(idx.resolve_proposal(PID, &anchor.signature, true).unwrap());
    run_doctor(&idx, PID, Some("2026-06-20"), 2000);
    let pending2 = list_proposals_readonly(&db, Some(PID)).unwrap();
    assert!(
        !pending2.iter().any(|p| p.title.contains("anchor not found")),
        "rejected proposal must not reappear"
    );
    assert!(
        pending2.iter().any(|p| p.title.contains("no type")),
        "un-rejected proposal stays pending"
    );
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
