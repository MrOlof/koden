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

use koden_lib::modules::brain::gist::synth::synthesize_intent;
use koden_lib::modules::brain::gist::{build_gist, build_gist_auto, write_gist};
use koden_lib::modules::brain::memory::doctor::run_doctor;
use koden_lib::modules::brain::memory::scan_project_memory;
use koden_lib::modules::brain::curate::{curate_act_only, curate_with_client, CurationReason};
use koden_lib::modules::brain::memory::proposal::ProposalAction;
use koden_lib::modules::brain::reflect::{
    reflect_with_client, ReflectClient, ReflectConfig, ReflectReason, ReflectResponse,
};
use koden_lib::modules::brain::resume::{
    gc_resume_dir, record_event, recover_all, resume_command, ResumePlan, ResumeRecord, SessionKey,
};
use koden_lib::modules::brain::store::{
    code_impact_readonly, get_symbol_readonly, list_notes_readonly, list_proposals_readonly,
    semantic_meta_readonly, SearchIndex, SqliteIndex,
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

/// P3 gate: an unchanged relaunch yields a BYTE-IDENTICAL gist (prompt-cache-safe),
/// and a content edit changes the cache key.
#[test]
fn gist_byte_identical_on_unchanged_relaunch() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/auth/login.ts", b"export function loginHandler() {}");
    write(
        root,
        ".koden-memory/n.md",
        b"---\nid: n\ntype: decision\ntitle: Auth approach\n---\nbody",
    );
    let db = store.path().join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);
    scan_project_memory(&idx, PID, root);

    let g1 = build_gist(&db, PID, "proj", "login", 400);
    let g2 = build_gist(&db, PID, "proj", "login", 400);
    assert_eq!(g1.bytes, g2.bytes, "unchanged relaunch must be byte-identical");
    assert_eq!(g1.fingerprint, g2.fingerprint);
    assert!(g1.bytes.contains("src/auth/login.ts"), "relevant file present: {}", g1.bytes);
    assert!(g1.bytes.contains("Auth approach"), "memory note present");

    // a content edit changes the fingerprint cache key (cache correctly invalidated)
    std::fs::write(root.join("src/auth/login.ts"), b"export function loginHandlerV2() {}").unwrap();
    index_changed(&idx, PID, root, &[root.join("src/auth/login.ts")]);
    let g3 = build_gist(&db, PID, "proj", "login", 400);
    assert_ne!(g3.fingerprint, g1.fingerprint, "content change → new cache key");
}

/// P3.2: cold-start synthesis is deterministic + drives a non-thin gist; the
/// write path lands the gist bytes in the agent file; auto-synth stays byte-stable.
#[test]
fn gist_cold_start_synth_and_write() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/payments/checkout.ts", b"export function createStripeCheckout() {}");
    write(
        root,
        ".koden-memory/n.md",
        b"---\nid: n\ntype: decision\ntitle: Stripe checkout flow\n---\nbody",
    );
    let db = store.path().join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);
    scan_project_memory(&idx, PID, root);

    // synthesis is deterministic and includes the project name + note title.
    let q1 = synthesize_intent(&db, PID, "shop");
    let q2 = synthesize_intent(&db, PID, "shop");
    assert_eq!(q1, q2);
    assert!(q1.contains("shop") && q1.contains("Stripe checkout flow"), "synth: {q1}");

    // auto-synth (blank intent) produces a non-thin, byte-stable gist.
    let g1 = build_gist_auto(&db, PID, "shop", "", 400);
    let g2 = build_gist_auto(&db, PID, "shop", "", 400);
    assert_eq!(g1.bytes, g2.bytes, "auto-synth gist must be byte-stable");
    assert!(g1.bytes.contains("checkout.ts"), "synth surfaced the stripe file: {}", g1.bytes);

    // write path lands the gist in the agent file.
    let out = store.path().join("agent-7.txt");
    let g3 = write_gist(&db, PID, "shop", "stripe checkout", 400, &out).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), g3.bytes);
    assert!(g3.bytes.contains("checkout.ts"));
}

