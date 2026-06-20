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
use crate::modules::brain::rank::{self, Leg};
use crate::modules::brain::tokenize;
use crate::modules::brain::Hit;

/// Per-column bm25 weights for the "identity" leg (path ~3×, symbols ~1.5×).
const W_IDENTITY: (f64, f64, f64) = (3.0, 1.5, 0.0);
/// Per-column bm25 weights for the "content" leg.
const W_CONTENT: (f64, f64, f64) = (0.0, 0.0, 1.0);
/// RRF leg weights ([DP-9]). Identity (path+symbols) weighted above content so a
/// filename match outranks a body-only mention (CONCEPT [DP-2]); content still
/// contributes for recall. Provisional defaults — the benchmark suite (labeled
/// ground-truth + negative control) calibrates these against real fixtures.
const RRF_W_IDENTITY: f64 = 1.5;
const RRF_W_CONTENT: f64 = 1.0;

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

        let tx = self.conn.unchecked_transaction()?;
        if let Some((_, old_rowid)) = existing {
            tx.execute("DELETE FROM code_fts WHERE rowid=?1", [old_rowid])?;
        }
        tx.execute(
            "INSERT INTO code_fts(path,symbols,content) VALUES(?1,'',?2)",
            (&path_tokens, &content_tokens),
        )?;
        let rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO files(project_id,path,hash,size,fts_rowid) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(project_id,path) DO UPDATE SET
                hash=excluded.hash, size=excluded.size, fts_rowid=excluded.fts_rowid",
            (project_id, rel_path, hash, size, rowid),
        )?;
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
        tx.commit()?;
        Ok(true)
    }
}

impl SearchIndex for SqliteIndex {
    fn search(&self, project: Option<&str>, query: &str, limit: usize) -> rusqlite::Result<Vec<Hit>> {
        search_with_conn(&self.conn, project, query, limit)
    }
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
                 ORDER BY {bm25} LIMIT ?3"
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
                 ORDER BY {bm25} LIMIT ?2"
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

/// Core hybrid search over an arbitrary connection (writer reuse or r/o reader).
pub fn search_with_conn(
    conn: &Connection,
    project: Option<&str>,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    let q_tokens = tokenize::tokenize(query);
    if q_tokens.is_empty() {
        return Ok(Vec::new());
    }
    let overfetch = (limit * 4).max(40);
    let leg_a = run_leg(conn, &build_match("path symbols", &q_tokens), project, W_IDENTITY, overfetch)?;
    let leg_b = run_leg(conn, &build_match("content", &q_tokens), project, W_CONTENT, overfetch)?;

    // Composite id "project\0path" keeps paths unique across projects.
    let key = |p: &(String, String)| format!("{}\u{0}{}", p.0, p.1);
    let a_ids: Vec<String> = leg_a.iter().map(key).collect();
    let b_ids: Vec<String> = leg_b.iter().map(key).collect();
    let fused = rank::weighted_rrf(&[
        Leg { weight: RRF_W_IDENTITY, ranked: &a_ids },
        Leg { weight: RRF_W_CONTENT, ranked: &b_ids },
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

/// File count for a project via a read-only connection.
pub fn file_count_readonly(db_path: &Path, project_id: &str) -> rusqlite::Result<i64> {
    let conn = open_readonly(db_path)?;
    conn.query_row(
        "SELECT COUNT(*) FROM files WHERE project_id=?1",
        [project_id],
        |r| r.get(0),
    )
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
