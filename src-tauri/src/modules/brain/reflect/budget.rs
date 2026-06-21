//! The budget ledger — crash-safe spend accounting for reflect, the ONLY
//! token-spending path (EXECUTION_PLAN §4.1.2-4.1.4). Ordering is the whole point:
//!
//! ```text
//! 1. check_and_reserve(est)  -- ONE txn: verify spent+est <= ceiling, else
//!                               OverBudget (no call); INSERT 'reserved'; COMMIT
//! 2. (network call)          -- may crash the process mid-flight
//! 3. reconcile(rid, actual)  -- ONE txn: mark 'spent', fold actual into total
//! ```
//!
//! A crash between 1 and 3 leaves a committed 'reserved' row; the boot
//! [sweep_orphaned_reservations] charges it at its estimate. So a crashed call is
//! *over*-counted, never free — `spent_total_usd` can never silently reset or leak.

use rusqlite::Connection;

use super::ReflectReason;

/// Read the global spend ceiling (USD). `0.0` (default) = reflect disabled.
pub fn ceiling(conn: &Connection) -> f64 {
    conn.query_row("SELECT ceiling_usd FROM brain_budget WHERE id=1", [], |r| r.get(0))
        .unwrap_or(0.0)
}

/// Read the authoritative running spend total (USD). Never recomputed from rows.
pub fn spent_total(conn: &Connection) -> f64 {
    conn.query_row("SELECT spent_total_usd FROM brain_budget WHERE id=1", [], |r| r.get(0))
        .unwrap_or(0.0)
}