/// P3 gate under concurrency: while the single writer toggles the index between
/// two states, a reader building gists must never observe a TORN snapshot — i.e.
/// for any cache key, the bytes are always identical. Before the single-snapshot
/// fix, build_gist read the fingerprint (key) and the body over separate
/// connections, so a key could map to two different byte strings.
#[test]
fn gist_cache_key_stable_under_concurrent_writes() {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/auth/login.ts", b"export function loginHandler() {}");
    let db = store.path().join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);

    // Writer thread: toggle one "login"-matching file in/out, committing each step.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_w = Arc::clone(&stop);
    let writer = std::thread::spawn(move || {
        let mut present = false;
        while !stop_w.load(Ordering::Relaxed) {
            if present {
                idx.remove_file(PID, "src/auth/loginExtra.ts").ok();
            } else {
                idx.index_file(
                    PID,
                    "src/auth/loginExtra.ts",
                    "export function loginExtra() {}",
                    "h-extra",
                    33,
                )
                .ok();
            }
            present = !present;
        }
    });

    // Reader: build gists in a tight loop, asserting key→bytes is a function.
    let mut seen: HashMap<String, String> = HashMap::new();
    for _ in 0..400 {
        let g = build_gist(&db, PID, "proj", "login", 400);
        if let Some(prev) = seen.get(&g.fingerprint) {
            assert_eq!(prev, &g.bytes, "torn snapshot: one cache key, two gist bodies");
        } else {
            seen.insert(g.fingerprint.clone(), g.bytes.clone());
        }
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();

    // Sanity: the writer actually churned the index, so the reader observed more
    // than one state (otherwise the no-tear assertion would be vacuous).
    assert!(seen.len() >= 2, "expected to observe >1 index state, saw {}", seen.len());
}

fn ts_chain(root: &Path) {
    write(root, "src/a.ts", b"export function alpha() {}");
    write(root, "src/b.ts", b"import { alpha } from './a';\nexport function bravo() { alpha(); }");
    write(root, "src/c.ts", b"import { bravo } from './b';\nexport function charlie() { bravo(); }");
}

/// P2 marquee: `code_impact` returns the AST reverse-import closure (tiered).
#[test]
fn code_impact_reverse_import_closure() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    ts_chain(root);
    let db = store.path().join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);

    let sym = get_symbol_readonly(&db, PID, "alpha").unwrap();
    assert_eq!(sym.len(), 1);
    assert_eq!(sym[0].path, "src/a.ts");
    assert_eq!(sym[0].kind, "function");

    // a defines alpha; b imports a; c imports b → dependents = {b, c}.
    let imp = code_impact_readonly(&db, PID, "alpha", 5).unwrap();
    assert_eq!(imp.defined_in, vec!["src/a.ts"]);
    assert!(imp.ast_dependents.contains(&"src/b.ts".to_string()), "{:?}", imp.ast_dependents);
    assert!(imp.ast_dependents.contains(&"src/c.ts".to_string()), "{:?}", imp.ast_dependents);
}

/// P2 gate (strengthened): incremental relink == full rebuild across ADD + DELETE
/// + RENAME + MODIFY in one batch — not just a single modify.
#[test]
fn incremental_relink_equals_full_rebuild_add_delete_rename() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    ts_chain(root); // a, b(→a), c(→b)

    let inc_store = tempfile::tempdir().unwrap();
    let inc = SqliteIndex::open(&inc_store.path().join("i.sqlite")).unwrap();
    index_dir(&inc, PID, root);

    // add d(→c); delete a; rename b→b2; modify c to import ./b2.
    write(root, "src/d.ts", b"import { charlie } from './c';\nexport function delta() {}");
    std::fs::remove_file(root.join("src/a.ts")).unwrap();
    std::fs::rename(root.join("src/b.ts"), root.join("src/b2.ts")).unwrap();
    std::fs::write(
        root.join("src/c.ts"),
        b"import { bravo } from './b2';\nexport function charlie() { bravo(); }",
    )
    .unwrap();
    index_changed(
        &inc,
        PID,
        root,
        &[
            root.join("src/d.ts"),
            root.join("src/a.ts"),
            root.join("src/b.ts"),
            root.join("src/b2.ts"),
            root.join("src/c.ts"),
        ],
    );

    let full_store = tempfile::tempdir().unwrap();
    let full = SqliteIndex::open(&full_store.path().join("i.sqlite")).unwrap();
    index_dir(&full, PID, root);

    assert_eq!(
        inc.project_edges(PID).unwrap(),
        full.project_edges(PID).unwrap(),
        "edges diverge (add/delete/rename)"
    );
    assert_eq!(
        inc.project_node_keys(PID).unwrap(),
        full.project_node_keys(PID).unwrap(),
        "nodes diverge (add/delete/rename)"
    );
}

