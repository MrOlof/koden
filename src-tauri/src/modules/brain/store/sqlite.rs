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
        let existing: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT hash, fts_rowid FROM files WHERE project_id=?1 AND path=?2",
                (project_id, rel_path),
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .ok();

        if let Some((old_hash, _)) = &existing {
            if old_hash == hash {
                return Ok(false); // unchanged — skip reindex
            }
        }

        let path_tokens = tokenize::tokenize(rel_path).join(" ");
        let content_tokens = tokenize::tokenize(content).join(" ");
        // P2: parse once → definitions (the `symbols` FTS column + `code_nodes`)
        // and raw import specs (`code_imports`). Only runs here on a new/changed
        // file — the unchanged-hash early return skips it. Edges are NOT touched
        // here; they're rebuilt as a pure function of imports+files (rebuild_edges).
        let analysis = analyze_for(rel_path, content);
        let symbol_tokens = analysis
            .as_ref()
            .map(|a| tokenize::tokenize(&a.symbol_names()).join(" "))
            .unwrap_or_default();

        let tx = self.conn.unchecked_transaction()?;
        if let Some((_, old_rowid)) = existing {
            tx.execute("DELETE FROM code_fts WHERE rowid=?1", [old_rowid])?;
        }
        tx.execute(
            "INSERT INTO code_fts(path,symbols,content) VALUES(?1,?2,?3)",
            (&path_tokens, &symbol_tokens, &content_tokens),
        )?;
        let rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO files(project_id,path,hash,size,fts_rowid) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(project_id,path) DO UPDATE SET
                hash=excluded.hash, size=excluded.size, fts_rowid=excluded.fts_rowid",
            (project_id, rel_path, hash, size, rowid),
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
        if let Some(a) = &analysis {
            for n in &a.nodes {
                tx.execute(
                    "INSERT OR IGNORE INTO code_nodes(project_id,path,name,kind,start_line,start_col) VALUES(?1,?2,?3,?4,?5,?6)",
                    rusqlite::params![project_id, rel_path, n.name, n.kind, n.start_line, n.start_col],
                )?;
            }
            for spec in &a.imports {
                tx.execute(
                    "INSERT OR IGNORE INTO code_imports(project_id,src_path,spec) VALUES(?1,?2,?3)",
                    (project_id, rel_path, spec.as_str()),
                )?;
            }
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
}

impl SearchIndex for SqliteIndex {
    fn search(&self, project: Option<&str>, query: &str, limit: usize) -> rusqlite::Result<Vec<Hit>> {
        search_with_conn(&self.conn, project, query, limit)
    }
}