/// Set the monthly ceiling (the wizard / settings write). `0.0` disables reflect.
pub fn set_ceiling(conn: &Connection, ceiling_usd: f64, now: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE brain_budget SET ceiling_usd=?1, updated_at=?2 WHERE id=1",
        rusqlite::params![ceiling_usd.max(0.0), now],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Atomically verify `spent_total + est ≤ ceiling` and, if so, durably INSERT a
/// `reserved` ledger row (committed BEFORE the caller makes the network call).
/// Returns the reservation rowid, or `Disabled` (ceiling 0) / `OverBudget` BEFORE
/// any I/O. The reservation's durability is what makes a mid-call crash safe.
pub fn check_and_reserve(
    conn: &Connection,
    model: &str,
    est_cost_usd: f64,
    now: i64,
) -> Result<i64, ReflectReason> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| ReflectReason::CallFailed(e.to_string()))?;
    let (ceiling_usd, spent): (f64, f64) = tx
        .query_row(
            "SELECT ceiling_usd, spent_total_usd FROM brain_budget WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| ReflectReason::CallFailed(e.to_string()))?;
    // Outstanding committed reservations (a failed/in-flight reconcile leaves a
    // 'reserved' row) count against the ceiling, so a write fault that strands a
    // reconcile can't let a LATER reflect in the same session spend past the cap —
    // the over-count is reconciled away on the next boot sweep.
    let reserved: f64 = tx
        .query_row(
            "SELECT COALESCE(SUM(est_cost_usd),0.0) FROM brain_budget_ledger WHERE status='reserved'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    // Fail SAFE (toward NOT spending) on any non-finite value — a NaN comparison is
    // always false, which would otherwise defeat both gates below.
    if !ceiling_usd.is_finite() || !spent.is_finite() || !reserved.is_finite() || !est_cost_usd.is_finite() {
        return Err(ReflectReason::Disabled);
    }
    if ceiling_usd <= 0.0 {
        return Err(ReflectReason::Disabled);
    }
    if spent + reserved + est_cost_usd > ceiling_usd {
        return Err(ReflectReason::OverBudget);
    }
    tx.execute(
        "INSERT INTO brain_budget_ledger(status,est_cost_usd,model,reserved_at)
         VALUES('reserved',?1,?2,?3)",
        rusqlite::params![est_cost_usd, model, now],
    )
    .map_err(|e| ReflectReason::CallFailed(e.to_string()))?;
    let id = tx.last_insert_rowid();
    tx.commit().map_err(|e| ReflectReason::CallFailed(e.to_string()))?;
    Ok(id)
}

/// Mark a reservation `spent` with its actual cost and fold that into the running
/// total — in ONE transaction. Only acts on a row still `reserved` (idempotent: a
/// second call, or a row already swept, folds nothing).
pub fn reconcile(conn: &Connection, reservation_id: i64, actual_cost_usd: f64, now: i64) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let changed = tx
        .execute(
            "UPDATE brain_budget_ledger SET status='spent', actual_cost_usd=?2, reconciled_at=?3
             WHERE id=?1 AND status='reserved'",
            rusqlite::params![reservation_id, actual_cost_usd, now],
        )
        .map_err(|e| e.to_string())?;
    if changed > 0 {
        tx.execute(
            "UPDATE brain_budget SET spent_total_usd = spent_total_usd + ?1, updated_at=?2 WHERE id=1",
            rusqlite::params![actual_cost_usd, now],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Boot sweep: any row still `reserved` (= a crash between reserve and reconcile)
/// is charged at its `est_cost_usd` (conservative — over-count a crash, never leak
/// free spend) and marked `spent`. Returns how many orphans were swept.
pub fn sweep_orphaned_reservations(conn: &Connection, now: i64) -> Result<usize, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let (count, total): (i64, f64) = tx
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(est_cost_usd),0.0) FROM brain_budget_ledger WHERE status='reserved'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if count > 0 {
        tx.execute(
            "UPDATE brain_budget_ledger
             SET status='spent', actual_cost_usd=est_cost_usd, reconciled_at=?1
             WHERE status='reserved'",
            [now],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE brain_budget SET spent_total_usd = spent_total_usd + ?1, updated_at=?2 WHERE id=1",
            rusqlite::params![total, now],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::brain::store::migrate::migrate;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn disabled_when_ceiling_zero() {
        let conn = db();
        assert_eq!(ceiling(&conn), 0.0, "default off");
        let r = check_and_reserve(&conn, "m", 0.001, 1);
        assert!(matches!(r, Err(ReflectReason::Disabled)));
    }

    #[test]
    fn overbudget_blocks_before_reserve() {
        let conn = db();
        set_ceiling(&conn, 0.01, 1).unwrap();
        // already spent up to the ceiling.
        conn.execute("UPDATE brain_budget SET spent_total_usd=0.01 WHERE id=1", []).unwrap();
        let r = check_and_reserve(&conn, "m", 0.001, 2);
        assert!(matches!(r, Err(ReflectReason::OverBudget)));
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM brain_budget_ledger", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "over-budget reserves nothing");
    }

    #[test]
    fn reserve_then_reconcile_updates_spent_once() {
        let conn = db();
        set_ceiling(&conn, 1.0, 1).unwrap();
        let rid = check_and_reserve(&conn, "m", 0.01, 2).unwrap();
        reconcile(&conn, rid, 0.004, 3).unwrap();
        assert!((spent_total(&conn) - 0.004).abs() < 1e-9, "spent = actual");
        // a second reconcile is a no-op (row no longer 'reserved').
        reconcile(&conn, rid, 0.004, 4).unwrap();
        assert!((spent_total(&conn) - 0.004).abs() < 1e-9, "no double-count");
    }

    #[test]
    fn outstanding_reservation_counts_against_ceiling() {
        // A stranded reservation (failed reconcile) must block a later reflect from
        // spending past the ceiling until the boot sweep folds it.
        let conn = db();
        set_ceiling(&conn, 0.03, 1).unwrap();
        let _r1 = check_and_reserve(&conn, "m", 0.02, 2).unwrap(); // reserved, NOT reconciled
        // spent_total is still 0, but 0.02 is outstanding → only 0.01 headroom left.
        let r2 = check_and_reserve(&conn, "m", 0.02, 3);
        assert!(matches!(r2, Err(ReflectReason::OverBudget)), "outstanding reservation enforced");
        // a small one that fits the remaining headroom still succeeds.
        assert!(check_and_reserve(&conn, "m", 0.005, 4).is_ok());
    }

    #[test]
    fn non_finite_ceiling_fails_safe_no_spend() {
        let conn = db();
        // A non-finite ceiling (corruption / out-of-band write) would defeat ordered
        // comparisons (every compare with NaN/Inf is false). SQLite nullifies NaN,
        // so use +Inf; either way the invariant is: NO reservation is created.
        conn.execute("UPDATE brain_budget SET ceiling_usd=?1 WHERE id=1", [f64::INFINITY]).unwrap();
        assert!(check_and_reserve(&conn, "m", 0.001, 1).is_err(), "corrupt ceiling must not reserve");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM brain_budget_ledger", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "no spend on a corrupt ceiling");
    }

    #[test]
    fn crash_midcall_is_overcounted_never_leaked() {
        let conn = db();
        set_ceiling(&conn, 1.0, 1).unwrap();
        let _rid = check_and_reserve(&conn, "m", 0.02, 2).unwrap();
        // process "crashes" here: row stays 'reserved', spent_total still 0.
        assert_eq!(spent_total(&conn), 0.0);
        // boot sweep charges the ESTIMATE (conservative).
        let swept = sweep_orphaned_reservations(&conn, 3).unwrap();
        assert_eq!(swept, 1);
        assert!((spent_total(&conn) - 0.02).abs() < 1e-9, "charged the estimate");
        // sweeping again does nothing (no reserved rows left) — monotonic.
        assert_eq!(sweep_orphaned_reservations(&conn, 4).unwrap(), 0);
        assert!((spent_total(&conn) - 0.02).abs() < 1e-9, "spent never resets");
    }
}
