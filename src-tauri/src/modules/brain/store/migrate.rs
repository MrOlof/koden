//! Versioned, idempotent migrations. All durable storage is versioned
//! (BUILD-PROMPT §13.4): corrupt *derived* data is safe to rebuild, the version
//! gate decides forward migrations. P0 establishes v1; later phases chain here.

use rusqlite::Connection;

use super::schema::{DDL, SCHEMA_VERSION};

/// Apply pragmas + base DDL and reconcile the stored `schema_version`.
/// Returns the version now in force.
pub fn migrate(conn: &Connection) -> rusqlite::Result<i64> {
    // WAL = single writer + many concurrent readers without blocking (CONCEPT §8).
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )?;

    // Read the stored version BEFORE (re)creating tables, so an upgrade can drop
    // derived tables whose schema changed. `brain_meta` must exist to read it.
    conn.execute_batch("CREATE TABLE IF NOT EXISTS brain_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
    let current: Option<i64> = conn
        .query_row(
            "SELECT value FROM brain_meta WHERE key='schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| s.parse().ok());

    if matches!(current, Some(v) if v == SCHEMA_VERSION) {
        // Already current: just ensure the (idempotent) DDL is present and return.
        conn.execute_batch(DDL)?;
        return Ok(SCHEMA_VERSION);
    }

    // Fresh or upgrade: do the whole migration in ONE transaction so "advanced to
    // vN" is a single atomic, durable fact — a crash mid-migration rolls back
    // cleanly and re-runs, never leaving a half-dropped schema or an out-of-step
    // version row. (PRAGMAs above must stay OUTSIDE the txn.)
    let tx = conn.unchecked_transaction()?;
    // On any upgrade, drop the DERIVED tables so the DDL recreates them at the
    // current schema and the next warm pass rebuilds them — backfills new columns
    // (incl. the AST-fed `symbols` and `notes.supersedes`). DERIVED = rebuildable
    // from disk: the code index (code_fts/code_nodes/code_imports/code_edges + the
    // `files` manifest) AND `notes` (re-scanned from the `.md` files by
    // `scan_project_memory` on the next warm pass). Truly CANONICAL data — proposals,
    // reject_signatures (human decisions), brain_budget(+ledger) (spend state),
    // brain_semantic_meta — is preserved PURELY by being absent from this DROP batch.
    // This is a drop-list, not a keep-list, so NEVER add a canonical table here.
    if matches!(current, Some(v) if v < SCHEMA_VERSION) {
        tx.execute_batch(
            "DROP TABLE IF EXISTS code_fts;
             DROP TABLE IF EXISTS code_nodes;
             DROP TABLE IF EXISTS code_imports;
             DROP TABLE IF EXISTS code_edges;
             DROP TABLE IF EXISTS notes;
             DELETE FROM files;",
        )?;
    }
    tx.execute_batch(DDL)?;
    tx.execute(
        "INSERT INTO brain_meta(key,value) VALUES('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    tx.commit()?;
    Ok(SCHEMA_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_is_idempotent_and_sets_version() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        // second run must not error and must keep the version.
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn upgrade_rebuilds_derived_file_tables() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        migrate(&conn).unwrap();
        // Simulate an older store with a stale derived row.
        conn.execute("UPDATE brain_meta SET value='1' WHERE key='schema_version'", [])
            .unwrap();
        conn.execute(
            "INSERT INTO files(project_id,path,hash,size,fts_rowid) VALUES('p','x.rs','h',1,1)",
            [],
        )
        .unwrap();
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        // The upgrade cleared the derived file manifest so a warm pass rebuilds it
        // (backfilling the AST-fed symbols column + any new columns).
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "upgrade clears derived file rows");
    }

    #[test]
    fn upgrade_preserves_budget_state() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        migrate(&conn).unwrap();
        // Simulate human-set spend state on an older store, then force an upgrade.
        conn.execute("UPDATE brain_budget SET ceiling_usd=1.0, spent_total_usd=0.42 WHERE id=1", [])
            .unwrap();
        conn.execute(
            "INSERT INTO brain_budget_ledger(status,est_cost_usd,actual_cost_usd,model,reserved_at,reconciled_at)
             VALUES('spent',0.002,0.002,'m',1,2)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE brain_meta SET value='5' WHERE key='schema_version'", [])
            .unwrap();
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        // Budget tables are CANONICAL: the upgrade must NOT drop them.
        let (ceiling, spent): (f64, f64) = conn
            .query_row("SELECT ceiling_usd, spent_total_usd FROM brain_budget WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(ceiling, 1.0, "ceiling survives upgrade");
        assert_eq!(spent, 0.42, "spent_total survives upgrade");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM brain_budget_ledger", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "ledger rows survive upgrade");
    }

    #[test]
    fn semantic_header_seeded_empty_and_preserved() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        migrate(&conn).unwrap();
        let (eid, dims): (String, i64) = conn
            .query_row("SELECT embedder_id, dims FROM brain_semantic_meta WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(eid, "", "v1 has no embedder");
        assert_eq!(dims, 0);
        // simulate an enablement-time write, then force an upgrade — must survive.
        conn.execute("UPDATE brain_semantic_meta SET embedder_id='bge-small', dims=384 WHERE id=1", [])
            .unwrap();
        conn.execute("UPDATE brain_meta SET value='6' WHERE key='schema_version'", []).unwrap();
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        let (eid2, dims2): (String, i64) = conn
            .query_row("SELECT embedder_id, dims FROM brain_semantic_meta WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((eid2.as_str(), dims2), ("bge-small", 384), "header survives upgrade");
        // re-open at the current version (the "already current" branch re-runs the
        // idempotent DDL incl. INSERT OR IGNORE) — must NOT clobber the set header.
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        let eid3: String = conn
            .query_row("SELECT embedder_id FROM brain_semantic_meta WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(eid3, "bge-small", "re-open does not clobber the header (INSERT OR IGNORE)");
    }

    #[test]
    fn fts5_is_available() {
        // Proves the bundled SQLite has FTS5 (else CREATE VIRTUAL TABLE errors).
        let conn = Connection::open_in_memory().expect("in-memory db");
        migrate(&conn).expect("migrate creates the fts5 vtable");
        conn.execute(
            "INSERT INTO code_fts(path,symbols,content) VALUES('a b','','hello world')",
            [],
        )
        .expect("fts5 insert");
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM code_fts WHERE code_fts MATCH '{content} : (\"hello\")'",
                [],
                |r| r.get(0),
            )
            .expect("fts5 match query");
        assert_eq!(n, 1);
    }
}
