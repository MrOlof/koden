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

    // On any upgrade, drop the DERIVED (file-backed) tables so the DDL recreates
    // them at the current schema and the next warm pass rebuilds them — backfills
    // new columns + the AST-fed `symbols` column. Canonical data (notes,
    // proposals, reject_signatures) is preserved (preserve-over-destroy).
    if matches!(current, Some(v) if v < SCHEMA_VERSION) {
        conn.execute_batch(
            "DROP TABLE IF EXISTS code_fts;
             DROP TABLE IF EXISTS code_nodes;
             DROP TABLE IF EXISTS code_imports;
             DROP TABLE IF EXISTS code_edges;
             DELETE FROM files;",
        )?;
    }

    conn.execute_batch(DDL)?;

    match current {
        Some(v) if v == SCHEMA_VERSION => Ok(v),
        // None (fresh) or older: set/advance to current. Future versioned
        // migration steps slot in here, each guarded by `v < N`.
        _ => {
            conn.execute(
                "INSERT INTO brain_meta(key,value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [SCHEMA_VERSION.to_string()],
            )?;
            Ok(SCHEMA_VERSION)
        }
    }
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
