# ADR-016 — Librarian LLM call off the worker thread

Status: **Accepted** — 2026-07-11, decided by Claude as Kosta's pre-approved
proxy (overnight Librarian gauntlet mandate). Implementation same night,
gated on the L2 acceptance re-run; if verification does not converge, this
drops back to Proposed and ships nothing.

## Finding (LIB-DESIGN-01, measured)

The brain worker is a single-threaded event loop (`worker.rs`, `for ev in rx`).
A Librarian round runs `reflect_auto` → `client.complete()` **inline on that
thread**, so the network call blocks all FS-delta indexing for its duration.

Gauntlet leg L2 numbers (2026-07-11, real watcher, 1502-file fixture, 5
concurrent activity streams):

| Metric | No round | During a 15 s round |
|---|---|---|
| FS-delta apply lag | median 202 ms | **20.14 s** (call + 400-event backlog drain) |
| Search latency | p50 5.4 ms | p50 6.6 ms (fast — but **stale** results) |
| Event loss | 0 | 0 (mpsc buffers losslessly) |
| Real round (fast provider) | — | 1.54 s call → 2.01 s staleness |

Impact scales with provider latency: a reasoning-family or overloaded provider
(15–30 s realistic) silently freezes index freshness for the whole call while
searches keep answering quickly from stale data — the exact inversion of the
brain's freshness promise. Consistency is NOT at risk (the same serialization
that causes the stall guarantees no interleaved writes; L2-S3 verified zero
corruption, zero orphans).

## Decision

Move only the **network call** off the worker thread; keep every DB touch on it:

1. Worker (on `Tick`, gates passed): build digest + `check_and_reserve`
   (both on-thread, exactly as today), then spawn the `complete()` call on a
   detached side thread and **continue the event loop**.
2. Side thread does network I/O ONLY — no `SqliteIndex` access — and sends
   `BrainEvent::ReflectDone { project, reservation, result }` back through the
   existing channel.
3. Worker handles `ReflectDone`: reconcile + validate + enqueue proposals —
   single-writer invariant preserved by construction (the writer never left
   the worker thread).
4. At most ONE round in flight per project (a pending-reservation flag in
   `LibrarianAuto`); a second Tick while in flight is a no-op — same cadence
   semantics as today.
5. Crash mid-call unchanged: the reservation is durable before spawn; the
   boot sweep charges orphans at estimate (L3-S4/S5 behavior stays the proof).
   A `ReflectDone` for a swept reservation reconciles as already-settled
   (idempotent — the sweep and the late reply must not double-charge).

## Acceptance (must pass before commit)

- L2-S2 re-run: staleness window during a 15 s fake round drops from ~20 s to
  ~baseline (≤ 1 s); event loss stays 0; search latency unchanged.
- L3 kill legs re-run: kill mid-call still sweeps at estimate, ledger sums.
- `librarian_rounds` suite unchanged (round-decision semantics untouched).
- New regression test: a delta arriving mid-round is searchable before the
  round completes.

## Rejected alternative

Full worker→thread-pool rework: touches every event class for a problem only
the LLM call has. The call is the only multi-second off-CPU wait in the loop.
