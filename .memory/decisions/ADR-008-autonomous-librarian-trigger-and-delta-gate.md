# ADR-008: Autonomous Librarian — event-driven trigger + delta-gated reflect

Status: Accepted — 2026-06-22 (supersedes the "manual, never on a timer" stance in ADR-006/reflect mod docs)

## Context

ADR-006 shipped reflect as **manual-only, default-$0** (a Reflect button; `reflect_once`
comment literally said "Never on a timer"). The manual gate was a second safety layer on
top of the budget ceiling. Kosta's objection (2026-06-22): a "behind the scenes Librarian"
that only runs when you press a button isn't behind the scenes — it defeats the purpose.

Key realization: the **budget ceiling is the real safety mechanism**, not the manual
trigger. Once a ceiling > 0 is set, autonomous background work is safe by construction
(she runs until the cap, then stops). The manual-only gate was over-cautious and conflicts
with the product vision. Kosta's direction: a **smart, non-LLM trigger** on accumulated
change, plus **delta updates** so cost scales with what's new, not corpus size.

## Decision

Make reflect autonomous, gated by two cheap (non-LLM) mechanisms, with the budget cap as
the spend throttle.

- **Trigger (free) — EVENT-DRIVEN, not a count and not a clock:** an incremental watcher pass
  marks the project `dirty` and stamps `last_change_ms`. The 60 s Tick runs `run_librarian_rounds`,
  which fires ONE delta-gated reflect for a dirty project when `due_for_round` holds: past the
  anti-hammer `LIBRARIAN_MIN_GAP_MS` (5 min) AND **either** it has gone quiet for
  `LIBRARIAN_IDLE_SETTLE_MS` (3 min — "she works in the gaps, never interrupting active edits")
  **or** an AI session just **exited** in it (a "settle now" boundary; `handle_agent` now returns
  the resolved project, and the `exited` arm sets `boundary`). `due_for_round` is a pure,
  unit-tested predicate. The design evolved same-day through two rejected forms: a fixed change
  COUNT (`LIBRARIAN_CHANGE_THRESHOLD = 20` — a bad proxy: 20 edits is noise in a huge repo, a
  rewrite in a tiny one) and a fixed 15-min CLOCK (`LIBRARIAN_ROUNDS_MS` — oblivious to what you're
  doing). Event-driven maps to "when would a real librarian step in?" — when the room goes quiet,
  or right after someone finishes. Only **incremental** changes set dirty; the startup/rescan bulk
  index does NOT (else opening the app would reflect).
- **Delta gate (the cost-saver):** `reflect::reflect_auto` builds the digest (notes + structural
  findings) locally, blake3-hashes it, and **skips the paid call** (`ReflectReason::Unchanged`,
  $0, zero requests) when the hash is byte-identical to the last successful pass. Because the
  digest already folds doctor findings, a code change that makes a note stale DOES change the
  hash and re-spends; a change that affects nothing the Librarian consumes costs $0.
- **Refactor (no churn):** `reflect_with_client` keeps its signature (13 test call sites
  untouched). New `build_digest` (shared, so the gate's hash matches what's sent),
  `reflect_auto_with_client` (offline-testable delta core), `reflect_auto` (resolve config/key
  then delta core). `reflect_once` is now a thin `prev=None` wrapper (manual click never skips).
- **Curate stays manual.** It can auto-modify memory (the ACT band archives notes); reflect only
  PROPOSES into the human-gated review inbox. So the autonomous path can't silently rewrite
  memory. Curate autonomy is a separate future decision.

Alternatives rejected: a fixed periodic timer (ignores whether anything changed — wasteful);
delta-by-changed-notes-only (loses reflect's cross-note consolidation, which is holistic);
persisting the counter/hash across restarts (in-memory is fine — the counter only needs to
accrue while the app is open; a restart just resets to 0, costing at most one extra pass).

## Consequences

- The Librarian is now genuinely behind-the-scenes: tidies as you work, within budget, silent
  and free when nothing material changed. Setting a budget ceiling is what turns autonomy ON;
  ceiling 0 (default) ⇒ `Disabled`, never runs.
- **Behavior change to flag:** a user who set a budget expecting manual-only reflect now gets
  autonomous passes within that budget. Intended (it's the product owner's ask), but if a
  "budget for manual only" mode is ever wanted, add an explicit auto-on/off toggle.
- The reflect network call briefly **blocks the worker thread** (so indexing pauses for the few
  seconds of an LLM call). Acceptable for a rare, budgeted background action; FS events queue
  and drain after.
- `ReflectReason::Unchanged` is a new serialized variant (`"unchanged"`) — surfaces in logs as
  `auto-reflect '<pid>' → Unchanged (0 proposal(s), $0.0000)`.
- Cadence is two fixed constants: `LIBRARIAN_IDLE_SETTLE_MS` (3 min quiet → she moves in) and
  `LIBRARIAN_MIN_GAP_MS` (5 min min between rounds) — `ponytail`, lift to user settings to tune.
  Practical feel: she fires ~3 min after you pause editing a project, or promptly after an AI
  session exits — never mid-flow, never on a blind clock.
- Tests: `due_for_round` pure predicate covering idle/boundary/min-gap (lib) +
  `reflect_auto_skips_unchanged_digest` (sandbox, proves a 2nd unchanged pass = 0 requests, $0).
  119 lib + 39 sandbox green. Trigger evolved same-day: `dc01ed9` count v1 → `6873154` time-based
  rounds → event-driven (idle-settle + agent-exit boundary, this revision).
- Still UNVERIFIED live: the real autonomous loop firing in the GUI (the watcher → counter →
  reflect chain) needs a running-app run with a budget set — same live-gap class as ADR-007.
