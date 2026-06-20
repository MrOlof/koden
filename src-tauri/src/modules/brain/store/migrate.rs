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
    conn.execute_batch(DDL)?;

    let current: Option<i64> = conn
        .query_row(
            "SELECT value FROM brain_meta WHERE key='schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| s.parse().ok());

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