/// A getter and setter on the same line are both kept (start_col disambiguates
/// the node PK — they used to collide and one was silently dropped).
#[test]
fn same_line_getter_setter_both_indexed() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/c.ts", b"export class C { get x() { return 1; } set x(v: number) {} }");
    let db = store.path().join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);
    let syms = get_symbol_readonly(&db, PID, "x").unwrap();
    assert_eq!(syms.len(), 2, "getter + setter both kept: {syms:?}");
}

/// P2 gate: an incrementally-relinked graph equals a full rebuild over the same
/// final on-disk state (identical nodes + edges).
#[test]
fn incremental_relink_equals_full_rebuild() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    ts_chain(root);

    // Incremental: index, mutate b.ts on disk, relink only the changed file.
    let inc_store = tempfile::tempdir().unwrap();
    let inc = SqliteIndex::open(&inc_store.path().join("i.sqlite")).unwrap();
    index_dir(&inc, PID, root);
    std::fs::write(
        root.join("src/b.ts"),
        b"import { alpha } from './a';\nexport function bravo2() { alpha(); }",
    )
    .unwrap();
    index_changed(&inc, PID, root, &[root.join("src/b.ts")]);

    // Full rebuild over the SAME final disk state.
    let full_store = tempfile::tempdir().unwrap();
    let full = SqliteIndex::open(&full_store.path().join("i.sqlite")).unwrap();
    index_dir(&full, PID, root);

    assert_eq!(
        inc.project_edges(PID).unwrap(),
        full.project_edges(PID).unwrap(),
        "edges diverge between incremental relink and full rebuild"
    );
    assert_eq!(
        inc.project_node_keys(PID).unwrap(),
        full.project_node_keys(PID).unwrap(),
        "nodes diverge between incremental relink and full rebuild"
    );
}

/// Fingerprint is deterministic across rebuilds (a P3 cache-stability proxy).
#[test]
fn semantic_header_persisted_empty_in_v1() {
    // P5 gate: a fresh brain.sqlite carries the embedderId header, empty (no
    // embedder compiled in the default build).
    let store = tempfile::tempdir().unwrap();
    let db = store.path().join("i.sqlite");
    let _idx = SqliteIndex::open(&db).unwrap();
    assert_eq!(semantic_meta_readonly(&db).unwrap(), (String::new(), 0));
}

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

// ---------------------------------------------------------------------------
// P4 — budgeted reflect, driven against the REAL index + a deterministic fake
// LLM (§13.22 fake provider contract). Proves the whole pipeline (digest →
// budget reserve → call → reconcile → map → enqueue) and every gate offline/$0.
// ---------------------------------------------------------------------------

/// Deterministic, contract-compatible fake of the Anthropic client. Counts calls
/// so the "spends nothing → zero requests" gates are provable.
struct FakeClient {
    calls: std::sync::atomic::AtomicUsize,
    resp: Result<ReflectResponse, String>,
    seen_user: std::sync::Mutex<String>,
}

impl FakeClient {
    fn ok(json: &str, input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            resp: Ok(ReflectResponse { json_text: json.into(), input_tokens, output_tokens }),
            seen_user: std::sync::Mutex::new(String::new()),
        }
    }
    fn failing(msg: &str) -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            resp: Err(msg.into()),
            seen_user: std::sync::Mutex::new(String::new()),
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// The exact user message that reached the client (post redact-before-send).
    fn last_user(&self) -> String {
        self.seen_user.lock().unwrap().clone()
    }
}

impl ReflectClient for FakeClient {
    fn complete(&self, _m: &str, _s: &str, user: &str, _t: u32) -> Result<ReflectResponse, String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.seen_user.lock().unwrap() = user.to_string();
        self.resp.clone()
    }
}

