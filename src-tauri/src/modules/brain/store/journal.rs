//! Canonical-tail sidecar journal — an append-only JSONL backup-of-last-resort for
//! the CANONICAL tables (human decisions + spend state; see
//! [`migrate::CANONICAL_TABLES`](super::migrate::CANONICAL_TABLES)). It closes the
//! 2026-07-06 probe gap: when a SQLite HEADER is destroyed so badly the
//! corrupt-cache ATTACH salvage (`sqlite.rs::salvage_canonical`) can read nothing,
//! the canonical rows would otherwise be lost silently. This journal replays them.
//!
//! ## Ordering contract (the DB is the source of truth; the journal is the backup)
//!
//! Every canonical write commits its SQLite transaction FIRST; only AFTER the commit
//! do we append one JSONL line describing the new row state. Consequences:
//!
//! - The journal can only ever LAG the DB, never lead it. A crash in the (small)
//!   window between `COMMIT` and the append loses only the JOURNAL LINE for that one
//!   write — the committed DB row is safe, and the DB is authoritative on the next
//!   healthy boot.
//! - The journal is consulted for reads EXACTLY ONCE: inside `open_with_recovery`'s
//!   rename-aside branch, after the fresh schema is created, replayed on top of
//!   whatever the ATTACH salvage recovered. A healthy open NEVER replays it.
//! - Appends are best-effort: a journal I/O failure is logged and swallowed — it must
//!   never fail an already-committed DB write.
//!
//! ## Replay
//!
//! Lines are applied in `seq` order, idempotently (INSERT OR REPLACE for upserts,
//! keyed DELETE for deletes), gated by a high-water mark (`canonical_replay_seq`)
//! stored INSIDE the rebuilt DB so a re-entered replay resumes without double-apply.
//! Corrupt / truncated lines are skipped with a warning, never a panic.
//!
//! ## Compaction ceiling
//!
//! `compact_if_large` rewrites the journal from live DB state when it exceeds
//! [`COMPACT_CAP_BYTES`] at a healthy recovery-wrapped open (never on the rebuild
//! path — that would erase the backup before replay). See the fn for the ceiling.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection, OptionalExtension};

/// The canonical tables the journal covers. This is `CANONICAL_TABLES` MINUS
/// `brain_budget_ledger`: the ledger is crash-safety scaffolding whose only DURABLE
/// consequence — `spent_total_usd` — is itself journaled via `brain_budget`. So a
/// fresh-DB replay starts with an EMPTY ledger (no dangling 'reserved' rows, hence
/// the boot sweep can't double-charge), while spend enforcement is fully restored.
/// MINUS `brain_activity` too (ADR-020): the session trail is high-frequency and
/// loss-tolerant — journaling it would churn this 8MB-capped low-frequency sidecar
/// for rows whose loss costs context, never correctness.
/// This list is BOTH the compaction source set AND the replay table whitelist — a
/// line naming any other table is rejected, so a generic SQL builder can never touch
/// a non-canonical (or bogus) table. Keep it in lockstep with the append call sites.
pub(crate) const JOURNALED_TABLES: &[&str] = &[
    "proposals",
    "reject_signatures",
    "brain_budget",
    "brain_librarian",
    "brain_librarian_pin",
    "brain_semantic_meta",
];

/// brain_meta key holding the highest journal `seq` already replayed into THIS db.
const HW_KEY: &str = "canonical_replay_seq";

// ponytail: the journal is bounded by rewrite-from-live-DB at healthy open once it
// passes this cap (canonical writes are low-frequency — proposals on human curate,
// budget on paid reflect rounds — so this is a slow-growing log; the cap is a
// safety net, not a hot path). Upgrade path if the rate ever grows: segment +
// drop-oldest, or fold the high-water into a periodic checkpoint line.
const COMPACT_CAP_BYTES: u64 = 8 * 1024 * 1024;

/// One journal line. `data` is a column→value map (all columns for an upsert; the
/// keyed columns for a delete). `ts` is informational only (never used by replay).
#[derive(serde::Serialize, serde::Deserialize)]
struct JournalLine {
    seq: u64,
    table: String,
    op: String, // "upsert" | "delete"
    data: serde_json::Map<String, serde_json::Value>,
    ts: i64,
}

