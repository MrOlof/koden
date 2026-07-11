//! Versioned, idempotent migrations. All durable storage is versioned
//! (BUILD-PROMPT §13.4): corrupt *derived* data is safe to rebuild, the version
//! gate decides forward migrations. P0 establishes v1; later phases chain here.

use rusqlite::Connection;

use super::schema::{DDL, SCHEMA_VERSION};

/// DERIVED-from-disk tables (rebuilt by the next warm walk / memory scan): dropped
/// on ANY schema-version change so the DDL recreates them at THIS build's schema.
/// This is a drop-list, not a keep-list — NEVER add a canonical table here.
/// (`every_table_is_classified_derived_or_canonical` enforces that each DDL table is
/// classified in exactly one of the two lists.)
pub(crate) const DERIVED_TABLES: &[&str] =
    &["code_fts", "code_nodes", "code_imports", "code_edges", "notes", "files"];

/// Truly CANONICAL tables — human decisions + spend state, NOT re-derivable from
/// disk. Preserved across migrations purely by being ABSENT from [DERIVED_TABLES],
/// and best-effort salvaged out of a corrupt cache file by
/// `SqliteIndex::open_with_recovery`.
pub(crate) const CANONICAL_TABLES: &[&str] = &[
    "proposals",
    "reject_signatures",
    "brain_budget",
    "brain_budget_ledger",
    "brain_librarian",
    "brain_librarian_pin",
    "brain_semantic_meta",
];

/// Columns added to CANONICAL tables AFTER their first ship (ADR-018) —
/// `(table, column, column DDL)`. Canonical tables survive version changes by being
/// absent from the DROP batch, so `CREATE TABLE IF NOT EXISTS` can never add a
/// column to an EXISTING store, and a SCHEMA_VERSION bump would not either (bumps
/// only drop/rebuild DERIVED tables, while rotating every gist cache key). The
/// additive-canonical migration is therefore a guarded `ALTER TABLE ... ADD COLUMN`
/// run idempotently on every open ([ensure_additive_columns]). Keep this list in
/// lockstep with the base DDL (fresh stores get the columns from `schema::DDL`;
/// existing stores from here) — [tests::additive_columns_match_the_ddl] enforces it.
/// Column DDL must be ALTER-legal: defaults constant, no PK/UNIQUE.
const ADDITIVE_CANONICAL_COLUMNS: &[(&str, &str, &str)] = &[
    ("proposals", "applied_ms", "INTEGER"),
    ("proposals", "reverted_ms", "INTEGER"),
    ("proposals", "auto_applied", "INTEGER NOT NULL DEFAULT 0"),
    ("proposals", "undo_created_id", "TEXT"),
    ("proposals", "undo_prior_path", "TEXT"),
    ("proposals", "undo_prior_bytes", "TEXT"),
    ("brain_librarian", "curation_mode", "TEXT NOT NULL DEFAULT 'autonomous'"),
];