/// Build a real index + project carrying one memory note (non-empty corpus).
fn index_with_note(work: &Path, store: &Path) -> (std::path::PathBuf, SqliteIndex) {
    write(work, ".koden-memory/auth.md", b"---\nid: auth\ntype: decision\ntitle: Auth approach\nstatus: active\n---\nbody\n");
    write(work, "src/app.ts", b"export function startApp() {}");
    let db = store.join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, work);
    scan_project_memory(&idx, PID, work);
    (db, idx)
}

const TWO_PROPOSALS: &str = r#"{"proposals":[
  {"kind":"insight","title":"Consolidate auth notes","detail":"two notes overlap","scope":"project","confidence":"high","evidence":["a.rs"]},
  {"kind":"stale","title":"Archive legacy decision","detail":"superseded","scope":"project","confidence":"medium"}
]}"#;

/// Gate 1: ceiling 0 (default) → Disabled, spends nothing, ZERO requests.
#[test]
fn reflect_disabled_makes_no_call() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (_db, idx) = index_with_note(work.path(), store.path());
    let fake = FakeClient::ok(TWO_PROPOSALS, 100, 100);
    let out = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    assert!(matches!(out.reason, ReflectReason::Disabled), "{:?}", out.reason);
    assert_eq!(out.spent_usd, 0.0);
    assert_eq!(fake.calls(), 0, "disabled → zero requests");
    assert_eq!(idx.budget_state().1, 0.0, "spent unchanged");
}

/// Gate 2: ceiling below the conservative estimate → OverBudget, ZERO requests.
#[test]
fn reflect_overbudget_blocks_before_call() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (_db, idx) = index_with_note(work.path(), store.path());
    idx.set_budget_ceiling(0.001, 1).unwrap(); // < est (~$0.01, output-dominated)
    let fake = FakeClient::ok(TWO_PROPOSALS, 100, 100);
    let out = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    assert!(matches!(out.reason, ReflectReason::OverBudget), "{:?}", out.reason);
    assert_eq!(fake.calls(), 0, "over-budget → zero requests");
    assert_eq!(idx.budget_state().1, 0.0, "spent unchanged");
}

/// Gate 3 (happy path): proposals enqueued into the P1 queue; spent = ACTUAL cost;
/// exactly one request; no orphaned reservation left behind.
#[test]
fn reflect_happy_path_enqueues_and_charges_actual() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (db, idx) = index_with_note(work.path(), store.path());
    idx.set_budget_ceiling(1.0, 1).unwrap();
    // Haiku: 1000*$1/Mtok + 200*$5/Mtok = $0.001 + $0.001 = $0.002.
    let fake = FakeClient::ok(TWO_PROPOSALS, 1000, 200);
    let out = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    assert!(matches!(out.reason, ReflectReason::Ok), "{:?}", out.reason);
    assert_eq!(out.proposals.len(), 2);
    assert!((out.spent_usd - 0.002).abs() < 1e-9, "actual cost, got {}", out.spent_usd);
    assert_eq!(fake.calls(), 1);
    let (_, spent) = idx.budget_state();
    assert!((spent - 0.002).abs() < 1e-9, "spent_total folds actual: {spent}");
    let props = list_proposals_readonly(&db, Some(PID)).unwrap();
    assert_eq!(props.iter().filter(|p| p.source == "reflect").count(), 2, "queued reflect proposals");
    // No reservation was orphaned (clean reconcile).
    assert_eq!(idx.sweep_orphaned_reservations(2000).unwrap(), 0, "no orphan after clean run");
}

/// Malformed model output → InvalidOutput, no proposals — but STILL charged actual
/// (a 2xx-with-garbage may have billed tokens).
#[test]
fn reflect_invalid_json_fails_open_but_charges() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (_db, idx) = index_with_note(work.path(), store.path());
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let fake = FakeClient::ok("not json at all", 1000, 200);
    let out = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    assert!(matches!(out.reason, ReflectReason::InvalidOutput), "{:?}", out.reason);
    assert_eq!(out.proposals.len(), 0);
    assert!((out.spent_usd - 0.002).abs() < 1e-9, "still charged actual");
    assert!((idx.budget_state().1 - 0.002).abs() < 1e-9);
    assert_eq!(fake.calls(), 1);
}