/// The append-only sidecar, sited next to the DB file. Single-writer (owned by the
/// worker's writer `SqliteIndex`); the `Mutex` guards the seq counter defensively
/// and lets the whole struct be `&self`.
pub struct CanonicalJournal {
    path: PathBuf,
    seq: Mutex<u64>,
}

/// `<db-stem>.canonical.jsonl` next to the DB (e.g. `index.sqlite` →
/// `index.canonical.jsonl`). NOT moved by `rename_corrupt_aside` (which only relocates
/// the db + its `-wal`/`-shm`), so it survives a rename-aside and replays into the
/// fresh store at the same logical location.
fn journal_path(db_path: &Path) -> PathBuf {
    let stem = db_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("index");
    db_path.with_file_name(format!("{stem}.canonical.jsonl"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Only `[A-Za-z0-9_]` identifiers reach a generated SQL string (defense in depth on
/// top of the [`JOURNALED_TABLES`] whitelist) — a tampered journal can't inject.
fn is_safe_ident(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn sql_to_json(v: SqlValue) -> serde_json::Value {
    match v {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(i) => serde_json::Value::from(i),
        // from(f64) yields Null for a non-finite value — fine: replay restores the
        // DDL default rather than a poisoned NaN/Inf.
        SqlValue::Real(f) => serde_json::Value::from(f),
        SqlValue::Text(s) => serde_json::Value::from(s),
        // Canonical tables have no BLOB columns; encode defensively so a future one
        // still round-trips rather than silently dropping.
        SqlValue::Blob(b) => serde_json::Value::from(b),
    }
}

fn json_to_sql(v: &serde_json::Value) -> SqlValue {
    match v {
        serde_json::Value::Null => SqlValue::Null,
        serde_json::Value::Bool(b) => SqlValue::Integer(*b as i64),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SqlValue::Integer(i)
            } else if let Some(u) = n.as_u64() {
                SqlValue::Integer(u as i64)
            } else {
                SqlValue::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => SqlValue::Text(s.clone()),
        // Arrays/objects only arise from a BLOB round-trip or tampering; store the
        // JSON text so nothing is lost.
        other => SqlValue::Text(other.to_string()),
    }
}

/// Read all VALID lines (skipping corrupt/truncated ones) sorted by `seq`. Also the
/// seq-initialization source at open. Bounded by the compaction cap.
fn read_valid_lines(path: &Path) -> Vec<JournalLine> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut out: Vec<JournalLine> = Vec::new();
    for (n, line) in BufReader::new(file).lines().enumerate() {
        let Ok(line) = line else { break }; // read error → stop (tail unreadable)
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JournalLine>(&line) {
            Ok(entry) => out.push(entry),
            Err(e) => log::warn!(
                "brain: skipping corrupt canonical-journal line {} ({e})",
                n + 1
            ),
        }
    }
    out.sort_by_key(|e| e.seq);
    out
}

impl CanonicalJournal {
    /// Attach the journal at the DB's sibling path. Does NOT create the file (only an
    /// append does), so a failed open leaves nothing behind. Seeds the seq counter
    /// from the last valid line so a restart continues monotonically.
    pub fn open(db_path: &Path) -> Self {
        let path = journal_path(db_path);
        let seq = read_valid_lines(&path).last().map(|e| e.seq).unwrap_or(0);
        Self { path, seq: Mutex::new(seq) }
    }

    fn append(&self, table: &str, op: &str, data: serde_json::Map<String, serde_json::Value>) {
        let mut seq = self.seq.lock().unwrap_or_else(|p| p.into_inner());
        *seq += 1;
        let line = JournalLine {
            seq: *seq,
            table: table.to_string(),
            op: op.to_string(),
            data,
            ts: now_ms(),
        };
        // Best-effort: the DB already committed; a journal failure must not propagate.
        if let Err(e) = write_line(&self.path, &line) {
            log::warn!("brain: canonical-journal append failed ({e}); DB remains source of truth");
        }
    }

    /// Append an upsert of every column of a single row read back from the DB. Called
    /// AFTER the row's txn commits, so it captures the exact durable state. A missing
    /// row (e.g. the write was a no-op) journals nothing.
    pub fn append_row(
        &self,
        conn: &Connection,
        table: &str,
        where_sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) {
        match read_row(conn, table, where_sql, params) {
            Ok(Some(map)) => self.append(table, "upsert", map),
            Ok(None) => {} // nothing to back up
            Err(e) => log::warn!("brain: canonical-journal readback of {table} failed ({e})"),
        }
    }

    /// Append a keyed delete. `key` is a JSON object of the WHERE columns; replay
    /// builds `DELETE FROM <table> WHERE c1=? AND …`. An empty key is ignored (would
    /// mean "delete the whole table", never intended).
    pub fn append_delete(&self, table: &str, key: serde_json::Value) {
        if let serde_json::Value::Object(map) = key {
            if !map.is_empty() {
                self.append(table, "delete", map);
            }
        }
    }

    /// Replay the journal into `conn` — ONLY from the corrupt-cache rebuild path.
    /// Idempotent + resumable via the in-DB high-water mark. Never panics.
    pub fn replay(&self, conn: &Connection) {
        let entries = read_valid_lines(&self.path);
        if entries.is_empty() {
            return;
        }
        let hw = read_hw(conn);
        let pending: Vec<&JournalLine> = entries.iter().filter(|e| e.seq > hw).collect();
        if pending.is_empty() {
            return;
        }
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(e) => {
                log::warn!("brain: canonical-journal replay could not open a txn ({e})");
                return;
            }
        };
        let mut applied = 0usize;
        let mut max_seq = hw;
        for e in &pending {
            match apply_entry(&tx, e) {
                Ok(true) => applied += 1,
                Ok(false) => {} // skipped (bad table/op/columns) — already warned
                Err(err) => {
                    log::warn!("brain: canonical-journal replay of seq {} failed ({err})", e.seq);
                }
            }
            max_seq = max_seq.max(e.seq);
        }
        // High-water + all rows advance atomically: a crash rolls the whole replay
        // back and re-runs from the same `hw` (idempotent upserts), never partway.
        if let Err(e) = set_hw(&tx, max_seq) {
            log::warn!("brain: canonical-journal replay could not stamp high-water ({e})");
        }
        match tx.commit() {
            Ok(()) => log::info!(
                "brain: replayed {applied} canonical journal line(s) into the rebuilt store (seq→{max_seq})"
            ),
            Err(e) => log::warn!("brain: canonical-journal replay commit failed ({e}); canonical tail not restored"),
        }
    }

    /// Rewrite the journal from live DB state when it exceeds the size cap. Called
    /// ONLY on a healthy recovery-wrapped open (the DB holds the true canonical
    /// state) — NEVER on the rebuild path, where the fresh DB is still empty and a
    /// rewrite would erase the backup before replay. Atomic (write tmp + rename);
    /// any failure leaves the original journal untouched.
    pub fn compact_if_large(&self, conn: &Connection) {
        let size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size <= COMPACT_CAP_BYTES {
            return;
        }
        let tmp = self.path.with_extension("canonical.jsonl.compacting");
        if let Err(e) = self.compact_to(conn, &tmp) {
            log::warn!("brain: canonical-journal compaction failed ({e}); keeping the existing journal");
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Test seam: force a compaction regardless of size (the cap is a const, not
    /// injectable). Same code path as [`Self::compact_if_large`] past the size gate.
    #[cfg(test)]
    pub fn compact_now_for_test(&self, conn: &Connection) {
        let tmp = self.path.with_extension("canonical.jsonl.compacting");
        self.compact_to(conn, &tmp).expect("test compaction");
    }

    fn compact_to(&self, conn: &Connection, tmp: &Path) -> Result<(), String> {
        let mut seq = self.seq.lock().unwrap_or_else(|p| p.into_inner());
        let file = File::create(tmp).map_err(|e| e.to_string())?;
        let mut w = BufWriter::new(file);
        let mut next: u64 = 0;
        for table in JOURNALED_TABLES {
            let rows = read_all_rows(conn, table).map_err(|e| e.to_string())?;
            for data in rows {
                next += 1;
                let line = JournalLine {
                    seq: next,
                    table: (*table).to_string(),
                    op: "upsert".to_string(),
                    data,
                    ts: now_ms(),
                };
                let s = serde_json::to_string(&line).map_err(|e| e.to_string())?;
                writeln!(w, "{s}").map_err(|e| e.to_string())?;
            }
        }
        w.flush().map_err(|e| e.to_string())?;
        drop(w);
        std::fs::rename(tmp, &self.path).map_err(|e| e.to_string())?;
        *seq = next;
        log::info!("brain: compacted canonical journal to {next} live row(s)");
        Ok(())
    }
}

/// Append a single serialized line, creating the file if needed.
fn write_line(path: &Path, line: &JournalLine) -> std::io::Result<()> {
    let s = serde_json::to_string(line).map_err(std::io::Error::other)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut w = BufWriter::new(file);
    writeln!(w, "{s}")?;
    w.flush()
}

/// Read one row as a column→JSON map. `SELECT *` so the journal is schema-generic:
/// column names come from the live statement, values are typed via SQLite's storage
/// class. Returns `None` when the row is absent.
fn read_row(
    conn: &Connection,
    table: &str,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Option<serde_json::Map<String, serde_json::Value>>> {
    // `table` is always a compile-time literal at the call sites; never user input.
    let sql = format!("SELECT * FROM {table} WHERE {where_sql}");
    conn.query_row(&sql, params, |row| Ok(row_to_map(row)))
        .optional()
}

fn read_all_rows(
    conn: &Connection,
    table: &str,
) -> rusqlite::Result<Vec<serde_json::Map<String, serde_json::Value>>> {
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table}"))?;
    let rows = stmt.query_map([], |row| Ok(row_to_map(row)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn row_to_map(row: &rusqlite::Row) -> serde_json::Map<String, serde_json::Value> {
    let stmt = row.as_ref();
    let n = stmt.column_count();
    let mut map = serde_json::Map::new();
    for i in 0..n {
        let name = match stmt.column_name(i) {
            Ok(name) => name.to_string(),
            Err(_) => continue,
        };
        let v: SqlValue = row.get(i).unwrap_or(SqlValue::Null);
        map.insert(name, sql_to_json(v));
    }
    map
}

/// Apply one replay entry. Returns `Ok(true)` if a statement ran, `Ok(false)` if the
/// entry was skipped (unknown table/op or unsafe column), `Err` on a SQL failure.
fn apply_entry(tx: &Connection, e: &JournalLine) -> rusqlite::Result<bool> {
    if !JOURNALED_TABLES.contains(&e.table.as_str()) {
        log::warn!("brain: canonical-journal replay skipping unknown table '{}'", e.table);
        return Ok(false);
    }
    let cols: Vec<&String> = e.data.keys().collect();
    if cols.is_empty() || !cols.iter().all(|c| is_safe_ident(c)) {
        log::warn!("brain: canonical-journal replay skipping seq {} (empty/unsafe columns)", e.seq);
        return Ok(false);
    }
    let vals: Vec<SqlValue> = e.data.values().map(json_to_sql).collect();
    let sql = match e.op.as_str() {
        "upsert" => {
            let names = cols.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(",");
            let ph = (1..=cols.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
            format!("INSERT OR REPLACE INTO {} ({names}) VALUES ({ph})", e.table)
        }
        "delete" => {
            let clause = cols
                .iter()
                .enumerate()
                .map(|(i, c)| format!("\"{c}\"=?{}", i + 1))
                .collect::<Vec<_>>()
                .join(" AND ");
            format!("DELETE FROM {} WHERE {clause}", e.table)
        }
        other => {
            log::warn!("brain: canonical-journal replay skipping seq {} (unknown op '{other}')", e.seq);
            return Ok(false);
        }
    };
    tx.execute(&sql, params_from_iter(vals.iter()))?;
    Ok(true)
}

fn read_hw(conn: &Connection) -> u64 {
    conn.query_row(
        "SELECT value FROM brain_meta WHERE key=?1",
        [HW_KEY],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|s| s.parse::<u64>().ok())
    .unwrap_or(0)
}

fn set_hw(conn: &Connection, seq: u64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO brain_meta(key,value) VALUES(?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![HW_KEY, seq.to_string()],
    )?;
    Ok(())
}