/// Add any missing additive-canonical column (see [ADDITIVE_CANONICAL_COLUMNS]).
/// Idempotent (PRAGMA table_info gate), cheap on the steady state (one PRAGMA per
/// table). Runs inside the caller's transaction on the upgrade path.
fn ensure_additive_columns(conn: &Connection) -> rusqlite::Result<()> {
    let mut cols_of: std::collections::HashMap<&str, Vec<String>> = std::collections::HashMap::new();
    for (table, col, ddl) in ADDITIVE_CANONICAL_COLUMNS {
        if !cols_of.contains_key(table) {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let names: Vec<String> =
                stmt.query_map([], |r| r.get::<_, String>(1))?.filter_map(Result::ok).collect();
            cols_of.insert(table, names);
        }
        let existing = cols_of.get_mut(table).expect("probed above");
        if !existing.iter().any(|c| c == col) {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ddl};"))?;
            existing.push((*col).to_string());
        }
    }
    Ok(())
}

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
    // ONLY a positively-missing row means "fresh store"; a read/parse ERROR must
    // propagate — treating it as fresh would skip the derived rebuild and then
    // stamp the current version over tables of unknown shape (ADR-010 cluster 4).
    // (An unparseable stamp surfaces as FromSqlConversionFailure, which the boot
    // recovery ladder classifies as a corrupt cache → rename aside + rebuild.)
    conn.execute_batch("CREATE TABLE IF NOT EXISTS brain_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
    let current: Option<i64> = match conn.query_row(
        "SELECT value FROM brain_meta WHERE key='schema_version'",
        [],
        |r| {
            let raw: String = r.get(0)?;
            raw.parse::<i64>().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
            })
        },
    ) {
        Ok(v) => Some(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e),
    };

    if matches!(current, Some(v) if v == SCHEMA_VERSION) {
        // Already current: just ensure the (idempotent) DDL is present — plus any
        // additive-canonical column a same-version store may still be missing
        // (CREATE TABLE IF NOT EXISTS cannot add columns to an existing table).
        conn.execute_batch(DDL)?;
        ensure_additive_columns(conn)?;
        return Ok(SCHEMA_VERSION);
    }

    // Fresh or upgrade: do the whole migration in ONE transaction so "advanced to
    // vN" is a single atomic, durable fact — a crash mid-migration rolls back
    // cleanly and re-runs, never leaving a half-dropped schema or an out-of-step
    // version row. (PRAGMAs above must stay OUTSIDE the txn.)
    let tx = conn.unchecked_transaction()?;
    // On ANY version change — upgrade OR DOWNGRADE (an older build opening a newer
    // store) — drop the DERIVED tables so the DDL recreates them at THIS build's
    // schema and the next warm pass rebuilds them — backfills new columns
    // (incl. the AST-fed `symbols`, `notes.supersedes`, and `files.accessed_*`).
    // A downgrade MUST rebuild too: silently stamping the version down would leave
    // derived tables shaped by a FUTURE schema under this build's queries.
    // DERIVED = rebuildable from disk: the code index (code_fts/code_nodes/
    // code_imports/code_edges + the `files` manifest — DROPped, not just DELETEd, so
    // added columns backfill) AND `notes` (re-scanned from the `.md` files by
    // `scan_project_memory` on the next warm pass). Truly CANONICAL data — proposals,
    // reject_signatures (human decisions), brain_budget(+ledger) (spend state),
    // brain_semantic_meta — is preserved PURELY by being absent from this DROP batch
    // ([DERIVED_TABLES] is a drop-list, not a keep-list — NEVER add a canonical
    // table to it). `current` here is Some(v != SCHEMA_VERSION) or None (fresh —
    // nothing to drop).
    if current.is_some() {
        for t in DERIVED_TABLES {
            tx.execute_batch(&format!("DROP TABLE IF EXISTS {t};"))?;
        }
    }
    tx.execute_batch(DDL)?;
    // Canonical tables were NOT dropped above, so an older store may still be
    // missing additive columns the fresh DDL carries — add them here, atomically
    // with the version stamp.
    ensure_additive_columns(&tx)?;
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
    fn upgrade_preserves_canonical_drops_derived() {
        // The load-bearing migration invariant: human/spend state (proposals,
        // reject_signatures, brain_budget) MUST survive an upgrade, while DERIVED-
        // from-disk tables (notes — rebuilt by scan) are dropped + rebuilt. A
        // regression that moved a canonical table into the DROP batch would
        // resurrect declined proposals on every version bump — this guards it.
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO proposals(project_id,signature,action,target_id,title,detail,source,status,created_ms)
             VALUES('p','sig','archive','n','t','d','curate','rejected',1)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO reject_signatures(project_id,reject_sig) VALUES('p','rj')", []).unwrap();
        conn.execute("UPDATE brain_budget SET spent_total_usd=0.5 WHERE id=1", []).unwrap();
        conn.execute(
            "INSERT INTO notes(project_id,id,path,hash) VALUES('p','n1','.koden-memory/n1.md','h')",
            [],
        )
        .unwrap();
        // force an upgrade
        conn.execute("UPDATE brain_meta SET value='1' WHERE key='schema_version'", []).unwrap();
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);

        let count = |t: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count("proposals"), 1, "CANONICAL proposals must survive (declined stays declined)");
        assert_eq!(count("reject_signatures"), 1, "CANONICAL reject_signatures must survive");
        let spent: f64 = conn.query_row("SELECT spent_total_usd FROM brain_budget WHERE id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(spent, 0.5, "CANONICAL budget spend must survive");
        assert_eq!(count("notes"), 0, "DERIVED notes is dropped + rebuilt by the next scan");
    }

    #[test]
    fn downgrade_rebuilds_derived_and_preserves_canonical() {
        // ADR-010 cluster 4: an older build opening a NEWER store must not silently
        // stamp the version down — the derived tables may carry a future schema this
        // build's queries don't understand. They are dropped + rebuilt; canonical
        // rows survive exactly as on an upgrade.
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO files(project_id,path,hash,size,fts_rowid) VALUES('p','x.rs','h',1,1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proposals(project_id,signature,action,target_id,title,detail,source,status,created_ms)
             VALUES('p','sig','archive','n','t','d','curate','rejected',1)",
            [],
        )
        .unwrap();
        // Simulate a store written by a FUTURE build.
        conn.execute(
            "UPDATE brain_meta SET value=?1 WHERE key='schema_version'",
            [(SCHEMA_VERSION + 1).to_string()],
        )
        .unwrap();
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        let files: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(files, 0, "downgrade rebuilds derived tables");
        let props: i64 = conn.query_row("SELECT COUNT(*) FROM proposals", [], |r| r.get(0)).unwrap();
        assert_eq!(props, 1, "canonical rows survive a downgrade");
        let v: String = conn
            .query_row("SELECT value FROM brain_meta WHERE key='schema_version'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION.to_string(), "version stamped back to this build");
    }

    #[test]
    fn unparseable_version_stamp_is_an_error_not_fresh() {
        // ADR-010 cluster 4: a version stamp that exists but cannot be read as an
        // integer must FAIL migrate — treating it as "fresh" would skip the derived
        // rebuild and stamp the current version over tables of unknown shape. (The
        // boot recovery ladder then classifies this as a corrupt cache and rebuilds.)
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute("UPDATE brain_meta SET value='not-a-number' WHERE key='schema_version'", [])
            .unwrap();
        assert!(migrate(&conn).is_err(), "garbage schema_version must not read as a fresh store");
    }

    #[test]
    fn every_table_is_classified_derived_or_canonical() {
        // The rebuild contract's completeness gate: every table the DDL creates must
        // be EXPLICITLY classified — DERIVED (dropped + rebuilt on any version
        // change) or CANONICAL (preserved + salvaged on corrupt-cache rebuild). An
        // unclassified new table would silently survive upgrades with a stale schema.
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
            .unwrap();
        let tables: Vec<String> =
            stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        for t in &tables {
            let classified = t == "brain_meta" // the version stamp — the migration gate itself
                || DERIVED_TABLES.contains(&t.as_str())
                || CANONICAL_TABLES.contains(&t.as_str())
                // FTS5 shadow tables (code_fts_data/_idx/…) drop with their vtable.
                || DERIVED_TABLES.iter().any(|d| t.starts_with(&format!("{d}_")));
            assert!(
                classified,
                "table '{t}' must be added to DERIVED_TABLES (dropped+rebuilt on version change) \
                 or CANONICAL_TABLES (preserved + salvaged)"
            );
        }
        // Both lists must name REAL tables (a typo would silently skip a drop/salvage)…
        for t in DERIVED_TABLES.iter().chain(CANONICAL_TABLES) {
            assert!(tables.iter().any(|x| x == t), "listed table '{t}' does not exist in the DDL");
        }
        // …and never overlap.
        for t in DERIVED_TABLES {
            assert!(!CANONICAL_TABLES.contains(t), "'{t}' cannot be both derived and canonical");
        }
    }

    #[test]
    fn additive_columns_added_to_existing_stores_and_defaults_hold() {
        // ADR-018 migration: a store created BEFORE the additive-canonical columns
        // (old-shape `proposals` / `brain_librarian`, version already current) must
        // gain them on the next open, preserving existing rows — the exact path a
        // real upgrade takes, since canonical tables are never dropped/rebuilt.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE brain_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE proposals (
                 project_id TEXT NOT NULL, signature TEXT NOT NULL, action TEXT NOT NULL,
                 target_id TEXT, title TEXT NOT NULL, detail TEXT NOT NULL,
                 source TEXT NOT NULL, status TEXT NOT NULL, created_ms INTEGER NOT NULL,
                 PRIMARY KEY (project_id, signature));
             CREATE TABLE brain_librarian (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 provider TEXT NOT NULL DEFAULT 'anthropic',
                 model TEXT NOT NULL DEFAULT 'claude-haiku-4-5',
                 base_url TEXT NOT NULL DEFAULT '',
                 in_rate_mtok REAL NOT NULL DEFAULT 1.0,
                 out_rate_mtok REAL NOT NULL DEFAULT 5.0,
                 updated_at INTEGER NOT NULL DEFAULT 0);
             INSERT INTO brain_librarian (id, provider) VALUES (1, 'openai');
             INSERT INTO proposals VALUES ('p','sig','archive','n','t','d','curate','pending',1);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO brain_meta(key,value) VALUES('schema_version', ?1)",
            [SCHEMA_VERSION.to_string()],
        )
        .unwrap();

        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);

        // Columns exist; the legacy row survived with the ALTER defaults.
        let (status, auto, undo): (String, i64, Option<String>) = conn
            .query_row(
                "SELECT status, auto_applied, undo_created_id FROM proposals WHERE signature='sig'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "pending", "legacy row preserved");
        assert_eq!(auto, 0, "auto_applied defaults 0");
        assert_eq!(undo, None, "no snapshot on a legacy row");
        let (provider, mode): (String, String) = conn
            .query_row("SELECT provider, curation_mode FROM brain_librarian WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(provider, "openai", "existing librarian selection preserved");
        assert_eq!(mode, "autonomous", "curation mode defaults AUTONOMOUS (ADR-018)");

        // Idempotent: a second open neither errors nor duplicates columns.
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn additive_columns_match_the_ddl() {
        // Lockstep gate: every additive-canonical column must exist on a FRESH store
        // (i.e. the base DDL also carries it), so fresh and upgraded stores converge
        // to the same shape — and each must name a real canonical table.
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        for (table, col, _) in ADDITIVE_CANONICAL_COLUMNS {
            assert!(
                CANONICAL_TABLES.contains(table),
                "additive column '{col}' targets non-canonical table '{table}'"
            );
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            let names: Vec<String> =
                stmt.query_map([], |r| r.get::<_, String>(1)).unwrap().filter_map(Result::ok).collect();
            assert!(
                names.iter().any(|c| c == col),
                "column '{table}.{col}' missing from the base DDL — add it to schema::DDL too"
            );
        }
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