/// A failed call charges the ESTIMATE (default-to-charging on uncertainty), and
/// leaves no orphaned reservation.
#[test]
fn reflect_call_failure_charges_estimate() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (_db, idx) = index_with_note(work.path(), store.path());
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let fake = FakeClient::failing("connection reset");
    let out = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    match &out.reason {
        ReflectReason::CallFailed(m) => assert!(m.contains("connection reset"), "{m}"),
        other => panic!("expected CallFailed, got {other:?}"),
    }
    assert_eq!(out.proposals.len(), 0);
    assert!(out.spent_usd > 0.0, "charged the estimate");
    assert!((idx.budget_state().1 - out.spent_usd).abs() < 1e-9, "spent == estimate");
    assert_eq!(fake.calls(), 1);
    assert_eq!(idx.sweep_orphaned_reservations(2000).unwrap(), 0, "failure path reconciled the reservation");
}

/// Re-running reflect on the same corpus + same output enqueues NOTHING new
/// (dedup by proposal signature) — but still makes the call + charges.
#[test]
fn reflect_dedups_on_rerun() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (db, idx) = index_with_note(work.path(), store.path());
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let json = r#"{"proposals":[{"kind":"insight","title":"Consolidate auth notes","detail":"d","scope":"project","confidence":"high"}]}"#;
    let out1 = reflect_with_client(&idx, &FakeClient::ok(json, 100, 100), &ReflectConfig::default(), PID, None, 1000);
    assert_eq!(out1.proposals.len(), 1);
    let fake2 = FakeClient::ok(json, 100, 100);
    let out2 = reflect_with_client(&idx, &fake2, &ReflectConfig::default(), PID, None, 2000);
    assert_eq!(out2.proposals.len(), 0, "duplicate signature not re-enqueued");
    assert!(matches!(out2.reason, ReflectReason::Ok));
    assert_eq!(fake2.calls(), 1, "still calls the model");
    let props = list_proposals_readonly(&db, Some(PID)).unwrap();
    assert_eq!(props.iter().filter(|p| p.source == "reflect").count(), 1, "queue holds one");
}

// ---------------------------------------------------------------------------
// V2 Flow G — stale-ADR curation. Notes that trip the signals → detect →
// ACT-band ($0 archive) + ESCALATE-band (budget-gated LLM verdict). Archive-
// biased, curate-sourced proposals into the human-gated P1 queue.
// ---------------------------------------------------------------------------
fn curate_fixture(work: &Path, store: &Path) -> (std::path::PathBuf, SqliteIndex) {
    // new: clean. old: superseded_by new (resolves) → escalate. stacked: revalidate
    // passed + superseded_by new → ACT band ($0).
    write(work, ".koden-memory/new.md", b"---\nid: new\ntype: decision\ntitle: New approach\n---\nbody\n");
    write(work, ".koden-memory/old.md", b"---\nid: old\ntype: decision\ntitle: Old approach\nsuperseded_by: new\n---\nbody\n");
    write(
        work,
        ".koden-memory/stacked.md",
        b"---\nid: stacked\ntype: decision\ntitle: Stacked\nsuperseded_by: new\nrevalidate_after: 2000-01-01\n---\nbody\n",
    );
    let db = store.join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, work);
    scan_project_memory(&idx, PID, work);
    (db, idx)
}

