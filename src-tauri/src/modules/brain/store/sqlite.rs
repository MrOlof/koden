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
use crate::modules::brain::ast::{self, Impact, SymbolInfo};
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
/// sweep over the labeled corpus + confusers shows `rrf_identity >= rrf_content` is
/// MRR-optimal (1.000 vs 0.875 when content dominates), at zero negative-control
/// leaks across the whole grid; 1.5 sits in that optimal band. Re-run the sweep
/// (`cargo test --test brain_bench -- --ignored`) and record before/after on any
/// change (§13.12). NB: RRF fuses by RANK, so the bm25 column MAGNITUDES above
/// (path 3×) only order WITHIN a leg — the leg RRF weights here are the load-bearing
/// cross-leg knob (the sweep confirms path_bm25 ∈ {2,3,4} doesn't move MRR).
const RRF_W_IDENTITY: f64 = 1.5;
const RRF_W_CONTENT: f64 = 1.0;

/// The labels of the RRF legs `search_with_conn` actually fuses, in order. The
/// single source of truth for the P5 no-vector-leg gate (`search::registered_search_legs`
/// returns this) — keep it in lockstep with the legs built in `search_with_conn`.
/// A future semantic `vector` leg is added here AND there together at enablement.
pub const SEARCH_LEG_LABELS: &[&str] = &["identity", "content"];

pub struct SqliteIndex {
    conn: Connection,
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

    /// Boot sweep: charge any reservation orphaned by a mid-call crash at its
    /// estimate, so a crashed reflect over-counts rather than leaking free spend (P4).
    pub fn sweep_orphaned_reservations(&self, now: i64) -> Result<usize, String> {
        crate::modules::brain::reflect::budget::sweep_orphaned_reservations(&self.conn, now)
    }

    /// Search with EXPLICIT ranking weights (the offline calibration seam, used by
    /// the relevance benchmark to sweep weights over the labeled corpus).
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

    /// Full note records for the doctor.
    pub fn list_note_records(&self, project_id: &str) -> rusqlite::Result<Vec<NoteRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, note_type, revalidate_after, supersedes, superseded_by, COALESCE(anchors,'[]')
             FROM notes WHERE project_id=?1",
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

/// Tiered impact of a symbol: the AST reverse-import closure (files that import,
/// transitively, the file(s) defining the symbol) plus the lexical
/// over-approximation (content mentions). CONCEPT §4.1b `code_impact`.
pub fn code_impact_readonly(
    db_path: &Path,
    project: &str,
    symbol: &str,
    depth: usize,
) -> rusqlite::Result<Impact> {
    let conn = open_readonly(db_path)?;
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

    // BFS the reverse-import closure (dst → src) from the defining files.
    let mut seen: std::collections::HashSet<String> = defined_in.iter().cloned().collect();
    let mut frontier: Vec<String> = defined_in.clone();
    {
        let mut stmt =
            conn.prepare("SELECT src_path FROM code_edges WHERE project_id=?1 AND dst_path=?2")?;
        for _ in 0..depth.max(1) {
            let mut next = Vec::new();
            for node in &frontier {
                let it = stmt.query_map((project, node.as_str()), |r| r.get::<_, String>(0))?;
                for x in it {
                    let s = x?;
                    if seen.insert(s.clone()) {
                        next.push(s);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
    }
    let def_set: std::collections::HashSet<&String> = defined_in.iter().collect();
    let mut ast_dependents: Vec<String> =
        seen.iter().filter(|p| !def_set.contains(*p)).cloned().collect();
    ast_dependents.sort();

    // Lexical over-approximation: content mentions not already covered.
    let exclude: std::collections::HashSet<String> =
        defined_in.iter().chain(ast_dependents.iter()).cloned().collect();
    let mut lexical_candidates: Vec<String> = search_with_conn(&conn, Some(project), symbol, 50)?
        .into_iter()
        .map(|h| h.path)
        .filter(|p| !exclude.contains(p))
        .collect();
    lexical_candidates.sort();
    lexical_candidates.dedup();

    Ok(Impact {
        symbol: symbol.to_string(),
        defined_in,
        ast_dependents,
        lexical_candidates,
    })
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
    let q_tokens = tokenize::tokenize(query);
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
    let fused = rank::weighted_rrf(&[
        Leg { weight: w.rrf_identity, ranked: &a_ids },
        Leg { weight: w.rrf_content, ranked: &b_ids },
    ]);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("index.sqlite");
        (dir, path)
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
        // The parameterized core with default weights must be byte-identical to the
        // production search (guards the V2.2 refactor).
        let (_dir, path) = temp_db();
        let idx = SqliteIndex::open(&path).expect("open");
        idx.index_file("p", "src/auth/login.rs", "pub fn loginHandler() {}", "a", 24).unwrap();
        idx.index_file("p", "src/db/pool.rs", "pub fn buildConnectionPool() {}", "b", 30).unwrap();
        idx.index_file("p", "src/api/router.rs", "// login route registered here", "c", 30).unwrap();
        for q in ["login handler", "connection pool", "login"] {
            let a = search_with_conn(&idx.conn, Some("p"), q, 10).unwrap();
            let b = search_with_weights(&idx.conn, Some("p"), q, 10, &SearchWeights::default()).unwrap();
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
}
