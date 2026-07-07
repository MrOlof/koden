//! `SqliteIndex` — the rusqlite (bundled + FTS5) implementation of the retrieval
//! store. The worker holds the single WRITER connection; command threads open
//! their own READ-ONLY connections (WAL → wait-free reads). CONCEPT §8.
//!
//! BM25 is FTS5's built-in `bm25()`: k1=1.2/b=0.75 match Conductr's K1/B
//! (`lexical.ts:11-12`). FTS5 uses the classic BM25 IDF, whereas Conductr uses the
//! 1+-smoothed BM25+ IDF (`lexical.ts:205`); the difference only reorders very
//! common terms and is ranking-equivalent for code corpora — a [DP-2] choice,
//! revisit if the relevance benchmark shows a gap. Per-column weights give the
//! first-class field weighting; the two legs (path+symbols vs content) fuse via
//! weighted RRF (`rank.rs`).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::SearchIndex;
use crate::modules::brain::ast::{self, Impact, ImpactDirection, ImpactRow, SymbolInfo};
use crate::modules::brain::memory::doctor::NoteRecord;
use crate::modules::brain::memory::proposal::{reject_signature, MemoryProposal, ProposalAction};
use crate::modules::brain::memory::{MemoryNote, NoteSummary};
use crate::modules::brain::rank::{self, Leg};
use crate::modules::brain::tokenize;
use crate::modules::brain::Hit;

/// Per-column bm25 weights for the "identity" leg (path ~3×, symbols ~1.5×).
const W_IDENTITY: (f64, f64, f64) = (3.0, 1.5, 0.0);
/// Per-column bm25 weights for the "content" leg.
const W_CONTENT: (f64, f64, f64) = (0.0, 0.0, 1.0);
/// RRF leg weights ([DP-9]). Identity (path+symbols) weighted above content so a
/// filename match outranks a body-only mention (CONCEPT [DP-2]); content still
/// contributes for recall. MEASURED (not guessed): the `brain_bench` calibration
/// sweep over the labeled corpus + confusers shows STRICT `rrf_identity > rrf_content`
/// is MRR-optimal (1.000 vs 0.875 once content ties-or-dominates), at zero
/// negative-control leaks across the whole grid; 1.5 sits in that optimal band. (The
/// boundary rrf_identity == rrf_content == 1.0 is in the LOSING band — ties break to
/// the distractor by ascending id — so do NOT lower the default to 1.0.) Re-run the sweep
/// (`cargo test --test brain_bench -- --ignored`) and record before/after on any
/// change (§13.12). NB: RRF fuses by RANK, so the bm25 column MAGNITUDES above
/// (path 3×) only order WITHIN a leg — the leg RRF weights here are the load-bearing
/// cross-leg knob (the sweep confirms path_bm25 ∈ {2,3,4} doesn't move MRR).
const RRF_W_IDENTITY: f64 = 1.5;
const RRF_W_CONTENT: f64 = 1.0;

/// V3 multi-token COVERAGE re-rank (NorrGit adoption): per-hit coverage = fraction
/// of DISTINCT query tokens (whole + camel parts + stems, exactly as `query_tokens`
/// emits + dedupes them — ADR-010 cluster 7) that the doc matches ANYWHERE
/// (path/symbols/content). Two effects, both DETERMINISTIC (probes run on the same
/// connection snapshot as the legs):
///   1. BLEND: `score *= 1 + COVERAGE_W · coverage` — a doc matching more of the
///      query outranks a doc matching a stray token. Multiplicative (like the
///      temporal boost) so RRF stays leg-pure; measured effect on the scoreboard
///      is ORDER-neutral on the labeled corpora (set metrics saturate), the gate
///      below is what moves P@10 — see brain_precision 2026-07-07 table.
///   2. RELATIVE GATE: for multi-token queries, hits with
///      `matched < COVERAGE_GATE_RATIO · best_matched` are DROPPED, with a
///      CONCEPT-BAG RESCUE: when even the BEST hit is partial (best < all tokens —
///      e.g. the gist's synthesized "project-name + note titles" intent, or
///      hard-concept "authentication flow"), any hit matching ≥
///      COVERAGE_RESCUE_MIN distinct tokens survives — multi-concept intents
///      legitimately have per-concept partial matches (the synth-gist sandbox test
///      is the real-consumer proof: without the rescue, the project's only code
///      file was gated out of its own gist by the note that SEEDED the intent).
///      When some hit FULLY covers the query (a focused query), the relative gate
///      prunes hard. The argmax hit is kept by construction, so a query with any
///      candidate still returns ≥1 hit.
/// MEASURED (brain_precision 2026-07-07): ratio sweep 0.6→macro P@10 0.92,
/// 0.7→0.96, 0.8→1.00; at 0.7 camel-token P@10 0.29→1.00 and exact-name 0.38→1.00;
/// recall (0.96) and negative pollution (0.05) IDENTICAL across the sweep. 0.8's
/// extra +0.04 comes solely from pruning the 0.75-coverage sibling tier (send-sms
/// → email/push), which the corpus cannot price for recall (its one multi-relevant
/// query has all relevants at full coverage) — and a scoreboard pinned at 1.00
/// stops discriminating. 0.7 is the chosen Pareto point (recall-safe side).
/// The concept-bag rescue changes NOTHING on this scoreboard (re-measured
/// identical table at 0.7) — its real-consumer proof is the synth-gist sandbox
/// test above. Single-token queries skip coverage entirely (uniform coverage=1 →
/// gate/blend are no-ops by construction). BOUNDEDNESS vs [DP-2]: the blend-factor RATIO
/// between any two docs is ≤ (1+COVERAGE_W) = 1.25 < the 1.5 cross-leg RRF margin,
/// and between EQUAL-coverage docs the factors cancel exactly — so the temporal
/// "recency can never bury a path match" guarantee is preserved among
/// equal-coverage docs. A body-only hit covering MORE distinct query tokens than a
/// stray-token path hit may now outrank it — deliberate: fuller query coverage is
/// a stronger relevance signal than one stray path token.
const COVERAGE_W: f64 = 0.25;
const COVERAGE_GATE_RATIO: f64 = 0.7;
/// Concept-bag rescue floor (see gate docs above): on partial-best queries a hit
/// matching at least this many DISTINCT tokens is never gated. 2 = "more than a
/// stray token".
const COVERAGE_RESCUE_MIN: usize = 2;
/// ponytail: probe at most this many distinct tokens; keeps the probe cost
/// O(tokens × candidates) with a hard small constant. On a >cap-token query
/// (real consumer: cold-start gist synth intents, which "can run long") every
/// probed count is an UNDERCOUNT — tail-token matches are invisible — so the
/// relative GATE is skipped entirely at the call site: undercounts may still
/// BLEND (monotone, neutral at 0) but must never EXCLUDE. (A partial fix that
/// only exempted `matched == 0` was non-monotonic: a file matching 1 probed +
/// 20 tail tokens was hard-dropped while its tail-only twin survived — matching
/// strictly MORE of the query removed it.) The m == 0 gate exemption in
/// `apply_coverage` remains as defense in depth: within the cap every candidate
/// matches ≥1 probed token, so 0 can only mean UNKNOWN. All precision-corpus
/// queries sit far below the cap, so every measured gate win (camel/exact-name
/// P@10) is unaffected. Upgrade path = IDF-weighted token selection if
/// tail-token matches ever need real RANKING, not just retention.
const COVERAGE_MAX_PROBE_TOKENS: usize = 24;

/// V2 temporal re-rank ([DP-12]) weights: a bounded multiplicative boost so a
/// recently-/frequently-touched file is NUDGED up WITHIN its lexical tier, never
/// buried. Quantized into coarse buckets so sub-threshold drift can't reorder two
/// near-equal docs. Boost = (1 + RECENCY_W·recency)·(1 + FREQ_W·freq), each factor in
/// [1, 1+W]. BOUNDEDNESS INVARIANT (enforced by `temporal_boost_cannot_flip_cross_leg`):
/// the max boost (1+RECENCY_W)(1+FREQ_W) MUST stay below the cross-leg RRF margin
/// (RRF_W_IDENTITY/RRF_W_CONTENT = 1.5) so a fresh body-only hit can NEVER outrank a
/// stale path/identity match — preserving [DP-2]. DETERMINISTIC: computed from STORED
/// `accessed_at_ms`/`accessed_count` and a snapshot-stable `ref_ms` (never `now()`).
const RECENCY_W: f64 = 0.25;
const FREQ_W: f64 = 0.1;
const DAY_MS: i64 = 86_400_000;
/// Recency factor for an UNSTAMPED file (accessed_at_ms == 0): NEUTRAL (mid), not
/// "maximally stale" — a never-touched-but-relevant file mustn't be driven to the
/// bottom just because some OTHER file was recently touched.
const NEUTRAL_RECENCY: f64 = 0.5;

/// Recency bucket (0=stale .. 4=fresh) for an age (ms) measured against `ref_ms`.
/// Coarse step-cliffs are DELIBERATE quantization — they keep the boost a function of
/// a few discrete buckets so sub-threshold age drift can't reorder near-equal docs
/// (do NOT replace with a continuous function of age — that reintroduces read drift).
fn recency_bucket(age_ms: i64) -> i64 {
    if age_ms < DAY_MS {
        4
    } else if age_ms < 7 * DAY_MS {
        3
    } else if age_ms < 30 * DAY_MS {
        2
    } else if age_ms < 90 * DAY_MS {
        1
    } else {
        0
    }
}

/// Frequency bucket = floor(log2(1+count)), capped at 4 → normalized 0..1.
fn freq_norm(count: i64) -> f64 {
    let n = (count.max(0) as u64).saturating_add(1); // 1 + count
    let bucket = (63 - n.leading_zeros() as i64).clamp(0, 4); // floor(log2(n)), capped
    bucket as f64 / 4.0
}

/// The multiplicative temporal boost for one doc. Pure + quantized → unit-testable.
/// `ref_ms` is the doc's OWN-project max accessed_at_ms (so age is relative + stable).
/// An unstamped doc (accessed_at_ms == 0) gets the NEUTRAL recency factor, so a fully-
/// unstamped scope yields a UNIFORM boost → no reordering.
fn temporal_boost(accessed_at_ms: i64, accessed_count: i64, ref_ms: i64) -> f64 {
    let recency = if accessed_at_ms == 0 {
        NEUTRAL_RECENCY
    } else {
        recency_bucket((ref_ms - accessed_at_ms).max(0)) as f64 / 4.0
    };
    (1.0 + RECENCY_W * recency) * (1.0 + FREQ_W * freq_norm(accessed_count))
}

/// The labels of the RRF legs `search_with_conn` actually fuses, in order. The
/// single source of truth for the P5 no-vector-leg gate (`search::registered_search_legs`
/// returns this) — keep it in lockstep with the legs built in `search_with_conn`.
/// A future semantic `vector` leg is added here AND there together at enablement.
pub const SEARCH_LEG_LABELS: &[&str] = &["identity", "content"];

pub struct SqliteIndex {
    conn: Connection,
}

/// Boot busy-retry budget: transient SQLITE_BUSY/LOCKED at startup (another Koden
/// instance mid-checkpoint, AV scan) is waited out briefly instead of declaring the
/// brain Degraded for the whole session.
/// ponytail: fixed budget blocks the worker thread up to ~2.5s at boot; upgrade
/// path = event-driven re-open (or exponential backoff) if real boots contend longer.
const BOOT_BUSY_RETRIES: u32 = 10;
const BOOT_BUSY_DELAY_MS: u64 = 250;

/// How an open/migrate failure is handled by the boot recovery ladder.
enum OpenFailure {
    /// Transient lock contention — retry briefly.
    Busy,
    /// The cache file itself is unusable — rename aside + rebuild fresh.
    Corrupt,
    /// Anything else (I/O, permissions, …) — propagate; the worker degrades.
    Other,
}

fn classify_open_failure(e: &rusqlite::Error) -> OpenFailure {
    match e {
        rusqlite::Error::SqliteFailure(f, _) => match f.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                OpenFailure::Busy
            }
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                OpenFailure::Corrupt
            }
            _ => OpenFailure::Other,
        },
        // migrate()'s version-stamp read: a stamp that exists but can't parse means
        // the meta content is garbage — a corrupt cache, rebuild it.
        rusqlite::Error::FromSqlConversionFailure(..) => OpenFailure::Corrupt,
        _ => OpenFailure::Other,
    }
}

/// Move a corrupt cache db (and its WAL/SHM siblings, so a later salvage-ATTACH of
/// the moved file sees the same last-committed state) aside under a unique
/// `<name>.corrupt-<pid>-<n>` suffix. No wall clock needed — pid+counter is unique
/// within the one directory these files live in.
fn rename_corrupt_aside(path: &Path) -> std::io::Result<std::path::PathBuf> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("index.sqlite")
        .to_string();
    let pid = std::process::id();
    for n in 0..100u32 {
        let aside = path.with_file_name(format!("{name}.corrupt-{pid}-{n}"));
        if aside.exists() {
            continue;
        }
        std::fs::rename(path, &aside)?;
        for suffix in ["-wal", "-shm"] {
            let src = path.with_file_name(format!("{name}{suffix}"));
            if src.exists() {
                let dst = path.with_file_name(format!("{name}.corrupt-{pid}-{n}{suffix}"));
                let _ = std::fs::rename(&src, &dst); // best-effort — main file already aside
            }
        }
        return Ok(aside);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free corrupt-aside slot",
    ))
}

impl SqliteIndex {
    /// Open (or create) the writer connection and run migrations.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        super::migrate::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Open with the boot recovery ladder (ADR-006: the index is a REBUILDABLE
    /// cache — a bad cache file must never brick the brain for every session):
    /// transient BUSY/LOCKED → bounded retry with a short sleep; CORRUPT/NOTADB →
    /// rename the cache aside, reopen fresh exactly ONCE, best-effort salvage of
    /// the CANONICAL tables from the moved file. Any other failure propagates
    /// unchanged (worker → Degraded, as before).
    pub fn open_with_recovery(path: &Path) -> rusqlite::Result<Self> {
        Self::open_with_recovery_at(
            path,
            BOOT_BUSY_RETRIES,
            std::time::Duration::from_millis(BOOT_BUSY_DELAY_MS),
        )
    }

    /// Recovery body with an injectable busy-retry budget (tests use a tiny one).
    fn open_with_recovery_at(
        path: &Path,
        busy_retries: u32,
        busy_delay: std::time::Duration,
    ) -> rusqlite::Result<Self> {
        let mut busy_left = busy_retries;
        loop {
            let err = match Self::open(path) {
                Ok(i) => return Ok(i),
                Err(e) => e,
            };
            match classify_open_failure(&err) {
                OpenFailure::Busy if busy_left > 0 => {
                    busy_left -= 1;
                    log::debug!("brain: store busy at boot ({err}); retrying ({busy_left} left)");
                    std::thread::sleep(busy_delay);
                }
                OpenFailure::Corrupt => {
                    // Keep the corrupt file (salvage source + forensics), never delete.
                    let aside = match rename_corrupt_aside(path) {
                        Ok(p) => p,
                        Err(io) => {
                            log::warn!(
                                "brain: corrupt store {} could not be moved aside ({io})",
                                path.display()
                            );
                            return Err(err);
                        }
                    };
                    log::warn!(
                        "brain: corrupt store detected ({err}); moved aside to {} — rebuilding fresh",
                        aside.display()
                    );
                    // Exactly one fresh retry: a failure HERE is disk-level, not
                    // cache-level, and propagates (no rename loop).
                    let fresh = Self::open(path)?;
                    fresh.salvage_canonical(&aside);
                    return Ok(fresh);
                }
                _ => return Err(err),
            }
        }
    }

    /// Best-effort copy of the CANONICAL tables (human decisions + spend state —
    /// the only rows a cache rebuild cannot re-derive from disk, see
    /// `migrate::CANONICAL_TABLES`) out of a corrupt store that was moved aside.
    /// Per-table: a page-level read error or schema drift loses THAT table's rows
    /// (logged loudly), never the whole salvage. Losing them entirely is acceptable
    /// (ADR-006) — but never silent.
    fn salvage_canonical(&self, corrupt: &Path) {
        if let Err(e) = self
            .conn
            .execute("ATTACH DATABASE ?1 AS salvage", [corrupt.to_string_lossy().as_ref()])
        {
            log::warn!(
                "brain: SALVAGE FAILED — cannot attach corrupt store {} ({e}); \
                 proposals / reject history / budget spend are LOST (rebuilt store starts clean)",
                corrupt.display()
            );
            return;
        }
        match self.conn.unchecked_transaction() {
            Ok(tx) => {
                let mut salvaged = 0usize;
                for t in super::migrate::CANONICAL_TABLES {
                    // OR REPLACE: the fresh DDL seeds singleton rows (brain_budget
                    // id=1, …) that the salvaged row must win over.
                    let sql = format!("INSERT OR REPLACE INTO main.{t} SELECT * FROM salvage.{t}");
                    match tx.execute(&sql, []) {
                        Ok(n) => salvaged += n,
                        Err(e) => log::warn!(
                            "brain: salvage of canonical table '{t}' failed ({e}); its rows are LOST"
                        ),
                    }
                }
                match tx.commit() {
                    Ok(()) => log::info!(
                        "brain: salvaged {salvaged} canonical row(s) from the corrupt store"
                    ),
                    Err(e) => log::warn!(
                        "brain: SALVAGE COMMIT FAILED ({e}); canonical rows are LOST"
                    ),
                }
            }
            Err(e) => {
                log::warn!("brain: SALVAGE FAILED — no transaction ({e}); canonical rows are LOST")
            }
        }
        let _ = self.conn.execute_batch("DETACH DATABASE salvage");
    }

    /// The single writer connection, for same-crate writers that need raw access
    /// (e.g. the P4 budget ledger runs its check/reserve/reconcile txns over it).
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Set the reflect spend ceiling (USD; 0.0 disables). Writer-side (P4).
    pub fn set_budget_ceiling(&self, ceiling_usd: f64, now: i64) -> Result<(), String> {
        crate::modules::brain::reflect::budget::set_ceiling(&self.conn, ceiling_usd, now)
    }

    /// Current reflect budget as `(ceiling_usd, spent_total_usd)` (P4).
    pub fn budget_state(&self) -> (f64, f64) {
        use crate::modules::brain::reflect::budget;
        (budget::ceiling(&self.conn), budget::spent_total(&self.conn))
    }