#[test]
fn curate_acts_and_escalates_into_archive_proposals() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (db, idx) = curate_fixture(work.path(), store.path());
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let fake = FakeClient::ok(
        r#"{"classification":"obsolete","action":"supersede","confidence":"high","reason":"replaced by new"}"#,
        1000,
        200,
    );
    let out = curate_with_client(&idx, &fake, &ReflectConfig::default(), PID, Some("2026-01-01"), 10);
    assert!(matches!(out.reason, CurationReason::Ok), "{:?}", out.reason);
    assert_eq!(out.acted, 1, "stacked (2.5) acted with no LLM");
    assert_eq!(out.escalated, 1, "old (1.5) escalated to the LLM");
    assert_eq!(fake.calls(), 1, "exactly one paid call (the escalate band)");
    assert!((out.spent_usd - 0.002).abs() < 1e-9, "charged the escalation: {}", out.spent_usd);
    let props = list_proposals_readonly(&db, Some(PID)).unwrap();
    let curate: Vec<_> = props.iter().filter(|p| p.source == "curate").collect();
    assert_eq!(curate.len(), 2, "one ACT archive + one escalated graded proposal");
    assert!(curate.iter().any(|p| p.target_id.as_deref() == Some("stacked") && p.action == ProposalAction::Archive));
    assert!(curate.iter().any(|p| p.target_id.as_deref() == Some("old") && p.action == ProposalAction::Supersede));
}

#[test]
fn curate_escalation_disabled_still_acts_for_free() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (db, idx) = curate_fixture(work.path(), store.path());
    // ceiling 0 (default): the ACT band still runs $0; escalation is gated.
    let fake = FakeClient::ok(r#"{"classification":"obsolete","action":"archive","confidence":"high","reason":"x"}"#, 100, 100);
    let out = curate_with_client(&idx, &fake, &ReflectConfig::default(), PID, Some("2026-01-01"), 10);
    assert!(matches!(out.reason, CurationReason::Disabled), "{:?}", out.reason);
    assert_eq!(out.acted, 1, "ACT-band archive still made for free");
    assert_eq!(fake.calls(), 0, "no paid call when the ceiling is off");
    assert_eq!(out.spent_usd, 0.0);
    let curate: Vec<_> = list_proposals_readonly(&db, Some(PID)).unwrap().into_iter().filter(|p| p.source == "curate").collect();
    assert_eq!(curate.len(), 1, "only the $0 ACT archive (escalate candidate not judged)");
}

#[test]
fn curate_still_valid_verdict_yields_no_proposal() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (db, idx) = curate_fixture(work.path(), store.path());
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let fake = FakeClient::ok(r#"{"classification":"still_valid","action":"update","confidence":"high","reason":"still good"}"#, 100, 100);
    let out = curate_with_client(&idx, &fake, &ReflectConfig::default(), PID, Some("2026-01-01"), 10);
    let curate: Vec<_> = list_proposals_readonly(&db, Some(PID)).unwrap().into_iter().filter(|p| p.source == "curate").collect();
    assert!(curate.iter().all(|p| p.target_id.as_deref() != Some("old")), "still-valid → not proposed");
    assert_eq!(out.escalated, 1, "it was still escalated (judged), just not proposed");
}

#[test]
fn curate_act_only_no_key_makes_free_proposals() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (db, idx) = curate_fixture(work.path(), store.path());
    let out = curate_act_only(&idx, PID, Some("2026-01-01"), 10);
    assert!(matches!(out.reason, CurationReason::NoKey), "{:?}", out.reason);
    assert_eq!(out.acted, 1);
    assert_eq!(out.spent_usd, 0.0);
    let curate: Vec<_> = list_proposals_readonly(&db, Some(PID)).unwrap().into_iter().filter(|p| p.source == "curate").collect();
    assert_eq!(curate.len(), 1);
}