/// One-parse AST analysis for a file, or `None` for non-AST languages.
fn analyze_for(rel_path: &str, content: &str) -> Option<ast::Analysis> {
    let ext = std::path::Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    ast::lang_for_ext(ext).map(|lang| ast::analyze(lang, content))
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
    const EXTS: &[&str] = &[
        "", ".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.tsx", "/index.js",
    ];
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

/// Run one BM25 leg, returning ranked `(project_id, path)` best-first.
fn run_leg(
    conn: &Connection,
    match_expr: &str,
    project: Option<&str>,
    w: (f64, f64, f64),
    limit: usize,
) -> rusqlite::Result<Vec<(String, String)>> {
    // Weights are fixed constants (no injection risk); inline them since FTS5
    // bm25() column-weight args are not reliably bindable.
    let bm25 = format!("bm25(code_fts, {:.4}, {:.4}, {:.4})", w.0, w.1, w.2);
    let mut rows: Vec<(String, String)> = Vec::new();
    match project {
        Some(pid) => {
            let sql = format!(
                "SELECT f.project_id, f.path FROM code_fts
                 JOIN files f ON f.fts_rowid = code_fts.rowid
                 WHERE code_fts MATCH ?1 AND f.project_id = ?2
                 ORDER BY {bm25}, f.path LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let it = stmt.query_map(
                rusqlite::params![match_expr, pid, limit as i64],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?;
            for x in it {
                rows.push(x?);
            }
        }
        None => {
            let sql = format!(
                "SELECT f.project_id, f.path FROM code_fts
                 JOIN files f ON f.fts_rowid = code_fts.rowid
                 WHERE code_fts MATCH ?1
                 ORDER BY {bm25}, f.path LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let it = stmt.query_map(
                rusqlite::params![match_expr, limit as i64],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?;
            for x in it {
                rows.push(x?);
            }
        }
    }
    Ok(rows)
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
}

impl Default for SearchWeights {
    fn default() -> Self {
        Self {
            identity_bm25: W_IDENTITY,
            content_bm25: W_CONTENT,
            rrf_identity: RRF_W_IDENTITY,
            rrf_content: RRF_W_CONTENT,
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
    let key = |p: &(String, String)| format!("{}\u{0}{}", p.0, p.1);
    let a_ids: Vec<String> = leg_a.iter().map(key).collect();
    let b_ids: Vec<String> = leg_b.iter().map(key).collect();
    let mut fused = rank::weighted_rrf(&[
        Leg { weight: w.rrf_identity, ranked: &a_ids },
        Leg { weight: w.rrf_content, ranked: &b_ids },
    ]);

    // V2 temporal re-rank ([DP-12]): a snapshot-stable multiplicative boost applied
    // AFTER fusion (RRF stays leg-pure — a per-doc multiplier is a document property,
    // not a leg). All inputs are STORED + read from this connection's snapshot, so
    // two reads of an unchanged index re-derive the same order → byte-identical gist.
    apply_temporal_boost(conn, project, &mut fused)?;

    let hits = fused
        .into_iter()
        .take(limit)
        .filter_map(|(id, score)| {
            id.split_once('\u{0}').map(|(proj, path)| Hit {
                project: proj.to_string(),
                path: path.to_string(),
                score,
            })
        })
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
    project: Option<&str>,
    fused: &mut [(String, f64)],
) -> rusqlite::Result<()> {
    if fused.is_empty() {
        return Ok(());
    }
    // Load (composite id → accessed_*) AND a PER-PROJECT ref_ms (max accessed_at_ms)
    // in one scan, so a doc's age is relative to its OWN project even on a cross-
    // project (project=None) search — indexing in project B never reorders project A.
    let mut access: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
    let mut proj_ref: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut scan = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<()> {
        let mut stmt = conn.prepare(sql)?;
        let it = stmt.query_map(params, |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
        })?;
        for row in it {
            let (proj, path, at, count) = row?;
            let e = proj_ref.entry(proj.clone()).or_insert(0);
            *e = (*e).max(at);
            access.insert(format!("{proj}\u{0}{path}"), (at, count));
        }
        Ok(())
    };
    match project {
        Some(pid) => scan(
            "SELECT project_id, path, accessed_at_ms, accessed_count FROM files WHERE project_id=?1",
            &[&pid],
        )?,
        None => scan("SELECT project_id, path, accessed_at_ms, accessed_count FROM files", &[])?,
    }

    for (id, score) in fused.iter_mut() {
        let (at, count) = access.get(id).copied().unwrap_or((0, 0));
        let ref_ms = id.split_once('\u{0}').and_then(|(p, _)| proj_ref.get(p)).copied().unwrap_or(0);
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

fn open_readonly(db_path: &Path) -> rusqlite::Result<Connection> {
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
/// the per-claim freshness-label inputs (ADR-011): `created`, the supersession
/// edges, and anchors. All note-file-derived state (note files are indexed), so
/// it is covered by the gist cache key's content fingerprint.
#[derive(Clone, Debug)]
pub struct GistNote {
    pub id: String,
    pub title: String,
    pub note_type: Option<String>,
    pub created: Option<String>,
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
        "SELECT id,title,note_type,created,supersedes,superseded_by,COALESCE(anchors,'[]')
         FROM notes WHERE project_id=?1 ORDER BY id",
    )?;
    let it = stmt.query_map([project_id], |r| {
        let anchors: Vec<String> =
            serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
        Ok(GistNote {
            id: r.get(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            note_type: r.get(2)?,
            created: r.get(3)?,
            supersedes: r.get(4)?,
            superseded_by: r.get(5)?,
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
        };
        let d = SearchWeights::default();
        assert_eq!(d.identity_bm25, pinned.identity_bm25, "W_IDENTITY drifted from the documented production value");
        assert_eq!(d.content_bm25, pinned.content_bm25, "W_CONTENT drifted");
        assert_eq!(d.rrf_identity, pinned.rrf_identity, "RRF_W_IDENTITY drifted");
        assert_eq!(d.rrf_content, pinned.rrf_content, "RRF_W_CONTENT drifted");

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