    /// Persist the Librarian LLM selection (provider/model/base URL + $/Mtok rates).
    /// Writer-side; defaults live in the `brain_librarian` singleton.
    pub fn set_librarian_config(
        &self,
        cfg: &crate::modules::brain::reflect::librarian::LibrarianConfig,
        now: i64,
    ) -> Result<(), String> {
        crate::modules::brain::reflect::librarian::set(&self.conn, cfg, now)
    }

    /// Boot sweep: charge any reservation orphaned by a mid-call crash at its
    /// estimate, so a crashed reflect over-counts rather than leaking free spend (P4).
    pub fn sweep_orphaned_reservations(&self, now: i64) -> Result<usize, String> {
        crate::modules::brain::reflect::budget::sweep_orphaned_reservations(&self.conn, now)
    }

    /// Record a meaningful touch of a file for the V2 temporal re-rank ([DP-12]):
    /// stamp `accessed_at_ms = now_ms` and bump `accessed_count`. Called by the worker
    /// only when a file is actually (re)indexed (a real content change), so the stored
    /// recency advances only when the fingerprint already changes — an unchanged
    /// relaunch leaves it fixed, preserving the gist byte-identity gate. No-op if the
    /// file row is absent. Writer-side.
    pub fn record_access(&self, project_id: &str, rel_path: &str, now_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE files SET accessed_at_ms=?3, accessed_count=accessed_count+1
             WHERE project_id=?1 AND path=?2",
            rusqlite::params![project_id, rel_path, now_ms],
        )?;
        Ok(())
    }

    /// Search with EXPLICIT ranking weights — the offline CALIBRATION seam, used
    /// only by the relevance benchmark to sweep weights over the labeled corpus.
    /// NOT for the production search path: it runs over the WRITER connection, so it
    /// bypasses the read-only WAL snapshot that `search_readonly`/`open_readonly_snapshot`
    /// give the gist byte-identity gate. Production callers must use those instead.
    pub fn search_weighted(
        &self,
        project: Option<&str>,
        query: &str,
        limit: usize,
        w: &SearchWeights,
    ) -> rusqlite::Result<Vec<Hit>> {
        search_with_weights(&self.conn, project, query, limit, w)
    }

    /// Index (insert or update) one file's pre-tokenized streams. No-ops when the
    /// content hash is unchanged. Atomic per file.
    pub fn index_file(
        &self,
        project_id: &str,
        rel_path: &str,
        content: &str,
        hash: &str,
        size: i64,
    ) -> rusqlite::Result<bool> {
        // Unchanged-hash early return BEFORE the (comparatively expensive)
        // tokenize/parse — keeps the serial/incremental no-op path cheap.
        let unchanged = self
            .conn
            .query_row(
                "SELECT hash FROM files WHERE project_id=?1 AND path=?2",
                (project_id, rel_path),
                |r| r.get::<_, String>(0),
            )
            .ok()
            .is_some_and(|old| old == hash);
        if unchanged {
            return Ok(false);
        }
        self.index_file_prepared(
            project_id,
            rel_path,
            &prepare_file(rel_path, content, hash.to_string(), size),
        )
    }

    /// Apply a [prepare_file] result — the WRITE half of [Self::index_file].
    /// Still no-ops (Ok(false)) on an unchanged hash, so a stale precomputed
    /// payload can never double-index. Atomic per file. This is the ONLY entry
    /// the parallel first-index consumer uses: compute happens on worker
    /// threads, every write stays on the single writer connection.
    pub fn index_file_prepared(
        &self,
        project_id: &str,
        rel_path: &str,
        prep: &PreparedFile,
    ) -> rusqlite::Result<bool> {
        let existing: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT hash, fts_rowid FROM files WHERE project_id=?1 AND path=?2",
                (project_id, rel_path),
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .ok();

        if let Some((old_hash, _)) = &existing {
            if old_hash == &prep.hash {
                return Ok(false); // unchanged — skip reindex
            }
        }

        let tx = self.conn.unchecked_transaction()?;
        if let Some((_, old_rowid)) = existing {
            tx.execute("DELETE FROM code_fts WHERE rowid=?1", [old_rowid])?;
        }
        tx.execute(
            "INSERT INTO code_fts(path,symbols,content) VALUES(?1,?2,?3)",
            (&prep.path_tokens, &prep.symbol_tokens, &prep.content_tokens),
        )?;
        let rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO files(project_id,path,hash,size,fts_rowid) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(project_id,path) DO UPDATE SET
                hash=excluded.hash, size=excluded.size, fts_rowid=excluded.fts_rowid",
            (project_id, rel_path, prep.hash.as_str(), prep.size, rowid),
        )?;
        // Replace this file's nodes + import specs.
        tx.execute(
            "DELETE FROM code_nodes WHERE project_id=?1 AND path=?2",
            (project_id, rel_path),
        )?;
        tx.execute(
            "DELETE FROM code_imports WHERE project_id=?1 AND src_path=?2",
            (project_id, rel_path),
        )?;
        for n in &prep.nodes {
            tx.execute(
                "INSERT OR IGNORE INTO code_nodes(project_id,path,name,kind,start_line,start_col) VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![project_id, rel_path, n.name, n.kind, n.start_line, n.start_col],
            )?;
        }
        for (spec, base) in &prep.imports {
            tx.execute(
                "INSERT OR IGNORE INTO code_imports(project_id,src_path,spec,base) VALUES(?1,?2,?3,?4)",
                (project_id, rel_path, spec.as_str(), base.as_str()),
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Insert or update a structured memory note (P1). The note's text is made
    /// searchable separately by the code walk; this is the typed/queryable row.
    pub fn upsert_note(
        &self,
        project_id: &str,
        note: &MemoryNote,
        rel_path: &str,
        hash: &str,
    ) -> rusqlite::Result<()> {
        let anchors_json = serde_json::to_string(&note.anchors).unwrap_or_else(|_| "[]".to_string());
        self.conn.execute(
            "INSERT INTO notes(project_id,id,path,note_type,status,title,scope,provenance,created,revalidate_after,supersedes,superseded_by,anchors,hash)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
             ON CONFLICT(project_id,id) DO UPDATE SET
                path=excluded.path, note_type=excluded.note_type, status=excluded.status,
                title=excluded.title, scope=excluded.scope, provenance=excluded.provenance,
                created=excluded.created, revalidate_after=excluded.revalidate_after,
                supersedes=excluded.supersedes, superseded_by=excluded.superseded_by,
                anchors=excluded.anchors, hash=excluded.hash",
            rusqlite::params![
                project_id, note.id, rel_path, note.note_type, note.status, note.title,
                note.scope, note.provenance, note.created, note.revalidate_after,
                note.supersedes, note.superseded_by, anchors_json, hash
            ],
        )?;
        Ok(())
    }

    pub fn note_count(&self, project_id: &str) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM notes WHERE project_id=?1",
            [project_id],
            |r| r.get(0),
        )
    }

    pub fn existing_note_ids(&self, project_id: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM notes WHERE project_id=?1")?;
        let it = stmt.query_map([project_id], |r| r.get::<_, String>(0))?;
        let mut v = Vec::new();
        for x in it {
            v.push(x?);
        }
        Ok(v)
    }

    /// Remove a note that vanished on disk, plus its dependent PENDING proposals
    /// (so the doctor doesn't keep regenerating findings for a gone note). One tx.
    pub fn remove_note(&self, project_id: &str, id: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let n = tx.execute(
            "DELETE FROM notes WHERE project_id=?1 AND id=?2",
            (project_id, id),
        )?;
        tx.execute(
            "DELETE FROM proposals WHERE project_id=?1 AND target_id=?2 AND status='pending'",
            (project_id, id),
        )?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// Full note records for the doctor. Deterministic order (ORDER BY id): the
    /// records feed the doctor findings and thus the reflect digest, whose hash is
    /// the autonomous delta gate — scan-order instability would break the
    /// "unchanged corpus ⇒ unchanged digest ⇒ $0" guarantee (ADR-010 cluster 5).
    pub fn list_note_records(&self, project_id: &str) -> rusqlite::Result<Vec<NoteRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, note_type, revalidate_after, supersedes, superseded_by, COALESCE(anchors,'[]')
             FROM notes WHERE project_id=?1 ORDER BY id",
        )?;
        let it = stmt.query_map([project_id], |r| {
            let anchors: Vec<String> =
                serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default();
            Ok(NoteRecord {
                id: r.get(0)?,
                note_type: r.get(1)?,
                revalidate_after: r.get(2)?,
                supersedes: r.get(3)?,
                superseded_by: r.get(4)?,
                anchors,
            })
        })?;
        let mut v = Vec::new();
        for x in it {
            v.push(x?);
        }
        Ok(v)
    }

    /// The set of indexed file paths (for anchor validation).
    pub fn indexed_path_set(
        &self,
        project_id: &str,
    ) -> rusqlite::Result<std::collections::HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM files WHERE project_id=?1")?;
        let it = stmt.query_map([project_id], |r| r.get::<_, String>(0))?;
        let mut set = std::collections::HashSet::new();
        for x in it {
            set.insert(x?);
        }
        Ok(set)
    }

    pub fn is_rejected(&self, project_id: &str, reject_sig: &str) -> rusqlite::Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM reject_signatures WHERE project_id=?1 AND reject_sig=?2",
            (project_id, reject_sig),
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// True if a proposal row with this signature exists in ANY status (pending,
    /// applied, or rejected) — `insert_proposal` would no-op on it. Used by the
    /// paid escalate bands to skip candidates BEFORE reserving/spending.
    pub fn proposal_exists(&self, project_id: &str, signature: &str) -> rusqlite::Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM proposals WHERE project_id=?1 AND signature=?2",
            (project_id, signature),
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// True if a curate-sourced proposal for this note is already awaiting review —
    /// the paid escalate band's pending-dedup gate (re-judging a note that is
    /// already in the inbox can add nothing the human hasn't seen).
    pub fn has_pending_curate_proposal(
        &self,
        project_id: &str,
        target_id: &str,
    ) -> rusqlite::Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM proposals
             WHERE project_id=?1 AND target_id=?2 AND source='curate' AND status='pending'",
            (project_id, target_id),
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Queue a proposal (dedup by signature). Returns true if it was newly added.
    pub fn insert_proposal(
        &self,
        project_id: &str,
        p: &MemoryProposal,
        created_ms: i64,
    ) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "INSERT INTO proposals(project_id,signature,action,target_id,title,detail,source,status,created_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(project_id,signature) DO NOTHING",
            rusqlite::params![
                project_id, p.signature, p.action.as_str(), p.target_id, p.title,
                p.detail, p.source, p.status, created_ms
            ],
        )?;
        Ok(n > 0)
    }

    /// Resolve a proposal: `reject` persists its reject-signature (so it can't
    /// reappear) and marks it rejected; otherwise marks it applied. Returns
    /// whether the proposal existed.
    pub fn resolve_proposal(
        &self,
        project_id: &str,
        signature: &str,
        reject: bool,
    ) -> rusqlite::Result<bool> {
        let row: Option<(String, Option<String>, String)> = self
            .conn
            .query_row(
                "SELECT action, target_id, title FROM proposals WHERE project_id=?1 AND signature=?2",
                (project_id, signature),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let Some((action_s, target_id, title)) = row else {
            return Ok(false);
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE proposals SET status=?3 WHERE project_id=?1 AND signature=?2",
            rusqlite::params![project_id, signature, if reject { "rejected" } else { "applied" }],
        )?;
        if reject {
            if let Some(action) = ProposalAction::from_token(&action_s) {
                let sig = reject_signature(action, target_id.as_deref(), &title);
                tx.execute(
                    "INSERT OR IGNORE INTO reject_signatures(project_id,reject_sig) VALUES(?1,?2)",
                    (project_id, sig),
                )?;
            }
        }
        tx.commit()?;
        Ok(true)
    }

    /// Flush the WAL (called on the idle tick).
    pub fn checkpoint(&self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
    }

    pub fn file_count(&self, project_id: &str) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE project_id=?1",
            [project_id],
            |r| r.get(0),
        )
    }

    /// The workspace aggregate fingerprint for a project (blake3 over sorted
    /// `(path, hash)`), the basis of P3's cache-stable gist key. CONCEPT [DP-13].
    pub fn project_fingerprint(&self, project_id: &str) -> rusqlite::Result<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash FROM files WHERE project_id=?1")?;
        let it = stmt.query_map([project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut entries: Vec<(String, String)> = Vec::new();
        for x in it {
            entries.push(x?);
        }
        Ok(crate::modules::brain::freshness::aggregate_fingerprint(&mut entries))
    }

    /// Sorted resolved import edges `(src, dst)` — for diagnostics + the
    /// incremental==full property test.
    pub fn project_edges(&self, project_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT src_path,dst_path FROM code_edges WHERE project_id=?1 ORDER BY src_path,dst_path",
        )?;
        let it = stmt.query_map([project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut v = Vec::new();
        for x in it {
            v.push(x?);
        }
        Ok(v)
    }

    /// Sorted node keys `path|name|kind|line` — for the property test.
    pub fn project_node_keys(&self, project_id: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT path,name,kind,start_line,start_col FROM code_nodes WHERE project_id=?1 ORDER BY path,name,kind,start_line,start_col",
        )?;
        let it = stmt.query_map([project_id], |r| {
            Ok(format!(
                "{}|{}|{}|{}|{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?
            ))
        })?;
        let mut v = Vec::new();
        for x in it {
            v.push(x?);
        }
        Ok(v)
    }

    /// path → content hash snapshot for a project — read once at the start of a
    /// full-index pass so the parallel compute workers can skip tokenize/parse
    /// on unchanged files (the compute-side twin of the writer-side hash-skip
    /// in [Self::index_file_prepared]). Safe as a pass-long snapshot: the ONE
    /// writer is the pass itself and each path is written at most once per pass.
    pub fn existing_hashes(
        &self,
        project_id: &str,
    ) -> rusqlite::Result<std::collections::HashMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash FROM files WHERE project_id=?1")?;
        let it = stmt.query_map([project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut m = std::collections::HashMap::new();
        for x in it {
            let (p, h) = x?;
            m.insert(p, h);
        }
        Ok(m)
    }

    /// All indexed paths for a project (used by reconcile to detect deletions).
    pub fn existing_paths(&self, project_id: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM files WHERE project_id=?1")?;
        let it = stmt.query_map([project_id], |r| r.get::<_, String>(0))?;
        let mut v = Vec::new();
        for x in it {
            v.push(x?);
        }
        Ok(v)
    }

    /// Remove a file's manifest row + its FTS document (deleted/moved on disk).
    /// The reconcile path calls this so removed files stop matching searches and
    /// `file_count` stays accurate. Returns whether a row was removed.
    pub fn remove_file(&self, project_id: &str, rel_path: &str) -> rusqlite::Result<bool> {
        let rowid: Option<i64> = self
            .conn
            .query_row(
                "SELECT fts_rowid FROM files WHERE project_id=?1 AND path=?2",
                (project_id, rel_path),
                |r| r.get(0),
            )
            .ok();
        let Some(rid) = rowid else {
            return Ok(false);
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM code_fts WHERE rowid=?1", [rid])?;
        tx.execute(
            "DELETE FROM files WHERE project_id=?1 AND path=?2",
            (project_id, rel_path),
        )?;
        tx.execute(
            "DELETE FROM code_nodes WHERE project_id=?1 AND path=?2",
            (project_id, rel_path),
        )?;
        tx.execute(
            "DELETE FROM code_imports WHERE project_id=?1 AND src_path=?2",
            (project_id, rel_path),
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Drop ALL indexed state for a project (used when a project is removed from the
    /// registry). Prunes every project-scoped table; the FTS rows are deleted via the
    /// files' `fts_rowid` first. Does NOT touch any user files — brain-local only.
    pub fn remove_project(&self, project_id: &str) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM code_fts WHERE rowid IN (SELECT fts_rowid FROM files WHERE project_id=?1)",
            [project_id],
        )?;
        tx.execute("DELETE FROM files WHERE project_id=?1", [project_id])?;
        tx.execute("DELETE FROM notes WHERE project_id=?1", [project_id])?;
        tx.execute("DELETE FROM proposals WHERE project_id=?1", [project_id])?;
        tx.execute("DELETE FROM reject_signatures WHERE project_id=?1", [project_id])?;
        tx.execute("DELETE FROM code_nodes WHERE project_id=?1", [project_id])?;
        tx.execute("DELETE FROM code_imports WHERE project_id=?1", [project_id])?;
        tx.execute("DELETE FROM code_edges WHERE project_id=?1", [project_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Rebuild the resolved import edges for a project from `code_imports` + the
    /// current file set. A pure function of (imports, files) — no parsing — so an
    /// incrementally-relinked graph and a full rebuild converge to the same edges.
    /// Called once per project pass (full or incremental).
    pub fn rebuild_edges(&self, project_id: &str) -> rusqlite::Result<()> {
        let files = self.indexed_path_set(project_id)?;
        let imports: Vec<(String, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT src_path, spec FROM code_imports WHERE project_id=?1")?;
            let it = stmt.query_map([project_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut v = Vec::new();
            for x in it {
                v.push(x?);
            }
            v
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM code_edges WHERE project_id=?1", [project_id])?;
        for (src, spec) in imports {
            if let Some(dst) = resolve_import(&src, &spec, &files) {
                if dst != src {
                    // skip self-loops (a file resolving an import to itself)
                    tx.execute(
                        "INSERT OR IGNORE INTO code_edges(project_id,src_path,dst_path,kind) VALUES(?1,?2,?3,'imports')",
                        (project_id, &src, &dst),
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Delta edge relink — recompute ONLY the edges a change-set can affect,
    /// converging byte-identically with a from-scratch [Self::rebuild_edges]
    /// (pinned by `relink_delta_converges_with_full_rebuild`). Affected srcs:
    /// - the changed/removed files themselves (their `code_imports` rows were
    ///   just rewritten by `index_file` / deleted by `remove_file`), and
    /// - every importer whose stored `base` is SERVED by a changed/removed file
    ///   (the dst side: a NEW file can become — or shadow — the resolution
    ///   target of an EXISTING import, and a removal can un-shadow the next
    ///   EXTS fallback; `serveable_bases` is the exact inverse of the EXTS
    ///   expansion, so no other import's candidate-existence can have changed).
    /// Cost ∝ delta (index-backed base lookups + per-candidate PK probes) —
    /// never O(project imports). `changed` may safely over-approximate (an
    /// unchanged file relinks to the same edges); iteration is over BTreeSets
    /// for deterministic order.
    pub fn relink_edges_delta(
        &self,
        project_id: &str,
        changed: &[String],
        removed: &[String],
    ) -> rusqlite::Result<()> {
        if changed.is_empty() && removed.is_empty() {
            return Ok(());
        }
        let mut srcs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut bases: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for p in changed.iter().chain(removed) {
            srcs.insert(p.clone());
            for b in serveable_bases(p) {
                bases.insert(b);
            }
        }
        {
            let mut stmt = self
                .conn
                .prepare("SELECT src_path FROM code_imports WHERE project_id=?1 AND base=?2")?;
            for b in &bases {
                let it = stmt.query_map((project_id, b.as_str()), |r| r.get::<_, String>(0))?;
                for s in it {
                    srcs.insert(s?);
                }
            }
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut del =
                tx.prepare("DELETE FROM code_edges WHERE project_id=?1 AND src_path=?2")?;
            // No SQL ORDER BY here: it baits the planner onto code_imports_base
            // with only project_id bound (an O(project-imports) scan per src —
            // the exact leak this fn exists to close); the (project_id, src_path)
            // seek + a Rust sort keeps it delta-bounded AND deterministic.
            let mut imp = tx.prepare(
                "SELECT base FROM code_imports WHERE project_id=?1 AND src_path=?2 AND base<>''",
            )?;
            let mut exists = tx.prepare("SELECT 1 FROM files WHERE project_id=?1 AND path=?2")?;
            let mut ins = tx.prepare(
                "INSERT OR IGNORE INTO code_edges(project_id,src_path,dst_path,kind) VALUES(?1,?2,?3,'imports')",
            )?;
            for src in &srcs {
                del.execute((project_id, src.as_str()))?;
                let mut src_bases: Vec<String> = imp
                    .query_map((project_id, src.as_str()), |r| r.get(0))?
                    .collect::<Result<_, _>>()?;
                src_bases.sort(); // deterministic resolution order (see above)
                // Per-language ext set, keyed off the IMPORTER like resolve_import
                // (a file's imports are always in its own language).
                let exts: &[&str] = if is_rust_path(src) { RUST_EXTS } else { EXTS };
                for base in src_bases {
                    // Same EXTS precedence as resolve_import, membership via PK
                    // probes against the CURRENT file set (bounded, not a set load).
                    for e in exts {
                        let cand = format!("{base}{e}");
                        if exists.exists((project_id, cand.as_str()))? {
                            if cand != *src {
                                // skip self-loops (a file resolving an import to itself)
                                ins.execute((project_id, src.as_str(), cand.as_str()))?;
                            }
                            break;
                        }
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl SearchIndex for SqliteIndex {
    fn search(&self, project: Option<&str>, query: &str, limit: usize) -> rusqlite::Result<Vec<Hit>> {
        search_with_conn(&self.conn, project, query, limit)
    }
}

/// The COMPUTE half of one file's index write: pre-tokenized FTS streams +
/// parsed definitions + import specs (with their normalized resolution bases).
/// A pure function of (rel_path, content) with owned fields only, so the
/// parallel first-index can build it on N compute threads and hand it to the
/// ONE writer for [SqliteIndex::index_file_prepared] (single-writer invariant).
pub struct PreparedFile {
    pub hash: String,
    pub size: i64,
    path_tokens: String,
    symbol_tokens: String,
    content_tokens: String,
    nodes: Vec<ast::CodeNode>,
    /// `(spec, base)` — base precomputed ('' = can never resolve in-project)
    /// so the delta relink can index-match affected imports.
    imports: Vec<(String, String)>,
}

/// Tokenize + parse one file for indexing — no connection touched. P2: parse
/// once → definitions (the `symbols` FTS column + `code_nodes`) and raw import
/// specs (`code_imports`). Edges are NOT touched here; they're rebuilt as a
/// pure function of imports+files (rebuild_edges / relink_edges_delta).
pub fn prepare_file(rel_path: &str, content: &str, hash: String, size: i64) -> PreparedFile {
    let path_tokens = tokenize::tokenize(rel_path).join(" ");
    let content_tokens = tokenize::tokenize(content).join(" ");
    let analysis = analyze_for(rel_path, content);
    let symbol_tokens = analysis
        .as_ref()
        .map(|a| tokenize::tokenize(&a.symbol_names()).join(" "))
        .unwrap_or_default();
    // `None` (non-AST language) flattens to empty vecs — the apply side deletes
    // stale nodes/imports unconditionally either way, byte-identical to before.
    let (nodes, imports) = match analysis {
        Some(a) => {
            let is_rust = is_rust_path(rel_path);
            let mut imports: Vec<(String, String)> = Vec::new();
            for spec in &a.imports {
                if is_rust {
                    let base = rust_use_base(rel_path, spec).unwrap_or_default();
                    imports.push((spec.clone(), base));
                    // A use path's LAST segment may be an ITEM, not a module —
                    // the defining FILE is then the parent module's. The parent
                    // path is emitted as its own import row so the fixed
                    // base+ext machinery (resolve / delta relink / serveable
                    // inverse) handles both shapes uniformly; the PK dedupes.
                    if let Some(parent) = rust_parent_spec(spec) {
                        let pbase = rust_use_base(rel_path, &parent).unwrap_or_default();
                        if !pbase.is_empty() {
                            imports.push((parent, pbase));
                        }
                    }
                } else {
                    let base = import_base(rel_path, spec).unwrap_or_default();
                    imports.push((spec.clone(), base));
                }
            }
            (a.nodes, imports)
        }
        None => (Vec::new(), Vec::new()),
    };
    PreparedFile { hash, size, path_tokens, symbol_tokens, content_tokens, nodes, imports }
}

/// One-parse AST analysis for a file, or `None` for non-AST languages.
fn analyze_for(rel_path: &str, content: &str) -> Option<ast::Analysis> {
    let ext = std::path::Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    ast::lang_for_ext(ext).map(|lang| ast::analyze(lang, content))
}

/// The fixed candidate suffixes a resolution base is expanded with (extension +
/// index fallback), in precedence order. Shared by [resolve_import] (full
/// rebuild), the delta relink's PK probes, and [serveable_bases] (the inverse
/// mapping) — the three MUST stay in lockstep or delta/full edge builds diverge.
/// The ext set is chosen PER LANGUAGE: by importer/src extension on the resolve
/// side, by dst extension on the serveable side (a `.rs` file can only satisfy
/// Rust bases and vice versa — the selectors agree because a file's imports are
/// always in its own language).
const EXTS: &[&str] = &[
    "", ".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.tsx", "/index.js",
];

/// Rust counterpart of [EXTS]: a use-path base `a/b` is served by `a/b.rs` or
/// `a/b/mod.rs`; a crate-root base (`crate` alone / a lib crate named from
/// tests|examples) by `<src>/lib.rs` or `<src>/main.rs`. No bare "" entry —
/// Rust specs never carry a file extension.
const RUST_EXTS: &[&str] = &[".rs", "/mod.rs", "/lib.rs", "/main.rs"];

/// One extension-check used by ALL per-language ext-set selections (prepare,
/// resolve, delta relink, serveable) so they can never disagree.
fn is_rust_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("rs"))
        .unwrap_or(false)
}

/// Normalized resolution BASE of an import spec: importer-dir + spec with
/// `.`/`..` folded (and `\` / ?query / #hash cleaned). `None` for bare/external
/// specifiers (e.g. `react`), root-escaping specs, and an empty result — exactly
/// the specs that can never resolve to an in-project edge. Stored per import row
/// (`code_imports.base`, '' for None) so the delta relink can index-match it.
fn import_base(importer: &str, spec: &str) -> Option<String> {
    // Normalize the specifier: backslashes → '/' (Windows-authored imports) and
    // drop any ?query / #hash suffix before resolving.
    let spec_norm = spec.replace('\\', "/");
    let spec_clean = spec_norm.split(['?', '#']).next().unwrap_or(&spec_norm);
    if !(spec_clean.starts_with("./") || spec_clean.starts_with("../")) {
        return None;
    }
    let dir = std::path::Path::new(importer)
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""));
    // `None` when the spec escapes the project root — not an in-project edge.
    let base = normalize_rel(&dir.join(spec_clean))?;
    if base.is_empty() {
        return None;
    }
    Some(base)
}

/// Resolution BASE of a Rust `use` path — a LEXICAL module-tree mapping (no fs,
/// pure fn of importer path + spec, like [import_base]):
/// - `crate::a::b`      → `<crate-src-dir>/a/b` (nearest ancestor dir named `src`)
/// - `self::a`          → `<own-module-dir>/a` (`x.rs` owns `x/`; mod/lib/main.rs own their dir)
/// - `super::a` (chain) → parent module dirs, popped per `super`
/// - `name::a::b` from a file OUTSIDE any `src/` tree but under tests|examples|
///   benches (separate crates that must name the lib crate) → `<sibling src>/a/b`.
///   In-src files use `crate::`/`self::`/`super::` for local paths, so a leading
///   crate NAME there is external → `None` (avoids `use serde_json::x` false edges).
/// `None` for std/core/alloc/proc_macro, bare crate names, and unmappable roots.
/// Known ceiling: `use a::Enum::Variant` maps only full + parent paths, so the
/// grandparent-defined variant case yields no edge (over-approximation stays in
/// the lexical tier); `mod` declarations and workspace cross-crate deps are not
/// mapped. Rust 2018 UNIFORM paths are unmapped too: a leading module NAME from
/// an in-src file (`use modules::x` in lib.rs instead of `crate::modules::x`)
/// hits the external-crate branch above and yields no edge — distinguishing a
/// local top-level module from an extern crate lexically would risk
/// `use serde_json::x` false edges, so the miss stays fail-open in the lexical
/// tier (the dependent still surfaces via lexical_candidates).
fn rust_use_base(importer: &str, spec: &str) -> Option<String> {
    let norm = importer.replace('\\', "/");
    let mut dir: Vec<&str> = norm.split('/').filter(|c| !c.is_empty()).collect();
    let file = dir.pop()?;
    let segs: Vec<String> = spec
        .split("::")
        .map(|s| s.trim().trim_start_matches("r#").to_string())
        .collect();
    if segs.is_empty()
        || segs
            .iter()
            .any(|s| s.is_empty() || !s.chars().all(|c| c.is_alphanumeric() || c == '_'))
    {
        return None;
    }
    // Own module dir: `worker.rs` owns `worker/`; mod.rs / lib.rs / main.rs own
    // the directory they sit in.
    let self_dir = |dir: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = dir.iter().map(|s| s.to_string()).collect();
        if !matches!(file, "mod.rs" | "lib.rs" | "main.rs") {
            if let Some(stem) = file.strip_suffix(".rs") {
                v.push(stem.to_string());
            }
        }
        v
    };
    let (mut base, rest) = match segs[0].as_str() {
        "std" | "core" | "alloc" | "proc_macro" => return None,
        "crate" => {
            let i = dir.iter().rposition(|c| *c == "src")?;
            (dir[..=i].iter().map(|s| s.to_string()).collect::<Vec<_>>(), &segs[1..])
        }
        "self" => (self_dir(&dir), &segs[1..]),
        "super" => {
            let mut d = self_dir(&dir);
            let mut i = 0;
            while i < segs.len() && segs[i] == "super" {
                if d.pop().is_none() {
                    return None; // escaped above the project root
                }
                i += 1;
            }
            (d, &segs[i..])
        }
        _name => {
            // Leading crate NAME: only meaningful from tests/examples/benches
            // crates (no `src` ancestor), which reference the lib by name.
            if dir.iter().any(|c| *c == "src") {
                return None;
            }
            let i = dir
                .iter()
                .rposition(|c| matches!(*c, "tests" | "examples" | "benches"))?;
            if segs.len() < 2 {
                return None; // bare crate name (`use serde;`) — never a file edge
            }
            let mut v: Vec<String> = dir[..i].iter().map(|s| s.to_string()).collect();
            v.push("src".to_string());
            (v, &segs[1..])
        }
    };
    base.extend(rest.iter().cloned());
    if base.is_empty() {
        return None;
    }
    Some(base.join("/"))
}

/// Parent path of a Rust use spec (`a::b::c` → `a::b`) — emitted as its own
/// import row because the last segment may be an item whose defining file is
/// the parent module's. `None` when the parent would be a BARE non-keyword
/// crate name: resolving that to `<src>/lib.rs` would edge every
/// `use extern_crate::item` to the local lib root (false edges). `crate` /
/// `self` / `super` parents are always local, hence safe.
fn rust_parent_spec(spec: &str) -> Option<String> {
    let (parent, _last) = spec.rsplit_once("::")?;
    if !parent.contains("::") && !matches!(parent, "crate" | "self" | "super") {
        return None;
    }
    Some(parent.to_string())
}

/// All bases an indexed file can SATISFY — the inverse of the EXTS expansion:
/// `x.ts` serves bases `x.ts` (bare) and `x` (`.ts`); `x/index.ts` additionally
/// serves `x` (`/index.ts`). When a file appears or disappears, ONLY imports
/// whose stored base is in this set can change resolution (incl. shadowing:
/// a new `x.ts` outranking an existing `x/index.ts` target, and un-shadowing on
/// delete) — the dst-side affected set of [SqliteIndex::relink_edges_delta].
fn serveable_bases(path: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if is_rust_path(path) {
        // A .rs file can only satisfy Rust bases (RUST_EXTS inverse, no bare "").
        for e in RUST_EXTS {
            if let Some(stripped) = path.strip_suffix(e) {
                if !stripped.is_empty() {
                    v.push(stripped.to_string());
                }
            }
        }
    } else {
        for e in EXTS {
            if e.is_empty() {
                v.push(path.to_string());
            } else if let Some(stripped) = path.strip_suffix(e) {
                if !stripped.is_empty() {
                    v.push(stripped.to_string());
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Resolve a relative import specifier to an indexed file path (extension +
/// index fallback). Bare/external specifiers (e.g. `react`) return `None`.
/// Module resolution via tsconfig paths / package exports / Cargo members is a
/// later P2 refinement; relative resolution covers the common case.
fn resolve_import(
    importer: &str,
    spec: &str,
    files: &std::collections::HashSet<String>,
) -> Option<String> {
    if is_rust_path(importer) {
        let base = rust_use_base(importer, spec)?;
        return RUST_EXTS
            .iter()
            .map(|e| format!("{base}{e}"))
            .find(|c| files.contains(c));
    }
    let base = import_base(importer, spec)?;
    EXTS.iter()
        .map(|e| format!("{base}{e}"))
        .find(|c| files.contains(c))
}

/// Lexically normalize a path (resolve `.`/`..`) to a forward-slash rel string.
/// Returns `None` if `..` escapes above the start (would point outside the
/// project root) — preventing a false edge to a same-named in-root file.
fn normalize_rel(p: &std::path::Path) -> Option<String> {
    use std::path::Component;
    let mut stack: Vec<String> = Vec::new();
    for comp in p.components() {
        match comp {
            Component::Normal(s) => stack.push(s.to_string_lossy().to_string()),
            Component::ParentDir => {
                stack.pop()?; // escaped above the root → not an in-project edge
            }
            _ => {}
        }
    }
    Some(stack.join("/"))
}

/// All definition locations of a symbol (`brain_get_symbol`).
pub fn get_symbol_readonly(
    db_path: &Path,
    project: &str,
    symbol: &str,
) -> rusqlite::Result<Vec<SymbolInfo>> {
    let conn = open_readonly(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT path,name,kind,start_line FROM code_nodes WHERE project_id=?1 AND name=?2 ORDER BY path,start_line",
    )?;
    let it = stmt.query_map((project, symbol), |r| {
        Ok(SymbolInfo {
            path: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            start_line: r.get(3)?,
        })
    })?;
    let mut v = Vec::new();
    for x in it {
        v.push(x?);
    }
    Ok(v)
}

/// Test-convention matcher for `exclude_tests` — a small PURE function:
/// a `tests` path segment, `*.test.*` / `*.spec.*` / `*_test.*` file names, or a
/// `test_*` prefix (pytest-style). Deliberately exactly these conventions —
/// // ponytail: no singular `test/` segment or `tests.rs` filename matching;
/// broader heuristics start eating real files (`latest.rs`, `attestation.ts`).
fn is_test_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let mut segs = norm.split('/').peekable();
    let mut file = "";
    while let Some(seg) = segs.next() {
        if segs.peek().is_none() {
            file = seg; // last segment = file name
        } else if seg == "tests" {
            return true; // a tests/ directory segment
        }
    }
    let lower = file.to_ascii_lowercase();
    lower.starts_with("test_")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.contains("_test.")
}

/// Depth-annotated directed BFS over `code_edges`. `reverse = true` walks
/// dst→src (upstream: who imports me); `false` walks src→dst (downstream: what
/// I import). Depth is minimal by construction (layered expansion + a visited
/// set, which is also the cycle guard — no infinite loop on import cycles).
/// Each frontier is SORTED before expansion so multi-path ties resolve
/// identically every run (deterministic-ordering invariant).
fn bfs_edges(
    conn: &Connection,
    project: &str,
    roots: &[String],
    depth: usize,
    reverse: bool,
) -> rusqlite::Result<Vec<(String, usize)>> {
    let sql = if reverse {
        "SELECT src_path FROM code_edges WHERE project_id=?1 AND dst_path=?2 ORDER BY src_path"
    } else {
        "SELECT dst_path FROM code_edges WHERE project_id=?1 AND src_path=?2 ORDER BY dst_path"
    };
    let mut stmt = conn.prepare(sql)?;
    let mut seen: std::collections::HashSet<String> = roots.iter().cloned().collect();
    let mut frontier: Vec<String> = roots.to_vec();
    let mut out: Vec<(String, usize)> = Vec::new();
    for d in 1..=depth {
        frontier.sort();
        let mut next = Vec::new();
        for node in &frontier {
            let it = stmt.query_map((project, node.as_str()), |r| r.get::<_, String>(0))?;
            for x in it {
                let s = x?;
                if seen.insert(s.clone()) {
                    out.push((s.clone(), d));
                    next.push(s);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    Ok(out)
}

/// Tiered impact of a symbol: the depth-annotated AST import closure around the
/// file(s) defining the symbol, plus the lexical over-approximation (content
/// mentions — never depth-annotated). CONCEPT §4.1b `code_impact`.
/// File-granular: edges are file-level imports, not symbol-level references.
/// Ordering is deterministic end-to-end: sorted `defined_in`, sorted BFS
/// frontiers, BTreeMap merge for `both`, final (depth asc, path asc) sort —
/// and truncation happens only AFTER that full ordering.
pub fn code_impact_readonly(
    db_path: &Path,
    project: &str,
    symbol: &str,
    depth: usize,
    direction: ImpactDirection,
    max_results: usize,
    exclude_tests: bool,
) -> rusqlite::Result<Impact> {
    let conn = open_readonly(db_path)?;
    // ponytail: depth ceiling 20 — file-level import graphs saturate well before.
    let depth = depth.clamp(1, 20);
    let max_results = max_results.clamp(1, 2000);
    let defined_in: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT path FROM code_nodes WHERE project_id=?1 AND name=?2")?;
        let it = stmt.query_map((project, symbol), |r| r.get::<_, String>(0))?;
        let mut v = Vec::new();
        for x in it {
            v.push(x?);
        }
        v.sort(); // deterministic order for multi-definition symbols
        v
    };

    // Directed BFS leg(s). `both` merges via BTreeMap keeping the MIN depth per
    // path — deterministic regardless of leg order.
    let mut reach: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut merge = |leg: Vec<(String, usize)>| {
        for (p, d) in leg {
            reach.entry(p).and_modify(|e| *e = (*e).min(d)).or_insert(d);
        }
    };
    if matches!(direction, ImpactDirection::Upstream | ImpactDirection::Both) {
        merge(bfs_edges(&conn, project, &defined_in, depth, true)?);
    }
    if matches!(direction, ImpactDirection::Downstream | ImpactDirection::Both) {
        merge(bfs_edges(&conn, project, &defined_in, depth, false)?);
    }

    // Filter (output-only: traversal stays intact so a test file mid-path can't
    // hide transitive reach), then order fully, THEN truncate — the kept prefix
    // is stable across runs.
    let mut rows: Vec<ImpactRow> = reach
        .iter()
        .filter(|(p, _)| !exclude_tests || !is_test_path(p))
        .map(|(p, d)| ImpactRow { path: p.clone(), depth: *d })
        .collect();
    rows.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.path.cmp(&b.path)));
    let result_total = rows.len();
    let truncated = result_total > max_results;
    if truncated {
        rows.truncate(max_results);
    }
    let ast_dependents: Vec<String> = rows.iter().map(|r| r.path.clone()).collect();

    // Lexical over-approximation tier: content mentions not already covered by
    // the graph (the FULL pre-truncation reach, so a cut row can't reappear as
    // a lexical candidate). Capped at 50, sorted — never depth-annotated.
    let exclude: std::collections::HashSet<&String> =
        defined_in.iter().chain(reach.keys()).collect();
    let mut lexical_candidates: Vec<String> = search_with_conn(&conn, Some(project), symbol, 50)?
        .into_iter()
        .map(|h| h.path)
        .filter(|p| !exclude.contains(p))
        .filter(|p| !exclude_tests || !is_test_path(p))
        .collect();
    lexical_candidates.sort();
    lexical_candidates.dedup();

    Ok(Impact {
        symbol: symbol.to_string(),
        direction: direction.as_str().to_string(),
        defined_in,
        ast_dependents,
        rows,
        lexical_candidates,
        truncated,
        result_total,
        truncated_reason: truncated.then(|| "max_results".to_string()),
    })
}

/// Tokenize a search query and DEDUPE repeated terms (first occurrence wins —
/// deterministic, order-preserving). `tokenize` emits whole + camel-split + stem
/// forms, so a repeated word (or camelCase parts recurring across query words)
/// would appear twice in the MATCH OR-list and double-count in bm25, biasing
/// multi-term queries toward the repeated term. Conductr's reference query path
/// scores a deduped term set; mirror it (ADR-010 cluster 7).
fn query_tokens(query: &str) -> Vec<String> {
    let mut toks = tokenize::tokenize(query);
    let mut seen = std::collections::HashSet::new();
    toks.retain(|t| seen.insert(t.clone()));
    toks
}

/// Build the FTS5 column-filtered OR query for a set of pre-tokenized query terms.
/// Tokens are pure ASCII alnum, quoted as FTS5 string literals.
fn build_match(columns: &str, q_tokens: &[String]) -> String {
    let or_clause = q_tokens
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("{{{columns}}} : ({or_clause})")
}

/// Run one BM25 leg, returning ranked `(project_id, path, fts_rowid)` best-first.
/// The rowid rides along so the coverage probes can restrict their MATCH to the
/// candidate set (O(tokens × candidates), never a full posting walk).
fn run_leg(
    conn: &Connection,
    match_expr: &str,
    project: Option<&str>,
    w: (f64, f64, f64),
    limit: usize,
) -> rusqlite::Result<Vec<(String, String, i64)>> {
    // Weights are fixed constants (no injection risk); inline them since FTS5
    // bm25() column-weight args are not reliably bindable.
    let bm25 = format!("bm25(code_fts, {:.4}, {:.4}, {:.4})", w.0, w.1, w.2);
    let mut rows: Vec<(String, String, i64)> = Vec::new();
    match project {
        Some(pid) => {
            let sql = format!(
                "SELECT f.project_id, f.path, code_fts.rowid FROM code_fts
                 JOIN files f ON f.fts_rowid = code_fts.rowid
                 WHERE code_fts MATCH ?1 AND f.project_id = ?2
                 ORDER BY {bm25}, f.path LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let it = stmt.query_map(
                rusqlite::params![match_expr, pid, limit as i64],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
            )?;
            for x in it {
                rows.push(x?);
            }
        }
        None => {
            let sql = format!(
                "SELECT f.project_id, f.path, code_fts.rowid FROM code_fts
                 JOIN files f ON f.fts_rowid = code_fts.rowid
                 WHERE code_fts MATCH ?1
                 ORDER BY {bm25}, f.path LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let it = stmt.query_map(
                rusqlite::params![match_expr, limit as i64],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
            )?;
            for x in it {
                rows.push(x?);
            }
        }
    }
    Ok(rows)
}

/// Probe which candidates each DISTINCT query token matches: ONE FTS5 MATCH per
/// token, restricted to the candidate rowids. Cost is O(tokens × candidates) with
/// small constants — pure FTS index lookups over ≤ `COVERAGE_MAX_PROBE_TOKENS`
/// tokens and the (overfetch-bounded) candidate set; no content rescans. Returns
/// `token_hits[i]` = set of composite ids matched by probed token i.
fn probe_token_hits(
    conn: &Connection,
    q_tokens: &[String],
    rowid_of: &std::collections::HashMap<String, i64>,
) -> rusqlite::Result<Vec<std::collections::HashSet<String>>> {
    let id_of: std::collections::HashMap<i64, &String> =
        rowid_of.iter().map(|(id, rid)| (*rid, id)).collect();
    // Rowids come from our own SELECT (i64) — inlining is injection-free and lets
    // one prepared statement serve every token probe.
    let mut rowids: Vec<i64> = rowid_of.values().copied().collect();
    rowids.sort_unstable();
    let in_list = rowids.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(",");
    let sql = format!("SELECT rowid FROM code_fts WHERE code_fts MATCH ?1 AND rowid IN ({in_list})");
    let mut stmt = conn.prepare(&sql)?;
    let mut token_hits: Vec<std::collections::HashSet<String>> = Vec::new();
    for tok in q_tokens.iter().take(COVERAGE_MAX_PROBE_TOKENS) {
        // Same quoting as `build_match`: tokens are [a-z0-9]+ from the tokenizer.
        let m = format!("\"{tok}\"");
        let mut hits = std::collections::HashSet::new();
        let it = stmt.query_map([&m], |r| r.get::<_, i64>(0))?;
        for rid in it {
            if let Some(id) = id_of.get(&rid?) {
                hits.insert((*id).clone());
            }
        }
        token_hits.push(hits);
    }
    Ok(token_hits)
}

/// PURE coverage computation: candidate id → COUNT of DISTINCT probed tokens
/// whose hit-set contains it (fractions derive from `count / token_hits.len()`).
/// `token_hits` must be built from an already-DEDUPED token list (`query_tokens`
/// — ADR-010 cluster 7), so a repeated query word can never deflate or
/// double-count coverage; the denominator is the number of distinct probed tokens.
fn coverage_counts(
    token_hits: &[std::collections::HashSet<String>],
    candidates: impl Iterator<Item = String>,
) -> std::collections::HashMap<String, usize> {
    candidates
        .map(|id| {
            let matched = token_hits.iter().filter(|s| s.contains(&id)).count();
            (id, matched)
        })
        .collect()
}

/// PURE gate+blend over the fused list (see the COVERAGE_* docs):
///   gate: keep a hit iff `matched ≥ gate_ratio · best_matched` (the argmax hit
///         always survives) OR — when best is PARTIAL (`best < n_tokens`, a
///         concept-bag query) — `matched ≥ COVERAGE_RESCUE_MIN`.
///         `matched == 0` is UNKNOWN coverage, never zero (a candidate matched
///         ≥1 query token to become a candidate; zero probed matches only
///         happens when its matches lie beyond COVERAGE_MAX_PROBE_TOKENS) —
///         such hits are kept, blend-neutral. Defense in depth only: the call
///         site already skips the gate on >cap-token queries, where ALL probed
///         counts (not just 0) are untrustworthy undercounts.
///   blend: `score *= 1 + w · matched/n_tokens` on the kept hits.
/// Integer counts (not floats) do the gate compare, so boundary cases are exact.
/// Order-stable: `retain` keeps fused order and the caller re-sorts with the
/// canonical comparator afterwards.
fn apply_coverage(
    fused: &mut Vec<(String, f64)>,
    matched: &std::collections::HashMap<String, usize>,
    n_tokens: usize,
    w: f64,
    gate_ratio: f64,
) {
    if n_tokens == 0 {
        return;
    }
    let best = fused.iter().filter_map(|(id, _)| matched.get(id).copied()).max().unwrap_or(0);
    if gate_ratio > 0.0 && best > 0 {
        let rescue_active = best < n_tokens;
        fused.retain(|(id, _)| {
            let m = matched.get(id).copied().unwrap_or(0);
            // m == 0 ⇒ UNKNOWN coverage (matches beyond the probe cap), not a
            // stray-token hit — keep it; the blend below is neutral at m = 0.
            m == 0
                || m as f64 >= gate_ratio * best as f64
                || (rescue_active && m >= COVERAGE_RESCUE_MIN)
        });
    }
    for (id, score) in fused.iter_mut() {
        let m = matched.get(id).copied().unwrap_or(0);
        *score *= 1.0 + w * (m as f64 / n_tokens as f64);
        debug_assert!(score.is_finite(), "coverage blend produced a non-finite score");
    }
}

/// The tunable ranking weights — the per-column bm25 weights for each FTS5 leg plus
/// the per-leg RRF weights. Extracted so the offline calibration sweep
/// (`brain_bench`) can vary them over the labeled corpus; production search uses
/// [SearchWeights::defaults] (the consts above).
#[derive(Clone, Copy, Debug)]
pub struct SearchWeights {
    /// bm25 (path, symbols, content) weights for the identity leg.
    pub identity_bm25: (f64, f64, f64),
    /// bm25 (path, symbols, content) weights for the content leg.
    pub content_bm25: (f64, f64, f64),
    /// RRF fusion weight for the identity leg.
    pub rrf_identity: f64,
    /// RRF fusion weight for the content leg.
    pub rrf_content: f64,
    /// Multiplicative coverage blend weight (`score *= 1 + w·coverage`). 0 disables.
    pub coverage_w: f64,
    /// Relative coverage gate: drop hits with `coverage < ratio · best_coverage`
    /// on multi-token queries. 0 disables (the calibration/anti-vanity seam).
    pub coverage_gate_ratio: f64,
}

impl Default for SearchWeights {
    fn default() -> Self {
        Self {
            identity_bm25: W_IDENTITY,
            content_bm25: W_CONTENT,
            rrf_identity: RRF_W_IDENTITY,
            rrf_content: RRF_W_CONTENT,
            coverage_w: COVERAGE_W,
            coverage_gate_ratio: COVERAGE_GATE_RATIO,
        }
    }
}

/// Core hybrid search over an arbitrary connection (writer reuse or r/o reader),
/// with the PRODUCTION weights. Thin wrapper over [search_with_weights].
pub fn search_with_conn(
    conn: &Connection,
    project: Option<&str>,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    search_with_weights(conn, project, query, limit, &SearchWeights::default())
}

/// [search_with_conn] with conventional test paths excluded — the code_impact
/// `exclude_tests` knob (see [is_test_path]) applied to the SEARCH path
/// (gauntlet S2 `no-test-exclusion-in-gist-search`). Test rows are dropped
/// BEFORE the limit cut, so an agent-facing consumer (the gist's "Relevant
/// files" budget) still receives up to `limit` PRODUCTION hits instead of
/// spending its file budget on tests/fixtures that lexically outrank them.
pub fn search_excluding_tests_with_conn(
    conn: &Connection,
    project: Option<&str>,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    search_core(conn, project, query, limit, &SearchWeights::default(), true)
}

/// [search_readonly] with conventional test paths excluded — the fresh-r/o-
/// connection twin of [search_excluding_tests_with_conn] (command-thread path).
pub fn search_readonly_excluding_tests(
    db_path: &Path,
    project: Option<&str>,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    let conn = open_readonly(db_path)?;
    search_excluding_tests_with_conn(&conn, project, query, limit)
}

/// Core hybrid search with EXPLICIT weights (the calibration seam). Identical to
/// `search_with_conn` when given `SearchWeights::default()` — proven by
/// `search_with_weights_defaults_equal_search_with_conn`.
pub fn search_with_weights(
    conn: &Connection,
    project: Option<&str>,
    query: &str,
    limit: usize,
    w: &SearchWeights,
) -> rusqlite::Result<Vec<Hit>> {
    search_core(conn, project, query, limit, w, false)
}

/// The ONE ranked-retrieval pipeline behind every search entry point.
/// `exclude_tests` applies [is_test_path] AFTER fusion/re-ranks but BEFORE the
/// limit cut (the same filter point as code_impact's rows), drawing replacements
/// from the per-leg overfetch pool — `false` is byte-identical to the historical
/// behavior. Deterministic either way (pure filter over an ordered list), so the
/// gist byte-identity gate is preserved.
fn search_core(
    conn: &Connection,
    project: Option<&str>,
    query: &str,
    limit: usize,
    w: &SearchWeights,
    exclude_tests: bool,
) -> rusqlite::Result<Vec<Hit>> {
    let q_tokens = query_tokens(query);
    if q_tokens.is_empty() {
        return Ok(Vec::new());
    }
    let overfetch = (limit * 4).max(40);
    // The two FTS5 legs — labelled by SEARCH_LEG_LABELS (identity, content). A
    // semantic vector leg is NOT fused here in v1 (the P5 no-vector-leg invariant).
    let leg_a = run_leg(conn, &build_match("path symbols", &q_tokens), project, w.identity_bm25, overfetch)?;
    let leg_b = run_leg(conn, &build_match("content", &q_tokens), project, w.content_bm25, overfetch)?;

    // Composite id "project\0path" keeps paths unique across projects.
    let key = |p: &(String, String, i64)| format!("{}\u{0}{}", p.0, p.1);
    let a_ids: Vec<String> = leg_a.iter().map(key).collect();
    let b_ids: Vec<String> = leg_b.iter().map(key).collect();
    let mut fused = rank::weighted_rrf(&[
        Leg { weight: w.rrf_identity, ranked: &a_ids },
        Leg { weight: w.rrf_content, ranked: &b_ids },
    ]);

    // V3 coverage re-rank (see COVERAGE_W docs): only meaningful on multi-token
    // queries — with one distinct token every candidate has coverage 1 by
    // construction (it matched to become a candidate), so probes are skipped.
    if q_tokens.len() >= 2 && (w.coverage_w > 0.0 || w.coverage_gate_ratio > 0.0) {
        let rowid_of: std::collections::HashMap<String, i64> = leg_a
            .iter()
            .chain(leg_b.iter())
            .map(|p| (key(p), p.2))
            .collect();
        let token_hits = probe_token_hits(conn, &q_tokens, &rowid_of)?;
        let matched = coverage_counts(&token_hits, fused.iter().map(|(id, _)| id.clone()));
        // Beyond the probe cap every count is an UNDERCOUNT (tail-token matches
        // are invisible): counts may still BLEND (monotone, neutral at 0) but
        // must never EXCLUDE — skip the gate (see COVERAGE_MAX_PROBE_TOKENS).
        let gate_ratio =
            if q_tokens.len() > COVERAGE_MAX_PROBE_TOKENS { 0.0 } else { w.coverage_gate_ratio };
        apply_coverage(&mut fused, &matched, token_hits.len(), w.coverage_w, gate_ratio);
    }

    // V2 temporal re-rank ([DP-12]): a snapshot-stable multiplicative boost applied
    // AFTER fusion (RRF stays leg-pure — a per-doc multiplier is a document property,
    // not a leg). All inputs are STORED + read from this connection's snapshot, so
    // two reads of an unchanged index re-derive the same order → byte-identical gist.
    apply_temporal_boost(conn, &mut fused)?;

    let hits = fused
        .into_iter()
        .filter_map(|(id, score)| {
            id.split_once('\u{0}').map(|(proj, path)| Hit {
                project: proj.to_string(),
                path: path.to_string(),
                score,
            })
        })
        .filter(|h| !exclude_tests || !is_test_path(&h.path))
        .take(limit)
        .collect();
    Ok(hits)
}

/// Multiply each fused score by its [temporal_boost] and re-sort with the SAME
/// comparator as `weighted_rrf` (score desc, then composite id asc) so the
/// deterministic tie-break is preserved. `ref_ms` = MAX(accessed_at_ms) over the
/// scope on THIS connection (snapshot-stable). A no-op (uniform boost) when all
/// files are unstamped (accessed_* == 0).
fn apply_temporal_boost(
    conn: &Connection,
    fused: &mut [(String, f64)],
) -> rusqlite::Result<()> {
    if fused.is_empty() {
        return Ok(());
    }
    // BOUNDED probe (the ADR-010 perf-pair fix): only the candidates being ranked
    // are looked up — they are already capped by the per-leg overfetch — plus ONE
    // `files_recency`-index MAX seek per distinct candidate project for its
    // `ref_ms`. Never a full files-table scan. Semantics are IDENTICAL to the
    // historical full scan (pinned bit-for-bit by
    // `temporal_boost_bounded_probe_matches_full_scan`): a missing files row reads
    // as (0, 0) and a project with no rows as ref_ms = 0, exactly as the scan's
    // absent map entries did; per-project ref_ms keeps a doc's age relative to its
    // OWN project on cross-project searches. Same snapshot connection → the gist
    // byte-identity gate is untouched.
    let mut row_stmt = conn.prepare(
        "SELECT accessed_at_ms, accessed_count FROM files WHERE project_id=?1 AND path=?2",
    )?;
    let mut ref_stmt = conn.prepare("SELECT MAX(accessed_at_ms) FROM files WHERE project_id=?1")?;
    let mut proj_ref: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (id, score) in fused.iter_mut() {
        // Ids are "project\0path" by construction; a malformed id degrades to the
        // same neutral (0,0)/ref 0 boost the full scan gave it.
        let (proj, path) = id.split_once('\u{0}').unwrap_or(("", ""));
        let (at, count): (i64, i64) = match row_stmt
            .query_row((proj, path), |r| Ok((r.get(0)?, r.get(1)?)))
        {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => (0, 0),
            Err(e) => return Err(e),
        };
        let ref_ms = match proj_ref.get(proj) {
            Some(v) => *v,
            None => {
                let v: i64 = ref_stmt
                    .query_row([proj], |r| r.get::<_, Option<i64>>(0))?
                    .unwrap_or(0);
                proj_ref.insert(proj.to_string(), v);
                v
            }
        };
        *score *= temporal_boost(at, count, ref_ms);
        debug_assert!(score.is_finite(), "temporal boost produced a non-finite score");
    }
    // Re-sort with the weighted_rrf comparator (score desc, id asc) — see rank.rs.
    // Scores are provably finite (RRF sum × bounded quantized boost); any future
    // score source (a vector leg) MUST preserve finiteness or this degrades to id-only.
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0))
    });
    Ok(())
}

pub(crate) fn open_readonly(db_path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    // Wait out a transient writer lock instead of returning a silent empty
    // result during indexing/checkpoint (esp. on Windows).
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(conn)
}

/// Open a read-only connection and pin ONE WAL snapshot for the whole read
/// session via a deferred read transaction. Every statement run on the returned
/// connection observes the same index state, so a multi-read consumer (the gist)
/// cannot tear across a concurrent worker commit — the P3 byte-identity gate
/// depends on the cache key (fingerprint) and the rendered body coming from one
/// state. Dropping the connection rolls the (read-only) transaction back.
pub fn open_readonly_snapshot(db_path: &Path) -> rusqlite::Result<Connection> {
    let conn = open_readonly(db_path)?;
    conn.execute_batch("BEGIN DEFERRED")?;
    // The WAL read mark is taken on the first table read; force it now by
    // touching the schema b-tree so the snapshot is pinned at open time,
    // independent of which read the caller runs first.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
    Ok(conn)
}

/// Search via a fresh read-only connection (command-thread path). Fail-soft: if
/// the DB isn't there yet, callers get an empty result, not an error.
pub fn search_readonly(
    db_path: &Path,
    project: Option<&str>,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    let conn = open_readonly(db_path)?;
    search_with_conn(&conn, project, query, limit)
}

/// Project aggregate fingerprint over a caller-supplied connection (gist cache
/// key). Use `*_with_conn` variants when several reads must share one snapshot.
pub fn project_fingerprint_with_conn(conn: &Connection, project_id: &str) -> rusqlite::Result<String> {
    let mut stmt = conn.prepare("SELECT path, hash FROM files WHERE project_id=?1")?;
    let it = stmt.query_map([project_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for x in it {
        entries.push(x?);
    }
    Ok(crate::modules::brain::freshness::aggregate_fingerprint(&mut entries))
}

/// Project aggregate fingerprint via a fresh read-only connection.
pub fn project_fingerprint_readonly(db_path: &Path, project_id: &str) -> rusqlite::Result<String> {
    project_fingerprint_with_conn(&open_readonly(db_path)?, project_id)
}

/// Digest of the project's TEMPORAL state — blake3 over the sorted
/// `(path, accessed_at_ms, accessed_count)` tuples. The temporal boost
/// ([temporal_boost]) shapes the gist body order, so this is folded into the gist
/// cache KEY alongside the (content-only, portable) fingerprint: any record_access
/// movement rotates the key, so the same key can never map to two different gist
/// bodies (no fingerprint-cache poisoning across reindex histories). Kept SEPARATE
/// from the fingerprint so the fingerprint stays content-portable across machines.
pub fn project_temporal_digest_with_conn(conn: &Connection, project_id: &str) -> rusqlite::Result<String> {
    let mut stmt =
        conn.prepare("SELECT path, accessed_at_ms, accessed_count FROM files WHERE project_id=?1")?;
    let it = stmt.query_map([project_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })?;
    let mut rows: Vec<(String, i64, i64)> = Vec::new();
    for x in it {
        rows.push(x?);
    }
    rows.sort(); // order-independent (Merkle-style)
    let mut h = blake3::Hasher::new();
    for (path, at, count) in &rows {
        h.update(path.as_bytes());
        h.update(b"\0");
        h.update(&at.to_le_bytes());
        h.update(b"\0");
        h.update(&count.to_le_bytes());
        h.update(b"\n");
    }
    Ok(h.finalize().to_hex().to_string())
}

/// Top distinct definition names in a file (sorted) over a caller-supplied
/// connection — for the gist skeleton.
pub fn symbols_for_path_with_conn(
    conn: &Connection,
    project_id: &str,
    path: &str,
    limit: usize,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT name FROM code_nodes WHERE project_id=?1 AND path=?2 ORDER BY name LIMIT ?3",
    )?;
    let it = stmt.query_map(rusqlite::params![project_id, path, limit as i64], |r| {
        r.get::<_, String>(0)
    })?;
    let mut v = Vec::new();
    for x in it {
        v.push(x?);
    }
    Ok(v)
}

/// Top distinct definition names in a file via a fresh read-only connection.
pub fn symbols_for_path_readonly(
    db_path: &Path,
    project_id: &str,
    path: &str,
    limit: usize,
) -> rusqlite::Result<Vec<String>> {
    symbols_for_path_with_conn(&open_readonly(db_path)?, project_id, path, limit)
}

/// File count for a project over a caller-supplied connection.
pub fn file_count_with_conn(conn: &Connection, project_id: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM files WHERE project_id=?1",
        [project_id],
        |r| r.get(0),
    )
}

/// File count for a project via a fresh read-only connection.
pub fn file_count_readonly(db_path: &Path, project_id: &str) -> rusqlite::Result<i64> {
    file_count_with_conn(&open_readonly(db_path)?, project_id)
}

/// Reflect budget `(ceiling_usd, spent_total_usd)` via a read-only connection
/// (the command-thread status read — WAL → wait-free vs the writer). P4.
pub fn budget_state_readonly(db_path: &Path) -> rusqlite::Result<(f64, f64)> {
    let conn = open_readonly(db_path)?;
    conn.query_row(
        "SELECT ceiling_usd, spent_total_usd FROM brain_budget WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// The Librarian LLM selection `(provider, model, base_url, in_rate_mtok, out_rate_mtok)`
/// via a read-only connection (the command-thread status read). Fail-soft to the
/// Anthropic Haiku default so a pre-table DB or read race never errors the UI.
pub fn librarian_config_readonly(
    db_path: &Path,
) -> rusqlite::Result<(String, String, String, f64, f64)> {
    let conn = open_readonly(db_path)?;
    Ok(conn
        .query_row(
            "SELECT provider, model, base_url, in_rate_mtok, out_rate_mtok FROM brain_librarian WHERE id=1",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, f64>(4)?,
                ))
            },
        )
        .unwrap_or_else(|_| {
            ("anthropic".to_string(), "claude-haiku-4-5".to_string(), String::new(), 1.0, 5.0)
        }))
}

/// Recent Librarian LLM calls (the budget ledger) via a read-only connection:
/// `(status, est_cost_usd, actual_cost_usd, model, reserved_at_ms)`, newest first.
/// Each row is a real reflect/curate call (reserved before the call, spent after).
pub fn librarian_ledger_readonly(
    db_path: &Path,
    limit: i64,
) -> rusqlite::Result<Vec<(String, f64, Option<f64>, String, i64)>> {
    let conn = open_readonly(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT status, est_cost_usd, actual_cost_usd, model, reserved_at \
         FROM brain_budget_ledger ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, f64>(1)?,
            r.get::<_, Option<f64>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    rows.collect()
}

/// Count of unresolved (pending) review-inbox proposals via a read-only connection.
pub fn pending_proposals_readonly(db_path: &Path) -> rusqlite::Result<i64> {
    let conn = open_readonly(db_path)?;
    conn.query_row(
        "SELECT COUNT(*) FROM proposals WHERE status='pending'",
        [],
        |r| r.get(0),
    )
}

/// The semantic `embedderId` header `(embedder_id, dims)` via a read-only
/// connection. Empty `("", 0)` in v1 (no embedder); set at enablement. P5.
///
/// Fail-soft: a missing table/row (e.g. a pre-v7 DB read before the worker
/// migrates) maps to `("", 0)` — the "no embedder" state — rather than erroring.
/// `Err` only when the store FILE is absent (genuinely not ready; callers fail-soft).
pub fn semantic_meta_readonly(db_path: &Path) -> rusqlite::Result<(String, i64)> {
    let conn = open_readonly(db_path)?;
    Ok(conn
        .query_row(
            "SELECT embedder_id, dims FROM brain_semantic_meta WHERE id=1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap_or_else(|_| (String::new(), 0)))
}

fn note_summary_from_row(r: &rusqlite::Row) -> rusqlite::Result<NoteSummary> {
    let anchors_json: String = r.get(5)?;
    let anchors: Vec<String> = serde_json::from_str(&anchors_json).unwrap_or_default();
    Ok(NoteSummary {
        id: r.get(0)?,
        title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
        note_type: r.get(2)?,
        status: r.get(3)?,
        path: r.get(4)?,
        anchors,
    })
}

fn proposal_from_row(r: &rusqlite::Row) -> rusqlite::Result<MemoryProposal> {
    let action =
        ProposalAction::from_token(&r.get::<_, String>(2)?).unwrap_or(ProposalAction::Update);
    Ok(MemoryProposal {
        project: r.get(0)?,
        signature: r.get(1)?,
        action,
        target_id: r.get(3)?,
        title: r.get(4)?,
        detail: r.get(5)?,
        source: r.get(6)?,
        status: r.get(7)?,
    })
}

/// List PENDING proposals (the review inbox) via a read-only connection.
pub fn list_proposals_readonly(
    db_path: &Path,
    project: Option<&str>,
) -> rusqlite::Result<Vec<MemoryProposal>> {
    let conn = open_readonly(db_path)?;
    let base = "SELECT project_id,signature,action,target_id,title,detail,source,status FROM proposals WHERE status='pending'";
    let mut rows = Vec::new();
    match project {
        Some(pid) => {
            let mut stmt = conn.prepare(&format!("{base} AND project_id=?1 ORDER BY created_ms"))?;
            let it = stmt.query_map([pid], proposal_from_row)?;
            for x in it {
                rows.push(x?);
            }
        }
        None => {
            let mut stmt = conn.prepare(&format!("{base} ORDER BY project_id, created_ms"))?;
            let it = stmt.query_map([], proposal_from_row)?;
            for x in it {
                rows.push(x?);
            }
        }
    }
    Ok(rows)
}

/// List structured memory notes (for the review inbox / cards) over a
/// caller-supplied connection. `project = None` lists every project's notes.
pub fn list_notes_with_conn(
    conn: &Connection,
    project: Option<&str>,
) -> rusqlite::Result<Vec<NoteSummary>> {
    let mut rows = Vec::new();
    match project {
        Some(pid) => {
            let mut stmt = conn.prepare(
                "SELECT id,title,note_type,status,path,COALESCE(anchors,'[]') FROM notes WHERE project_id=?1 ORDER BY id",
            )?;
            let it = stmt.query_map([pid], note_summary_from_row)?;
            for x in it {
                rows.push(x?);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id,title,note_type,status,path,COALESCE(anchors,'[]') FROM notes ORDER BY project_id,id",
            )?;
            let it = stmt.query_map([], note_summary_from_row)?;
            for x in it {
                rows.push(x?);
            }
        }
    }
    Ok(rows)
}

/// List structured memory notes via a fresh read-only connection.
pub fn list_notes_readonly(
    db_path: &Path,
    project: Option<&str>,
) -> rusqlite::Result<Vec<NoteSummary>> {
    list_notes_with_conn(&open_readonly(db_path)?, project)
}

/// A note as the GIST memory layer needs it — `NoteSummary`'s display fields plus
/// the per-claim freshness-label inputs (ADR-011): `created`, `revalidate_after`,
/// the supersession edges, and anchors. All note-file-derived state (note files
/// are indexed), so it is covered by the gist cache key's content fingerprint.
/// (`revalidate_after`'s wall-clock-dependent OVERDUE outcome is folded into the
/// key separately — see the gist's overdue-set digest.)
#[derive(Clone, Debug)]
pub struct GistNote {
    pub id: String,
    pub title: String,
    pub note_type: Option<String>,
    pub created: Option<String>,
    pub revalidate_after: Option<String>,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub anchors: Vec<String>,
}

/// Notes for the gist memory layer over a caller-supplied (pinned-snapshot)
/// connection. ORDER BY id — deterministic (the gist byte-identity gate).
pub fn gist_notes_with_conn(
    conn: &Connection,
    project_id: &str,
) -> rusqlite::Result<Vec<GistNote>> {
    let mut stmt = conn.prepare(
        "SELECT id,title,note_type,created,revalidate_after,supersedes,superseded_by,COALESCE(anchors,'[]')
         FROM notes WHERE project_id=?1 ORDER BY id",
    )?;
    let it = stmt.query_map([project_id], |r| {
        let anchors: Vec<String> =
            serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default();
        Ok(GistNote {
            id: r.get(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            note_type: r.get(2)?,
            created: r.get(3)?,
            revalidate_after: r.get(4)?,
            supersedes: r.get(5)?,
            superseded_by: r.get(6)?,
            anchors,
        })
    })?;
    let mut v = Vec::new();
    for x in it {
        v.push(x?);
    }
    Ok(v)
}

/// Temporal touch state `(accessed_at_ms, accessed_count)` of one indexed file,
/// `None` when the path isn't indexed. Input to the gist's per-claim freshness
/// labels (ADR-011) — covered by the gist cache key via the temporal digest.
pub fn file_touch_with_conn(
    conn: &Connection,
    project_id: &str,
    path: &str,
) -> rusqlite::Result<Option<(i64, i64)>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT accessed_at_ms, accessed_count FROM files WHERE project_id=?1 AND path=?2",
        (project_id, path),
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("index.sqlite");
        (dir, path)
    }

    /// ADR-010 cluster 7: repeated query terms are deduped (first occurrence wins)
    /// before the MATCH so they can't double-count in bm25. camelCase queries emit
    /// whole + part tokens, so duplicates arise even from distinct words.
    #[test]
    fn query_tokens_dedupes_preserving_first_occurrence_order() {
        assert_eq!(query_tokens("getUser getUser data"), vec!["getuser", "get", "user", "data"]);
        // Parts recurring ACROSS words dedupe too ("user" from both camel splits).
        assert_eq!(
            query_tokens("getUser userData"),
            vec!["getuser", "get", "user", "userdata", "data"]
        );
        assert!(query_tokens("").is_empty());
    }

    /// ADR-010 cluster 4: a NOTADB cache file (garbage bytes) must not brick the
    /// brain — it is moved aside (kept, never deleted) and a fresh store rebuilt.
    #[test]
    fn open_with_recovery_rebuilds_a_notadb_cache() {
        let (_dir, path) = temp_db();
        std::fs::write(&path, b"definitely not a sqlite database, sorry").unwrap();
        let idx = SqliteIndex::open_with_recovery(&path).expect("recovery open");
        idx.index_file("p", "a.rs", "fn a() {}", "h", 9).unwrap();
        assert_eq!(idx.existing_paths("p").unwrap().len(), 1, "rebuilt store works");
        let aside: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "corrupt original moved aside, not deleted");
    }

    /// ADR-010 cluster 4: the corrupt-rebuild path must best-effort carry the
    /// CANONICAL rows (human decisions + spend state) into the fresh store —
    /// including singletons whose fresh-DDL seed row must lose to the salvaged one.
    #[test]
    fn recovery_salvages_canonical_tables_from_the_old_store() {
        let (_dir, path) = temp_db();
        {
            let old = SqliteIndex::open(&path).unwrap();
            old.conn
                .execute(
                    "INSERT INTO proposals(project_id,signature,action,target_id,title,detail,source,status,created_ms)
                     VALUES('p','sig','archive','n','t','d','curate','rejected',1)",
                    [],
                )
                .unwrap();
            old.conn
                .execute("INSERT INTO reject_signatures(project_id,reject_sig) VALUES('p','rj')", [])
                .unwrap();
            old.conn
                .execute("UPDATE brain_budget SET ceiling_usd=2.0, spent_total_usd=0.42 WHERE id=1", [])
                .unwrap();
        } // closed → WAL checkpointed into the main file
        let aside = path.with_file_name("index.sqlite.corrupt-test-0");
        std::fs::rename(&path, &aside).unwrap();
        let fresh = SqliteIndex::open(&path).unwrap();
        fresh.salvage_canonical(&aside);
        let props: i64 = fresh
            .conn
            .query_row("SELECT COUNT(*) FROM proposals WHERE status='rejected'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(props, 1, "rejected proposal salvaged (declined stays declined)");
        let rejects: i64 = fresh
            .conn
            .query_row("SELECT COUNT(*) FROM reject_signatures", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rejects, 1, "reject history salvaged");
        let (ceiling, spent): (f64, f64) = fresh
            .conn
            .query_row("SELECT ceiling_usd, spent_total_usd FROM brain_budget WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((ceiling, spent), (2.0, 0.42), "budget spend replaces the fresh seed row");
    }

    /// ADR-010 cluster 4: transient BUSY at boot is retried (bounded) and NEVER
    /// misclassified as corruption — the cache file stays exactly where it is.
    #[test]
    fn boot_busy_is_retried_bounded_and_never_treated_as_corruption() {
        let (_dir, path) = temp_db();
        // A pre-WAL (rollback-journal) db whose EXCLUSIVE lock blocks even the
        // `journal_mode=WAL` pragma → the immediate-BUSY shape of a boot race.
        let blocker = Connection::open(&path).unwrap();
        blocker.execute_batch("CREATE TABLE keepme(x); BEGIN EXCLUSIVE").unwrap();
        let res =
            SqliteIndex::open_with_recovery_at(&path, 2, std::time::Duration::from_millis(10));
        assert!(res.is_err(), "still-locked store fails after the bounded retries");
        let no_aside = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(no_aside, "BUSY must never move the cache aside");
        assert!(path.exists(), "cache file untouched by a busy boot");
        blocker.execute_batch("ROLLBACK").unwrap();
        drop(blocker);
        let idx = SqliteIndex::open_with_recovery(&path).expect("opens once the lock clears");
        let n: i64 = idx.conn.query_row("SELECT COUNT(*) FROM keepme", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "pre-existing data intact after the lock clears");
    }

    #[test]
    fn index_then_search_finds_by_identifier() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file(
            "p1",
            "src/auth/login.rs",
            "pub fn validateUserCredentials(user: &User) -> bool { true }",
            "h1",
            64,
        )
        .expect("index");
        idx.index_file("p1", "src/ui/button.rs", "fn render() {}", "h2", 16)
            .expect("index");

        let hits = search_with_conn(&idx.conn, Some("p1"), "validate credentials", 10).expect("search");
        assert!(!hits.is_empty(), "expected a hit");
        assert_eq!(hits[0].path, "src/auth/login.rs");
    }

    #[test]
    fn path_match_outranks_body_only() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        // login.rs: query word in the PATH; other.rs: query word only in body.
        idx.index_file("p", "src/login.rs", "fn handler() {}", "a", 10).unwrap();
        idx.index_file("p", "src/other.rs", "// login flow happens here", "b", 10).unwrap();
        let hits = search_with_conn(&idx.conn, Some("p"), "login", 10).unwrap();
        assert_eq!(hits[0].path, "src/login.rs", "path hit should win, got {hits:?}");
    }

    #[test]
    fn temporal_boost_rewards_fresh_and_frequent() {
        let r = 1_000_000_000i64; // ref_ms
        let fresh_freq = temporal_boost(r, 20, r); // age 0 (fresh), count 20
        let stale_rare = temporal_boost(r - 120 * DAY_MS, 0, r); // age 120d (>90 → bucket 0)
        assert!(fresh_freq > stale_rare, "fresh+frequent must out-boost stale+rare");
        // a fresh doc beats a 120d-stale one even with equal (zero) frequency.
        assert!(temporal_boost(r, 0, r) > temporal_boost(r - 120 * DAY_MS, 0, r));
        // all-zero (unstamped) inputs → a fixed UNIFORM boost (no reordering); an
        // unstamped file gets the NEUTRAL recency factor, NOT "maximally stale".
        assert_eq!(temporal_boost(0, 0, 0), temporal_boost(0, 0, 0));
        assert_eq!(temporal_boost(0, 0, 0), 1.0 + RECENCY_W * NEUTRAL_RECENCY);
        // an unstamped file is NEUTRAL, strictly above a truly-stale (bucket 0) file.
        assert!(temporal_boost(0, 0, 1_000_000) > temporal_boost(1, 0, 1_000_000 + 120 * DAY_MS));
        // bounded: never more than the max recency × max freq factor.
        assert!(fresh_freq <= (1.0 + RECENCY_W) * (1.0 + FREQ_W) + 1e-9);
    }

    #[test]
    fn temporal_boost_cannot_flip_cross_leg() {
        // THE boundedness invariant ([DP-2] protection): the largest possible boost
        // must stay strictly below the cross-leg RRF margin, so a fully-boosted
        // body-only (content-leg) hit can never outrank an un-boosted path/identity
        // hit. Without this, temporal recency would bury filename matches.
        let max_boost = (1.0 + RECENCY_W) * (1.0 + FREQ_W);
        let cross_leg_margin = RRF_W_IDENTITY / RRF_W_CONTENT;
        assert!(
            max_boost < cross_leg_margin,
            "max temporal boost {max_boost} must be < cross-leg margin {cross_leg_margin} (else recency buries path-matches)"
        );
    }

    #[test]
    fn recency_reorders_equal_score_files() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        // identical content → identical bm25 → adjacent RRF ranks (a_old wins on path asc).
        idx.index_file("p", "src/a_old.rs", "pub fn sharedThing() {}", "h1", 24).unwrap();
        idx.index_file("p", "src/z_new.rs", "pub fn sharedThing() {}", "h2", 24).unwrap();
        // Without recency, a_old is rank-1 (path tie-break).
        let base = search_with_conn(&idx.conn, Some("p"), "shared thing", 10).unwrap();
        assert_eq!(base[0].path, "src/a_old.rs", "baseline: path tie-break → a_old first");
        // Stamp z_new as fresh and a_old as 40 days stale.
        let now = 100 * DAY_MS;
        idx.record_access("p", "src/z_new.rs", now).unwrap();
        idx.record_access("p", "src/a_old.rs", now - 40 * DAY_MS).unwrap();
        let boosted = search_with_conn(&idx.conn, Some("p"), "shared thing", 10).unwrap();
        assert_eq!(boosted[0].path, "src/z_new.rs", "recency promotes the fresher equal-score file: {boosted:?}");
    }

    #[test]
    fn temporal_boost_is_byte_stable_across_reads() {
        // Two searches over an UNCHANGED (but recency-stamped) index must be identical
        // — the property the gist byte-identity gate depends on.
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "src/a.rs", "pub fn alpha() {}", "h1", 20).unwrap();
        idx.index_file("p", "src/b.rs", "pub fn alphaTwo() {}", "h2", 24).unwrap();
        idx.record_access("p", "src/a.rs", 5 * DAY_MS).unwrap();
        idx.record_access("p", "src/b.rs", 100 * DAY_MS).unwrap();
        let r1 = search_with_conn(&idx.conn, Some("p"), "alpha", 10).unwrap();
        let r2 = search_with_conn(&idx.conn, Some("p"), "alpha", 10).unwrap();
        assert_eq!(r1.len(), r2.len());
        for (x, y) in r1.iter().zip(&r2) {
            assert_eq!((&x.path, x.score), (&y.path, y.score), "search not byte-stable across reads");
        }
    }

    /// Perf-pair sub-goal A: the bounded per-candidate probe must be BIT-IDENTICAL
    /// to the historical full-files-table scan it replaced — same boosts, same
    /// comparator, same output, including the degenerate cases (row missing from
    /// `files`, project with no rows, cross-project per-project ref_ms).
    #[test]
    fn temporal_boost_bounded_probe_matches_full_scan() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p1", "src/a.rs", "pub fn alpha() {}", "h1", 8).unwrap();
        idx.index_file("p1", "src/b.rs", "pub fn beta() {}", "h2", 8).unwrap();
        idx.index_file("p1", "src/c.rs", "pub fn gamma() {}", "h3", 8).unwrap();
        idx.index_file("p2", "src/a.rs", "pub fn alpha() {}", "h4", 8).unwrap();
        idx.record_access("p1", "src/a.rs", 100 * DAY_MS).unwrap();
        idx.record_access("p1", "src/b.rs", 60 * DAY_MS).unwrap();
        idx.record_access("p2", "src/a.rs", 3 * DAY_MS).unwrap();
        // src/c.rs stays UNSTAMPED; p3\0ghost.rs has NO files row at all.
        let fused_in: Vec<(String, f64)> = vec![
            ("p1\u{0}src/a.rs".into(), 0.030),
            ("p1\u{0}src/b.rs".into(), 0.030), // equal score → the boost decides
            ("p1\u{0}src/c.rs".into(), 0.028),
            ("p2\u{0}src/a.rs".into(), 0.027),
            ("p3\u{0}ghost.rs".into(), 0.026),
        ];

        // REFERENCE: the historical full-scan algorithm, replicated verbatim.
        let mut expect = fused_in.clone();
        {
            let mut access: std::collections::HashMap<String, (i64, i64)> =
                std::collections::HashMap::new();
            let mut proj_ref: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            let mut stmt = idx
                .conn
                .prepare("SELECT project_id, path, accessed_at_ms, accessed_count FROM files")
                .unwrap();
            let it = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .unwrap();
            for row in it {
                let (proj, p, at, count) = row.unwrap();
                let e = proj_ref.entry(proj.clone()).or_insert(0);
                *e = (*e).max(at);
                access.insert(format!("{proj}\u{0}{p}"), (at, count));
            }
            for (id, score) in expect.iter_mut() {
                let (at, count) = access.get(id).copied().unwrap_or((0, 0));
                let ref_ms = id
                    .split_once('\u{0}')
                    .and_then(|(p, _)| proj_ref.get(p))
                    .copied()
                    .unwrap_or(0);
                *score *= temporal_boost(at, count, ref_ms);
            }
            expect.sort_by(|a, b| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0))
            });
        }

        let mut got = fused_in.clone();
        apply_temporal_boost(&idx.conn, &mut got).unwrap();
        let bits = |v: &[(String, f64)]| -> Vec<(String, u64)> {
            v.iter().map(|(i, s)| (i.clone(), s.to_bits())).collect()
        };
        assert_eq!(bits(&got), bits(&expect), "bounded probe must be bit-identical to the full scan");
        // Sanity: the boost actually discriminated — a.rs (age 0 vs ref 100d) beats
        // the equal-scored b.rs (age 40d) despite b's input tie.
        let pos = |p: &str| got.iter().position(|(i, _)| i == p).unwrap();
        assert!(pos("p1\u{0}src/a.rs") < pos("p1\u{0}src/b.rs"), "fresher equal-score doc first: {got:?}");
    }

    #[test]
    fn serveable_bases_inverts_exts_expansion() {
        // The delta relink's dst-side correctness hinges on this inverse: for
        // EVERY base and EVERY suffix, the candidate `base + ext` must serve
        // `base` — else an appearing file could satisfy an import the relink
        // never revisits.
        for base in ["x", "a/b", "pkg/util"] {
            for e in EXTS {
                let cand = format!("{base}{e}");
                assert!(
                    serveable_bases(&cand).contains(&base.to_string()),
                    "candidate {cand} must serve base {base}"
                );
            }
        }
        // And the bare identity always holds.
        assert!(serveable_bases("weird.name.zz").contains(&"weird.name.zz".to_string()));
        // Rust half of the inverse (defect `rust-imports-no-ast-dependents`):
        // every RUST_EXTS candidate of a base must serve that base.
        for base in ["x", "src/gist", "src-tauri/src"] {
            for e in RUST_EXTS {
                let cand = format!("{base}{e}");
                assert!(
                    serveable_bases(&cand).contains(&base.to_string()),
                    "rust candidate {cand} must serve base {base}"
                );
            }
        }
        // A .rs file must NOT serve TS-style bases (bare path / index fallback).
        assert!(!serveable_bases("src/gist.rs").contains(&"src/gist.rs".to_string()));
    }

    /// Perf-pair sub-goal B: the CONVERGENCE property. Over a scripted
    /// add / modify-imports / rename / delete sequence — including the dst-side
    /// classes (a new file becoming the target of an existing import, extension
    /// shadowing of an index-file target, un-shadowing on delete) — the
    /// delta-relinked edge set must be byte-identical to a from-scratch full
    /// rebuild on the same end state, at EVERY step.
    #[test]
    fn relink_delta_converges_with_full_rebuild() {
        let (_d1, p1) = temp_db();
        let (_d2, p2) = temp_db();
        let inc = SqliteIndex::open(&p1).expect("open inc");
        let full = SqliteIndex::open(&p2).expect("open full");

        fn edges(idx: &SqliteIndex) -> Vec<(String, String, String)> {
            let mut stmt = idx
                .conn
                .prepare("SELECT src_path,dst_path,kind FROM code_edges WHERE project_id='p' ORDER BY src_path,dst_path,kind")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .map(|x| x.unwrap())
                .collect()
        }
        // Apply one scripted step to BOTH stores: `puts` = (path, content) upserts,
        // `dels` = removals. inc gets the DELTA relink, full a FULL rebuild.
        let step = |label: &str, puts: &[(&str, &str)], dels: &[&str]| {
            let mut changed: Vec<String> = Vec::new();
            let mut removed: Vec<String> = Vec::new();
            for (path, content) in puts {
                // content doubles as the hash so a modify really reindexes.
                inc.index_file("p", path, content, content, content.len() as i64).unwrap();
                full.index_file("p", path, content, content, content.len() as i64).unwrap();
                changed.push(path.to_string());
            }
            for path in dels {
                inc.remove_file("p", path).unwrap();
                full.remove_file("p", path).unwrap();
                removed.push(path.to_string());
            }
            inc.relink_edges_delta("p", &changed, &removed).unwrap();
            full.rebuild_edges("p").unwrap();
            assert_eq!(edges(&inc), edges(&full), "step '{label}': delta relink diverged from full rebuild");
        };

        // z.ts is untouched by most later steps — its z→a edge must survive every
        // delta that cannot affect it (the negative control for over-deletion).
        step("add a+z (unresolved ./b)", &[
            ("a.ts", "import './b';\nexport const a = 1;"),
            ("z.ts", "import './a';\nexport const z = 1;"),
        ], &[]);
        step("new file becomes dst of EXISTING import", &[("b/index.ts", "export const b = 1;")], &[]);
        assert!(edges(&inc).contains(&("a.ts".into(), "b/index.ts".into(), "imports".into())), "dst-side class: {:?}", edges(&inc));
        step("extension shadowing: b.ts outranks b/index.ts", &[("b.ts", "export const b2 = 1;")], &[]);
        assert!(edges(&inc).contains(&("a.ts".into(), "b.ts".into(), "imports".into())), "shadow shift: {:?}", edges(&inc));
        step("add another importer of ./b", &[("w.ts", "import './b';\nexport const w = 1;")], &[]);
        step("modify a's imports (./b → ./c, unresolved)", &[("a.ts", "import './c';\nexport const a = 2;")], &[]);
        step("new .tsx satisfies a's pending import", &[("c.tsx", "export const c = 1;")], &[]);
        step("rename c.tsx → cc.tsx (dst removed)", &[("cc.tsx", "export const c = 1;")], &["c.tsx"]);
        step("delete b.ts (UN-shadow → b/index.ts fallback)", &[], &["b.ts"]);
        assert!(edges(&inc).contains(&("w.ts".into(), "b/index.ts".into(), "imports".into())), "un-shadow on delete: {:?}", edges(&inc));
        step("delete b/index.ts (w unresolved)", &[], &["b/index.ts"]);
        step("delete importer z.ts", &[], &["z.ts"]);
        // End state: the only edge left is nothing — a, w, cc unresolved/leafs.
        assert_eq!(edges(&inc), Vec::<(String, String, String)>::new(), "end state: {:?}", edges(&inc));
    }

    /// Regression (gauntlet defect `rust-imports-no-ast-dependents`): end-to-end
    /// on a Rust fixture crate, `use` imports become AST-confirmed file edges and
    /// `code_impact` returns real `ast_dependents` for a Rust symbol — the tier
    /// was EMPTY for the whole Rust surface before this fix. Runs every step
    /// through BOTH the delta relink and a full rebuild (convergence pinned for
    /// the RUST_EXTS shadowing classes), with negative controls for std/external
    /// crates.
    #[test]
    fn rust_use_edges_feed_impact_ast_dependents() {
        let (_d1, p1) = temp_db();
        let (_d2, p2) = temp_db();
        let inc = SqliteIndex::open(&p1).expect("open inc");
        let full = SqliteIndex::open(&p2).expect("open full");
        fn edges(idx: &SqliteIndex) -> Vec<(String, String)> {
            let mut stmt = idx
                .conn
                .prepare("SELECT src_path,dst_path FROM code_edges WHERE project_id='p' ORDER BY src_path,dst_path")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|x| x.unwrap())
                .collect()
        }
        let step = |label: &str, puts: &[(&str, &str)], dels: &[&str]| {
            let mut changed: Vec<String> = Vec::new();
            let mut removed: Vec<String> = Vec::new();
            for (path, content) in puts {
                inc.index_file("p", path, content, content, content.len() as i64).unwrap();
                full.index_file("p", path, content, content, content.len() as i64).unwrap();
                changed.push(path.to_string());
            }
            for path in dels {
                inc.remove_file("p", path).unwrap();
                full.remove_file("p", path).unwrap();
                removed.push(path.to_string());
            }
            inc.relink_edges_delta("p", &changed, &removed).unwrap();
            full.rebuild_edges("p").unwrap();
            assert_eq!(edges(&inc), edges(&full), "step '{label}': delta relink diverged from full rebuild");
        };

        step("rust crate", &[
            ("src/lib.rs", "pub mod gist;\npub mod commands;\npub struct TopItem;"),
            ("src/gist/mod.rs", "pub struct Gist;\npub fn build_gist() {}"),
            // item + {self, Item} group forms — both must edge to the module FILE.
            ("src/commands.rs", "use crate::gist::{self, Gist};\nuse crate::gist::build_gist;\npub fn go() {}"),
            // super:: item import from a sibling module file.
            ("src/gist/synth.rs", "use super::Gist;\npub fn synth() {}"),
            // examples/tests are separate crates that name the lib crate.
            ("examples/cli.rs", "use mylib::gist::build_gist;\nfn main() {}"),
            ("tests/it.rs", "use mylib::gist::Gist;\nfn t() {}"),
            // NEGATIVE CONTROL: std/external uses from in-src files → zero edges.
            ("src/noise.rs", "use std::collections::HashMap;\nuse serde::Deserialize;\nuse serde_json::json;\npub fn n() {}"),
        ], &[]);
        let e = edges(&inc);
        for (src, dst) in [
            ("src/commands.rs", "src/gist/mod.rs"),
            ("src/gist/synth.rs", "src/gist/mod.rs"),
            ("examples/cli.rs", "src/gist/mod.rs"),
            ("tests/it.rs", "src/gist/mod.rs"),
        ] {
            assert!(e.contains(&(src.into(), dst.into())), "missing {src}→{dst} in {e:?}");
        }
        assert!(
            !e.iter().any(|(s, _)| s == "src/noise.rs"),
            "std/external uses must not edge: {e:?}"
        );

        // `use crate::Item` (item defined in the crate root) → lib.rs via the
        // crate-keyword parent row (always local, hence safe to root-resolve).
        step("crate-root item import", &[("src/uses_top.rs", "use crate::TopItem;\npub fn u() {}")], &[]);
        assert!(edges(&inc).contains(&("src/uses_top.rs".into(), "src/lib.rs".into())), "{:?}", edges(&inc));

        // Shadowing: a sibling gist.rs outranks gist/mod.rs (.rs precedes /mod.rs
        // in RUST_EXTS), and deleting it un-shadows back — both through the delta.
        step("rs shadows mod.rs", &[("src/gist.rs", "pub fn shadow() {}")], &[]);
        assert!(edges(&inc).contains(&("src/commands.rs".into(), "src/gist.rs".into())), "shadow: {:?}", edges(&inc));
        step("un-shadow on delete", &[], &["src/gist.rs"]);
        assert!(edges(&inc).contains(&("src/commands.rs".into(), "src/gist/mod.rs".into())), "un-shadow: {:?}", edges(&inc));

        // THE defect's acceptance: impact of a Rust symbol has AST dependents.
        let i = code_impact_readonly(&p1, "p", "build_gist", 5, ImpactDirection::Upstream, 200, false)
            .unwrap();
        assert_eq!(i.defined_in, vec!["src/gist/mod.rs".to_string()]);
        for want in ["src/commands.rs", "examples/cli.rs", "src/gist/synth.rs"] {
            assert!(
                i.ast_dependents.contains(&want.to_string()),
                "missing AST dependent {want}: {:?}",
                i.ast_dependents
            );
        }
        // exclude_tests still filters the tier (tests/it.rs has a tests/ segment).
        let no_tests =
            code_impact_readonly(&p1, "p", "build_gist", 5, ImpactDirection::Upstream, 200, true)
                .unwrap();
        assert!(no_tests.ast_dependents.contains(&"src/commands.rs".to_string()));
        assert!(!no_tests.ast_dependents.contains(&"tests/it.rs".to_string()));
    }

    #[test]
    fn resolve_import_edges() {
        let files: std::collections::HashSet<String> = [
            "src/a.ts",
            "src/util/index.ts", // a directory import resolves here via the EXTS fallback
            "shared.ts",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        // bare / package specifiers are not relative → no edge.
        assert_eq!(resolve_import("src/b.ts", "react", &files), None);
        // relative file import resolves with an extension appended.
        assert_eq!(resolve_import("src/b.ts", "./a", &files), Some("src/a.ts".into()));
        // directory import resolves to <dir>/index.ts.
        assert_eq!(resolve_import("src/b.ts", "./util", &files), Some("src/util/index.ts".into()));
        // a spec that ESCAPES the project root (more `..` than depth) is NOT an edge,
        // even if a same-named file exists at the root (the false-edge guard).
        assert_eq!(resolve_import("src/b.ts", "../../shared", &files), None);
        // one `..` from src/ lands at the root → resolves to the root file.
        assert_eq!(resolve_import("src/b.ts", "../shared", &files), Some("shared.ts".into()));
        // ?query / #hash suffixes are stripped before resolving.
        assert_eq!(resolve_import("src/b.ts", "./a?raw", &files), Some("src/a.ts".into()));
    }

    /// Regression (gauntlet defect `rust-imports-no-ast-dependents`): Rust use
    /// paths resolve to defining files via the lexical module-tree mapping.
    #[test]
    fn rust_resolve_import_paths() {
        let files: std::collections::HashSet<String> = [
            "src-tauri/src/lib.rs",
            "src-tauri/src/modules/brain/gist/mod.rs",
            "src-tauri/src/modules/brain/worker.rs",
            "src-tauri/src/json.rs", // collision bait for the external-crate guard
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let cmd = "src-tauri/src/modules/brain/commands.rs";
        // crate:: → nearest `src` ancestor; module dir falls back to mod.rs.
        assert_eq!(
            resolve_import(cmd, "crate::modules::brain::gist", &files),
            Some("src-tauri/src/modules/brain/gist/mod.rs".into())
        );
        assert_eq!(
            resolve_import(cmd, "crate::modules::brain::worker", &files),
            Some("src-tauri/src/modules/brain/worker.rs".into())
        );
        // super:: from a mod.rs pops to the parent module's dir.
        assert_eq!(
            resolve_import("src-tauri/src/modules/brain/gist/mod.rs", "super::worker", &files),
            Some("src-tauri/src/modules/brain/worker.rs".into())
        );
        // `crate` alone (the parent row of `use crate::Item`) → the crate root file.
        assert_eq!(resolve_import(cmd, "crate", &files), Some("src-tauri/src/lib.rs".into()));
        // examples/tests name the lib crate → sibling src tree.
        assert_eq!(
            resolve_import("src-tauri/examples/cli.rs", "koden_lib::modules::brain::gist", &files),
            Some("src-tauri/src/modules/brain/gist/mod.rs".into())
        );
        assert_eq!(
            resolve_import("src-tauri/tests/it.rs", "koden_lib::modules::brain::gist", &files),
            Some("src-tauri/src/modules/brain/gist/mod.rs".into())
        );
        // NEGATIVE CONTROLS: std and external crate names never edge from in-src
        // files — even when a same-named local module exists (src/json.rs bait).
        assert_eq!(resolve_import(cmd, "std::collections::HashMap", &files), None);
        assert_eq!(resolve_import(cmd, "serde_json::json", &files), None);
        assert_eq!(resolve_import(cmd, "serde", &files), None);
        // Bare external crate from a test crate: no root-file edge either.
        assert_eq!(resolve_import("src-tauri/tests/it.rs", "serde", &files), None);
    }

    #[test]
    fn normalize_rel_rejects_root_escape() {
        use std::path::Path;
        assert_eq!(normalize_rel(Path::new("src/./util/../a.ts")).as_deref(), Some("src/a.ts"));
        assert_eq!(normalize_rel(Path::new("src/../../x")), None, "escapes above root → None");
    }

    #[test]
    fn search_with_weights_defaults_equal_search_with_conn() {
        // Guards the V2.2 refactor against default-weights DRIFT. Pinning the
        // production weights as INLINE LITERALS (independent of SearchWeights::default)
        // is the point: if anyone edits Default or the W_*/RRF_W_* consts, the
        // field-by-field assertion below trips — so production ordering (and the gist
        // byte-identity gate) can't change silently. (A `default() == default()`
        // comparison would be tautological and catch nothing.)
        let pinned = SearchWeights {
            identity_bm25: (3.0, 1.5, 0.0),
            content_bm25: (0.0, 0.0, 1.0),
            rrf_identity: 1.5,
            rrf_content: 1.0,
            coverage_w: 0.25,
            coverage_gate_ratio: 0.7,
        };
        let d = SearchWeights::default();
        assert_eq!(d.identity_bm25, pinned.identity_bm25, "W_IDENTITY drifted from the documented production value");
        assert_eq!(d.content_bm25, pinned.content_bm25, "W_CONTENT drifted");
        assert_eq!(d.rrf_identity, pinned.rrf_identity, "RRF_W_IDENTITY drifted");
        assert_eq!(d.rrf_content, pinned.rrf_content, "RRF_W_CONTENT drifted");
        assert_eq!(d.coverage_w, pinned.coverage_w, "COVERAGE_W drifted");
        assert_eq!(d.coverage_gate_ratio, pinned.coverage_gate_ratio, "COVERAGE_GATE_RATIO drifted");

        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "src/auth/login.rs", "pub fn loginHandler() {}", "a", 24).unwrap();
        idx.index_file("p", "src/db/pool.rs", "pub fn buildConnectionPool() {}", "b", 30).unwrap();
        idx.index_file("p", "src/api/router.rs", "// login route registered here", "c", 30).unwrap();
        // search_with_conn must thread the PINNED production weights (not whatever
        // Default happens to be) — so compare against the literal struct.
        for q in ["login handler", "connection pool", "login"] {
            let a = search_with_conn(&idx.conn, Some("p"), q, 10).unwrap();
            let b = search_with_weights(&idx.conn, Some("p"), q, 10, &pinned).unwrap();
            assert_eq!(a.len(), b.len(), "q={q}");
            for (x, y) in a.iter().zip(&b) {
                assert_eq!((&x.path, x.score), (&y.path, y.score), "q={q}: divergent hit");
            }
        }
    }

    // ---- V3 coverage re-rank: pure-fn units + end-to-end behavior ----

    fn hs(ids: &[&str]) -> std::collections::HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    /// Coverage = COUNT of matched DISTINCT tokens per candidate (pure fn).
    #[test]
    fn coverage_counts_distinct_tokens_per_candidate() {
        // 3 distinct tokens; doc "full" matches all, "half" one, "none" zero.
        let token_hits = vec![hs(&["full", "half"]), hs(&["full"]), hs(&["full"])];
        let m = coverage_counts(
            &token_hits,
            ["full", "half", "none"].iter().map(|s| s.to_string()),
        );
        assert_eq!(m["full"], 3);
        assert_eq!(m["half"], 1);
        assert_eq!(m["none"], 0);
        // Degenerate: zero probed tokens → count 0 (apply_coverage no-ops on n=0).
        let m0 = coverage_counts(&[], ["x".to_string()].into_iter());
        assert_eq!(m0["x"], 0);
    }

    /// ADR-010 cluster 7 carried into coverage: repeated query words (and camel
    /// parts recurring across words) are DEDUPED by `query_tokens` BEFORE probing,
    /// so repetition can neither deflate the denominator nor double-count a match.
    #[test]
    fn coverage_denominator_uses_deduped_distinct_tokens() {
        assert_eq!(
            query_tokens("sendEmail sendEmail email"),
            query_tokens("sendEmail email"),
            "repeated words must not change the probed token set"
        );
        // A doc matching the single distinct concept covers 1/1, not 1/3.
        let toks = query_tokens("login login login");
        assert_eq!(toks, vec!["login"]);
        let m = coverage_counts(&[hs(&["doc"])], ["doc".to_string()].into_iter());
        assert_eq!(m["doc"], 1, "one DISTINCT token → denominator 1 → full coverage");
    }

    /// FULL-coverage best (focused query): the relative gate prunes below
    /// ratio·best — no rescue — and blend multiplies kept scores by 1 + w·cov.
    #[test]
    fn apply_coverage_gates_relative_when_best_is_full() {
        let matched: std::collections::HashMap<String, usize> =
            [("full".to_string(), 4), ("near".to_string(), 3), ("stray".to_string(), 1)].into();
        let mut fused =
            vec![("full".to_string(), 1.0), ("near".to_string(), 0.9), ("stray".to_string(), 0.8)];
        apply_coverage(&mut fused, &matched, 4, 0.25, 0.7);
        // best=4 (FULL of n=4) → no rescue; threshold 2.8 → "stray"(1) dropped.
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["full", "near"]);
        assert!((fused[0].1 - 1.0 * (1.0 + 0.25 * 1.0)).abs() < 1e-12);
        assert!((fused[1].1 - 0.9 * (1.0 + 0.25 * 0.75)).abs() < 1e-12);
        // ratio 0 disables the gate AND w 0 disables the blend (calibration seam).
        let mut fused2 = vec![("stray".to_string(), 0.8)];
        apply_coverage(&mut fused2, &matched, 4, 0.0, 0.0);
        assert_eq!(fused2.len(), 1);
        assert_eq!(fused2[0].1, 0.8, "w=0, ratio=0 → identity");
    }

    /// PARTIAL best (concept-bag query, e.g. the synthesized gist intent): the
    /// ≥COVERAGE_RESCUE_MIN rescue keeps multi-token partial matches that the
    /// relative gate alone would prune; single-stray-token hits still drop.
    #[test]
    fn apply_coverage_rescues_multi_token_hits_on_partial_best() {
        // n=4, best=3 (< n → rescue active): "code"(2) is below 0.7·3=2.1 but ≥2.
        let matched: std::collections::HashMap<String, usize> =
            [("note".to_string(), 3), ("code".to_string(), 2), ("stray".to_string(), 1)].into();
        let mut fused =
            vec![("note".to_string(), 1.0), ("code".to_string(), 0.9), ("stray".to_string(), 0.8)];
        apply_coverage(&mut fused, &matched, 4, 0.25, 0.7);
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["note", "code"], "2-token hit rescued, stray token still gated");
    }

    /// `matched == 0` means UNKNOWN coverage (the candidate's matches lie beyond
    /// COVERAGE_MAX_PROBE_TOKENS — a genuinely zero-coverage candidate cannot
    /// exist), so the gate must keep it and the blend must be neutral.
    #[test]
    fn apply_coverage_keeps_unprobed_candidates() {
        // "unprobed" is absent from the matched map entirely (probes never saw it).
        let matched: std::collections::HashMap<String, usize> =
            [("full".to_string(), 4), ("stray".to_string(), 1)].into();
        let mut fused = vec![
            ("full".to_string(), 1.0),
            ("stray".to_string(), 0.9),
            ("unprobed".to_string(), 0.8),
        ];
        apply_coverage(&mut fused, &matched, 4, 0.25, 0.7);
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["full", "unprobed"],
            "unknown-coverage hit kept, stray-token hit still gated"
        );
        let unprobed_score = fused.iter().find(|(id, _)| id == "unprobed").unwrap().1;
        assert_eq!(unprobed_score, 0.8, "unknown coverage must be blend-NEUTRAL");
    }

    /// END-TO-END regression for the COVERAGE_MAX_PROBE_TOKENS hard-drop: on a
    /// query with MORE distinct tokens than the probe cap (real consumer: long
    /// cold-start gist synth intents), probed counts are UNDERCOUNTS and the
    /// gate is skipped, so BOTH a file matching ONLY tail tokens (probed m=0)
    /// AND a file matching a few probed + tail tokens (small nonzero m — the
    /// case an m==0-only exemption non-monotonically hard-dropped: matching
    /// strictly MORE of the query removed it) must still be returned.
    #[test]
    fn coverage_cap_does_not_drop_tail_token_matchers() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        // 24 filler concepts (fill the probe window) + 2 tail concepts.
        let fillers = [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
            "india", "juliett", "kilo", "lima", "mike", "november", "oscar", "papa",
            "quebec", "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray",
        ];
        idx.index_file("p", "src/hub.ts", &format!("// {}", fillers.join(" ")), "a", 60).unwrap();
        idx.index_file("p", "src/zebra.ts", "export function zebraQuokka() {}", "b", 30).unwrap();
        // Mixed candidate: 2 probed tokens + the tail tokens — probed m is a
        // small NONZERO undercount, so the m==0 exemption alone never rescued it.
        idx.index_file(
            "p",
            "src/mixed.ts",
            "// alpha bravo\nexport function zebraQuokka() {}",
            "c",
            20,
        )
        .unwrap();
        let query = format!("{} zebra quokka", fillers.join(" "));
        assert!(
            query_tokens(&query).len() > COVERAGE_MAX_PROBE_TOKENS,
            "fixture must exceed the probe cap or the test proves nothing"
        );
        let hits = search_with_conn(&idx.conn, Some("p"), &query, 10).unwrap();
        let paths: Vec<&str> = hits.iter().map(|h| h.path.as_str()).collect();
        assert!(
            paths.contains(&"src/zebra.ts"),
            "tail-token matcher hard-dropped by the probe cap: {paths:?}"
        );
        assert!(
            paths.contains(&"src/mixed.ts"),
            "probed+tail matcher hard-dropped (non-monotonic gate): {paths:?}"
        );
        assert!(paths.contains(&"src/hub.ts"), "probed full-coverage hit missing: {paths:?}");
    }

    /// Single-token queries have uniform coverage by construction → gate must be
    /// a no-op (guards the `q_tokens.len() >= 2` skip in search_with_weights).
    #[test]
    fn coverage_single_token_query_is_a_noop() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "src/login.rs", "fn handler() {}", "a", 10).unwrap();
        idx.index_file("p", "src/other.rs", "// login flow happens here", "b", 10).unwrap();
        let hits = search_with_conn(&idx.conn, Some("p"), "login", 10).unwrap();
        assert_eq!(hits.len(), 2, "single-token query must not gate anything: {hits:?}");
    }

    /// End-to-end: a multi-token camel query keeps the full-coverage doc and
    /// prunes stray single-token matchers (the measured camel-class P@10 fix).
    #[test]
    fn coverage_gate_prunes_stray_token_hits() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "src/auth/reset.ts", "export function sendPasswordResetEmail() {}", "a", 40).unwrap();
        idx.index_file("p", "src/notify/sms.ts", "export function sendSms() {}", "b", 30).unwrap();
        idx.index_file("p", "tests/fixtures.ts", "// sample password words for the fuzzer", "c", 30).unwrap();
        let hits = search_with_conn(&idx.conn, Some("p"), "sendPasswordResetEmail", 10).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
            vec!["src/auth/reset.ts"],
            "stray 'send'/'password' matchers must be gated out"
        );
        // The UNGATED seam still sees the collisions (anti-vanity/calibration path).
        let ungated = SearchWeights { coverage_w: 0.0, coverage_gate_ratio: 0.0, ..SearchWeights::default() };
        let raw = idx.search_weighted(Some("p"), "sendPasswordResetEmail", 10, &ungated).unwrap();
        assert_eq!(raw.len(), 3, "ungated search must still retrieve the colliders: {raw:?}");
    }

    /// Determinism: coverage probes read the same snapshot as the legs — two
    /// identical searches must return identical (path, score) lists.
    #[test]
    fn coverage_rerank_is_byte_stable_across_reads() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "src/a.rs", "pub fn alphaBeta() {}", "h1", 20).unwrap();
        idx.index_file("p", "src/b.rs", "pub fn alphaOnly() {}", "h2", 24).unwrap();
        let r1 = search_with_conn(&idx.conn, Some("p"), "alpha beta", 10).unwrap();
        let r2 = search_with_conn(&idx.conn, Some("p"), "alpha beta", 10).unwrap();
        assert!(!r1.is_empty());
        assert_eq!(r1.len(), r2.len());
        for (x, y) in r1.iter().zip(&r2) {
            assert_eq!((&x.path, x.score), (&y.path, y.score), "coverage re-rank not byte-stable");
        }
    }

    #[test]
    fn unchanged_hash_is_skipped() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        assert!(idx.index_file("p", "a.rs", "x", "h", 1).unwrap());
        assert!(!idx.index_file("p", "a.rs", "x", "h", 1).unwrap(), "same hash skips");
        assert_eq!(idx.file_count("p").unwrap(), 1);
    }

    #[test]
    fn reindex_replaces_document() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "a.rs", "alpha", "h1", 5).unwrap();
        idx.index_file("p", "a.rs", "bravo", "h2", 5).unwrap();
        assert_eq!(idx.file_count("p").unwrap(), 1, "still one file row");
        // old term gone, new term present
        assert!(search_with_conn(&idx.conn, Some("p"), "alpha", 10).unwrap().is_empty());
        assert!(!search_with_conn(&idx.conn, Some("p"), "bravo", 10).unwrap().is_empty());
    }

    #[test]
    fn symbols_column_populated_from_ast() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "src/auth.rs", "pub fn loginHandler() {}", "h", 24).unwrap();
        let sym: String = idx
            .conn
            .query_row(
                "SELECT symbols FROM code_fts JOIN files f ON f.fts_rowid=code_fts.rowid \
                 WHERE f.project_id='p'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // tree-sitter extracted `loginHandler`; the tokenizer split it.
        assert!(sym.contains("login") && sym.contains("handler"), "symbols col: {sym}");
        // a non-AST file leaves symbols empty
        idx.index_file("p", "notes.txt", "loginHandler mention", "h2", 20).unwrap();
        let sym2: String = idx
            .conn
            .query_row(
                "SELECT symbols FROM code_fts JOIN files f ON f.fts_rowid=code_fts.rowid \
                 WHERE f.path='notes.txt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sym2, "", "non-AST file has empty symbols");
    }

    #[test]
    fn empty_query_returns_empty() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        assert!(search_with_conn(&idx.conn, None, "   ", 10).unwrap().is_empty());
    }

    #[test]
    fn remove_file_prunes_index() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "a.rs", "alpha", "h1", 5).unwrap();
        idx.index_file("p", "b.rs", "bravo", "h2", 5).unwrap();
        assert_eq!(idx.existing_paths("p").unwrap().len(), 2);
        assert!(idx.remove_file("p", "a.rs").unwrap());
        assert_eq!(idx.file_count("p").unwrap(), 1);
        assert!(
            search_with_conn(&idx.conn, Some("p"), "alpha", 10).unwrap().is_empty(),
            "removed file must stop matching"
        );
        assert!(!search_with_conn(&idx.conn, Some("p"), "bravo", 10).unwrap().is_empty());
        assert!(!idx.remove_file("p", "a.rs").unwrap(), "removing twice is a no-op");
    }

    /// HARD GATE proof from a real index run: the worker pipeline is
    /// `secrets::redact()` → `index_file()`. Plant secrets, run that exact
    /// sequence, then prove via the real search path that no secret material is
    /// retrievable, while surrounding code + a git-SHA control remain searchable.
    #[test]
    fn planted_secrets_never_reach_the_index() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        let src = concat!(
            "const apiKey = \"sk-ABCD1234efgh5678IJKL9012mnop\";\n",
            "let password = \"Tr0ub4dor!3Kx9Lm2Qp\";\n",
            "// commit a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0\n",
            "pub fn validateUserCredentials(input: &str) -> bool { true }\n",
        );
        let (redacted, n) = crate::modules::brain::secrets::redact(src);
        assert!(n >= 2, "expected >=2 redactions, got {n}: {redacted}");
        idx.index_file("p", "src/config.ts", &redacted, "h", redacted.len() as i64)
            .unwrap();

        // No secret material survives in the index (real-run proof via search).
        for leaked in ["sk", "troubdor", "3kx9lm2qp", "abcd1234efgh5678ijkl9012mnop"] {
            assert!(
                search_with_conn(&idx.conn, Some("p"), leaked, 10).unwrap().is_empty(),
                "secret token '{leaked}' is retrievable from the index"
            );
        }
        // Redaction is surgical: surrounding identifiers + the git-SHA control stay.
        assert!(
            !search_with_conn(&idx.conn, Some("p"), "validate credentials", 10).unwrap().is_empty(),
            "non-secret identifiers must remain searchable"
        );
        assert!(
            !search_with_conn(
                &idx.conn,
                Some("p"),
                "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0",
                10
            )
            .unwrap()
            .is_empty(),
            "git-SHA must-not-redact control must remain searchable"
        );
    }

    // ---- code_impact fixtures + tests (depth-annotated directed BFS) ----

    fn row(p: &str, d: usize) -> ImpactRow {
        ImpactRow { path: p.to_string(), depth: d }
    }

    /// Impact fixture (project "p"): `targetSym` defined in src/dep.ts.
    /// Upstream (importers, transitively): a→dep, b→a, c→b, x→{a,c} — x is
    /// MULTI-PATH (via a = 2 hops, via c = 4 hops → minimal depth 2).
    /// Downstream (what dep imports): helper (1) → leaf (2).
    fn impact_fixture(idx: &SqliteIndex) {
        let put = |p: &str, c: &str| {
            idx.index_file("p", p, c, p, c.len() as i64).unwrap();
        };
        put("src/dep.ts", "import './helper';\nexport function targetSym() {}");
        put("src/helper.ts", "import './leaf';\nexport function helperThing() {}");
        put("src/leaf.ts", "export function leafThing() {}");
        put("src/a.ts", "import './dep';\nexport const a = 1;");
        put("src/b.ts", "import './a';\nexport const b = 1;");
        put("src/c.ts", "import './b';\nexport const c = 1;");
        put("src/x.ts", "import './a';\nimport './c';\nexport const x = 1;");
        idx.rebuild_edges("p").unwrap();
    }

    #[test]
    fn impact_upstream_depth_is_minimal_and_deterministic() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        impact_fixture(&idx);
        let go = || {
            code_impact_readonly(&path, "p", "targetSym", 10, ImpactDirection::Upstream, 200, false)
                .unwrap()
        };
        let i = go();
        assert_eq!(i.direction, "upstream");
        assert_eq!(i.defined_in, vec!["src/dep.ts"]);
        // Minimal depths: x is reachable at 2 (via a) AND 4 (via c) — must be 2.
        // Full order = (depth asc, path asc): the depth-2 tie is path-sorted.
        let expect =
            vec![row("src/a.ts", 1), row("src/b.ts", 2), row("src/x.ts", 2), row("src/c.ts", 3)];
        assert_eq!(i.rows, expect, "minimal BFS depths in (depth, path) order");
        assert_eq!(
            i.ast_dependents,
            vec!["src/a.ts", "src/b.ts", "src/x.ts", "src/c.ts"],
            "flat wire-compat list mirrors rows"
        );
        assert!(!i.truncated);
        assert_eq!(i.result_total, 4);
        assert_eq!(i.truncated_reason, None);
        // Deterministic across runs (multi-path ties resolve identically).
        let j = go();
        assert_eq!(i.rows, j.rows);
        assert_eq!(i.lexical_candidates, j.lexical_candidates);
    }

    #[test]
    fn impact_downstream_and_both_directions() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        impact_fixture(&idx);
        let down = code_impact_readonly(
            &path, "p", "targetSym", 10, ImpactDirection::Downstream, 200, false,
        )
        .unwrap();
        assert_eq!(down.direction, "downstream");
        assert_eq!(
            down.rows,
            vec![row("src/helper.ts", 1), row("src/leaf.ts", 2)],
            "downstream = what the defining file imports, depth-annotated"
        );
        let both =
            code_impact_readonly(&path, "p", "targetSym", 10, ImpactDirection::Both, 200, false)
                .unwrap();
        assert_eq!(both.direction, "both");
        assert_eq!(
            both.rows,
            vec![
                row("src/a.ts", 1),
                row("src/helper.ts", 1),
                row("src/b.ts", 2),
                row("src/leaf.ts", 2),
                row("src/x.ts", 2),
                row("src/c.ts", 3),
            ],
            "deterministic merge of both legs in (depth, path) order"
        );
    }

    #[test]
    fn impact_cycle_safe_and_both_merges_min_depth() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        let put = |p: &str, c: &str| {
            idx.index_file("p", p, c, p, c.len() as i64).unwrap();
        };
        // Import CYCLE: cyc1 → cyc2 → cyc3 → cyc1; symbol defined in cyc1.
        put("src/cyc1.ts", "import './cyc2';\nexport function cycSym() {}");
        put("src/cyc2.ts", "import './cyc3';\nexport const c2 = 1;");
        put("src/cyc3.ts", "import './cyc1';\nexport const c3 = 1;");
        idx.rebuild_edges("p").unwrap();
        // Without the visited set this BFS never terminates — depth 20 bounds it
        // anyway, but the assertions prove the walk stops at the cycle closure.
        let up = code_impact_readonly(&path, "p", "cycSym", 20, ImpactDirection::Upstream, 200, false)
            .unwrap();
        assert_eq!(up.rows, vec![row("src/cyc3.ts", 1), row("src/cyc2.ts", 2)]);
        let down =
            code_impact_readonly(&path, "p", "cycSym", 20, ImpactDirection::Downstream, 200, false)
                .unwrap();
        assert_eq!(down.rows, vec![row("src/cyc2.ts", 1), row("src/cyc3.ts", 2)]);
        // `both` keeps the MIN depth per path across the two legs.
        let both = code_impact_readonly(&path, "p", "cycSym", 20, ImpactDirection::Both, 200, false)
            .unwrap();
        assert_eq!(both.rows, vec![row("src/cyc2.ts", 1), row("src/cyc3.ts", 1)]);
    }

    #[test]
    fn impact_truncation_fires_after_full_ordering() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        impact_fixture(&idx);
        // Full order: a@1, b@2, x@2, c@3. max_results=2 cuts INSIDE the depth-2
        // tie — the kept prefix must be the path-sorted head (b kept, x cut).
        let t = code_impact_readonly(&path, "p", "targetSym", 10, ImpactDirection::Upstream, 2, false)
            .unwrap();
        assert!(t.truncated);
        assert_eq!(t.result_total, 4, "pre-truncation count");
        assert_eq!(t.truncated_reason.as_deref(), Some("max_results"));
        assert_eq!(t.rows, vec![row("src/a.ts", 1), row("src/b.ts", 2)]);
        assert_eq!(t.ast_dependents, vec!["src/a.ts", "src/b.ts"]);
        // Negative control: an exact fit is NOT truncation.
        let n = code_impact_readonly(&path, "p", "targetSym", 10, ImpactDirection::Upstream, 4, false)
            .unwrap();
        assert!(!n.truncated);
        assert_eq!(n.truncated_reason, None);
        assert_eq!(n.result_total, 4);
        assert_eq!(n.rows.len(), 4);
        // max_results clamps to >= 1 (0 would silence the whole result).
        let one = code_impact_readonly(&path, "p", "targetSym", 10, ImpactDirection::Upstream, 0, false)
            .unwrap();
        assert_eq!(one.rows, vec![row("src/a.ts", 1)]);
        assert!(one.truncated);
    }

    #[test]
    fn is_test_path_matches_conventions_only() {
        for t in [
            "src/tests/a.ts",
            "pkg\\tests\\b.rs",
            "tests/root.ts",
            "src/foo.test.ts",
            "src/Foo.Spec.tsx",
            "src/foo_test.go",
            "src/test_thing.py",
            "foo.test.ts",
        ] {
            assert!(is_test_path(t), "{t} should match a test convention");
        }
        for n in [
            "src/latest.rs",
            "src/attestation.ts",
            "src/protest/file.ts",
            "src/testimony.ts",
            "src/foo.spec",       // no trailing extension → not *.spec.*
            "src/contest_tester.rs",
            "src/tests.rs",       // a FILE named tests.rs is not a tests/ segment
        ] {
            assert!(!is_test_path(n), "{n} must NOT match");
        }
    }

    #[test]
    fn impact_exclude_tests_filters_rows_and_lexical() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        impact_fixture(&idx);
        let put = |p: &str, c: &str| {
            idx.index_file("p", p, c, p, c.len() as i64).unwrap();
        };
        put("src/tests/it.ts", "import '../dep';\nexport const t = 1;");
        put("src/dep.test.ts", "import './dep';\nexport const t2 = 1;");
        put("src/mention.ts", "// targetSym mentioned here, no import edge");
        put("tests/lex.ts", "// targetSym mention from a test file");
        idx.rebuild_edges("p").unwrap();

        let all = code_impact_readonly(&path, "p", "targetSym", 10, ImpactDirection::Upstream, 200, false)
            .unwrap();
        assert!(all.rows.contains(&row("src/dep.test.ts", 1)), "{:?}", all.rows);
        assert!(all.rows.contains(&row("src/tests/it.ts", 1)), "{:?}", all.rows);
        assert!(all.lexical_candidates.contains(&"src/mention.ts".to_string()));
        assert!(all.lexical_candidates.contains(&"tests/lex.ts".to_string()));

        let f = code_impact_readonly(&path, "p", "targetSym", 10, ImpactDirection::Upstream, 200, true)
            .unwrap();
        assert_eq!(
            f.rows,
            vec![row("src/a.ts", 1), row("src/b.ts", 2), row("src/x.ts", 2), row("src/c.ts", 3)],
            "test-convention paths dropped from rows"
        );
        assert_eq!(f.result_total, 4, "result_total counts post-filter rows");
        assert!(f.lexical_candidates.contains(&"src/mention.ts".to_string()));
        assert!(
            !f.lexical_candidates.contains(&"tests/lex.ts".to_string()),
            "lexical tier is filtered too: {:?}",
            f.lexical_candidates
        );
    }

    /// Regression (gauntlet S2 `no-test-exclusion-in-gist-search`): the search
    /// path carries the same `exclude_tests` knob as code_impact. Test-convention
    /// rows are dropped BEFORE the limit cut, so a capped agent-facing consumer
    /// (the gist's MAX_FILES budget) gets a FULL budget of production hits even
    /// when tests lexically outrank them. Negative control: the default path is
    /// unchanged and still surfaces the test files.
    #[test]
    fn search_excluding_tests_drops_test_paths_before_the_limit_cut() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        // The test files match "redaction gate" in PATH + SYMBOLS + content — they
        // outrank the content-only production files (the observed S2 shape, where
        // tests/*.rs took 4 of the gist's top 5 slots).
        idx.index_file("p", "tests/redaction_gate.rs", "fn redaction_gate() { /* redaction gate */ }", "t1", 44).unwrap();
        idx.index_file("p", "src/gate.test.ts", "export const redactionGate = 1; // redaction gate", "t2", 49).unwrap();
        idx.index_file("p", "src/secrets.rs", "// the redaction gate lives here", "p1", 32).unwrap();
        idx.index_file("p", "src/scan.rs", "// feeds the redaction gate", "p2", 27).unwrap();

        // Negative control: knob OFF — the defect shape (a test file at rank 1,
        // tests inside the cut) is really present, and behavior is unchanged.
        let all = search_with_conn(&idx.conn, Some("p"), "redaction gate", 2).unwrap();
        assert!(is_test_path(&all[0].path), "control: a test file should outrank: {all:?}");

        // Knob ON with the SAME limit: zero test paths AND a still-full budget —
        // both production files, proving the filter runs before the cut (a
        // post-cut filter would return fewer than `limit` hits here).
        let prod = search_excluding_tests_with_conn(&idx.conn, Some("p"), "redaction gate", 2).unwrap();
        assert_eq!(prod.len(), 2, "budget must stay full: {prod:?}");
        assert!(prod.iter().all(|h| !is_test_path(&h.path)), "test path leaked: {prod:?}");
        let mut got: Vec<&str> = prod.iter().map(|h| h.path.as_str()).collect();
        got.sort();
        assert_eq!(got, vec!["src/scan.rs", "src/secrets.rs"]);

        // Deterministic (the gist byte-identity gate rides on this).
        let again = search_excluding_tests_with_conn(&idx.conn, Some("p"), "redaction gate", 2).unwrap();
        assert_eq!(
            prod.iter().map(|h| (&h.path, h.score)).collect::<Vec<_>>(),
            again.iter().map(|h| (&h.path, h.score)).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn impact_multi_definition_symbol_uses_nearest_def() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        let put = |p: &str, c: &str| {
            idx.index_file("p", p, c, p, c.len() as i64).unwrap();
        };
        put("src/d1.ts", "export function dupSym() {}");
        put("src/d2.ts", "export function dupSym() {}");
        put("src/far.ts", "import './d1';\nexport const f = 1;");
        put("src/near.ts", "import './d2';\nexport const n = 1;");
        // farther reaches d1 via far (2 hops) but imports d2 DIRECTLY — the
        // minimal depth from the NEAREST defining file (1) must win.
        put("src/farther.ts", "import './far';\nimport './d2';\nexport const ff = 1;");
        idx.rebuild_edges("p").unwrap();
        let i = code_impact_readonly(&path, "p", "dupSym", 10, ImpactDirection::Upstream, 200, false)
            .unwrap();
        assert_eq!(i.defined_in, vec!["src/d1.ts", "src/d2.ts"], "sorted defs");
        assert_eq!(
            i.rows,
            vec![row("src/far.ts", 1), row("src/farther.ts", 1), row("src/near.ts", 1)],
            "farther is depth 1 (direct d2 import), not 2 (via far→d1)"
        );
    }

    #[test]
    fn impact_depth_clamps_1_to_20() {
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        let put = |p: &str, c: &str| {
            idx.index_file("p", p, c, p, c.len() as i64).unwrap();
        };
        // A 22-file chain: f00 defines chainSym; f01 imports f00; … f21 imports f20.
        put("src/f00.ts", "export function chainSym() {}");
        for i in 1..=21usize {
            put(
                &format!("src/f{i:02}.ts"),
                &format!("import './f{:02}';\nexport const x{i} = 1;", i - 1),
            );
        }
        idx.rebuild_edges("p").unwrap();
        // depth 0 clamps to 1 → direct importer only (NOT empty — the floor
        // discriminates against "0 = disabled" regressions).
        let d0 = code_impact_readonly(&path, "p", "chainSym", 0, ImpactDirection::Upstream, 200, false)
            .unwrap();
        assert_eq!(d0.rows, vec![row("src/f01.ts", 1)]);
        // depth usize::MAX clamps to 20 → f21 (hop 21) is NOT reached.
        let dmax = code_impact_readonly(
            &path, "p", "chainSym", usize::MAX, ImpactDirection::Upstream, 200, false,
        )
        .unwrap();
        assert_eq!(dmax.rows.len(), 20);
        assert_eq!(dmax.rows.last(), Some(&row("src/f20.ts", 20)));
        assert!(
            !dmax.rows.iter().any(|r| r.path == "src/f21.ts"),
            "hop 21 must be cut by the depth ceiling"
        );
    }
}