/// P4 crash-resume, end-to-end: per-pane journal → boot recovery (skips cleanly
/// exited panes, keeps the latest captured session id) → Tier-2 launch rewrite.
#[test]
fn resume_journal_recovers_and_plans_tier2() {
    fn rec(kind: &str, cwd: &str, sid: Option<&str>) -> ResumeRecord {
        ResumeRecord {
            ts: 1,
            kind: kind.into(),
            agent: Some("claude".into()),
            cwd: cwd.into(),
            project: Some("p".into()),
            claude_session_id: sid.map(String::from),
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let rdir = dir.path();
    // Pane A: claude working; a later signal captured its session id.
    let a = SessionKey::derive("/work/proj", "claude", None);
    record_event(rdir, &a, &rec("started", "/work/proj", None)).unwrap();
    record_event(rdir, &a, &rec("working", "/work/proj", Some("sess-xyz"))).unwrap();
    // Pane B: cleanly exited → no recovery card.
    let b = SessionKey::derive("/work/other", "claude", None);
    record_event(rdir, &b, &rec("started", "/work/other", None)).unwrap();
    record_event(rdir, &b, &rec("exited", "/work/other", None)).unwrap();

    let recovered = recover_all(rdir);
    assert_eq!(recovered.len(), 1, "only the still-open pane gets a card");
    let pane = &recovered[0];
    assert_eq!(pane.last_kind, "working");
    assert_eq!(pane.claude_session_id.as_deref(), Some("sess-xyz"), "kept the captured id");

    match resume_command(pane, "claude") {
        ResumePlan::Tier2 { command } => assert!(command.contains("--resume sess-xyz"), "{command}"),
        other => panic!("expected Tier2, got {other:?}"),
    }
    // A fresh journal is not GC'd.
    assert_eq!(gc_resume_dir(rdir, 0, 7), 0);
}

/// A 2xx with implausible 0/0 reported usage must NOT charge $0 (Anthropic still
/// bills input) — it floors to the conservative estimate (M3 hardening).
#[test]
fn reflect_zero_usage_charges_estimate() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (_db, idx) = index_with_note(work.path(), store.path());
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let fake = FakeClient::ok(r#"{"proposals":[]}"#, 0, 0); // garbled usage
    let out = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    assert!(matches!(out.reason, ReflectReason::Ok), "{:?}", out.reason);
    assert!(out.spent_usd > 0.0, "0/0 usage must charge the estimate, not $0");
    let (_, spent) = idx.budget_state();
    assert!((spent - out.spent_usd).abs() < 1e-9 && spent > 0.0, "spent folds the floored estimate: {spent}");
}

/// SECRET GATE (M1+M2): a secret planted in a note anchor (redacted at scan) AND in
/// superseded_by (which the doctor interpolates into a finding detail) must NOT
/// reach the cloud — the assembled message is redacted before the client sees it.
#[test]
fn reflect_redacts_secrets_before_cloud() {
    const SECRET: &str = "sk-proj-ABCD1234EFGH5678IJKL9012MNOP3456QRST";
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    // anchor secret (→ digest line) + superseded_by secret (→ doctor finding detail).
    write(
        root,
        ".koden-memory/leaky.md",
        format!(
            "---\nid: leaky\ntype: decision\ntitle: clean title\nsuperseded_by: {SECRET}\nanchors:\n  - src/{SECRET}.rs\n---\nbody\n"
        )
        .as_bytes(),
    );
    let db = store.path().join("i.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);
    scan_project_memory(&idx, PID, root);
    idx.set_budget_ceiling(1.0, 1).unwrap();

    let fake = FakeClient::ok(r#"{"proposals":[]}"#, 100, 100);
    let _ = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    let sent = fake.last_user();
    assert_eq!(fake.calls(), 1);
    assert!(!sent.contains(SECRET), "secret reached the cloud-bound message: {sent}");
    assert!(sent.contains("REDACTED"), "expected a redaction marker in: {sent}");
    // and the stored note (UI/table surface) also has the anchor redacted at scan.
    let notes = list_notes_readonly(&db, Some(PID)).unwrap();
    assert!(notes[0].anchors.iter().all(|a| !a.contains(SECRET)), "anchor secret in table: {:?}", notes[0].anchors);
}

/// No notes → EmptyCorpus, no call, no spend (don't burn a token on nothing).
#[test]
fn reflect_empty_corpus_is_noop() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/app.ts", b"export function f() {}");
    let idx = SqliteIndex::open(&store.path().join("i.sqlite")).unwrap();
    index_dir(&idx, PID, root); // code, but no .koden-memory notes
    idx.set_budget_ceiling(1.0, 1).unwrap();
    let fake = FakeClient::ok(TWO_PROPOSALS, 100, 100);
    let out = reflect_with_client(&idx, &fake, &ReflectConfig::default(), PID, None, 1000);
    assert!(matches!(out.reason, ReflectReason::EmptyCorpus), "{:?}", out.reason);
    assert_eq!(fake.calls(), 0, "empty corpus → zero requests");
    assert_eq!(idx.budget_state().1, 0.0);
}
