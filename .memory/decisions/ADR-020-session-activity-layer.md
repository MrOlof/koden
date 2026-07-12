# ADR-020: Session activity layer + hands-off freshness + ambient notifications

Status: Accepted — Kosta, 2026-07-12

Acceptance shape: "sessions feel up to date; the Librarian catches things; a
crash leaves a trail; no manual reindexing."

## Context

The Brain saw FILES but not SESSIONS. What a user actually did — the prompts
they submitted, which sessions ran where, when an agent started and exited —
never reached Rust: the `UserPromptSubmit` hook's bus line
(`~/.koden/director-bus.jsonl`) was consumed only by the frontend
`AgentBusBridge` for the per-pane Inputs list, then forgotten. Consequences:

- The Librarian reflected on notes it already had, blind to what the sessions
  were actually about.
- A crash left no trace of the work in flight (the P4 resume journal keys
  panes, not activity).
- A session's writes could sit unindexed until the watcher happened to fire or
  the user rescanned by hand.
- The Librarian's autonomous applies were invisible unless you opened the
  Brain pane and looked.

## Decision

A `brain_activity` trail owned by the single writer, folded into both existing
context surfaces via their own established invariants, plus coalesced ambient
notifications.

### The trail

- `brain_activity` (schema.rs): `seq / project_id / ts_ms / session_pty /
  kind(turn|files|start|end) / payload_redacted`, index `(project_id, ts_ms)`.
  New-table idiom = the `brain_librarian_pin` precedent: `CREATE TABLE IF NOT
  EXISTS` in the base DDL, classified CANONICAL (observed history is not
  re-derivable from a disk walk), NO `SCHEMA_VERSION` bump.
- Deliberately NOT journaled (`journal::JOURNALED_TABLES` unchanged): the
  sidecar is an 8MB-capped, low-frequency backup for decisions + spend; the
  trail is high-frequency and loss-tolerant — losing it costs context, never
  correctness.
- **Redacted at ingest** (`secrets::redact`, the same pure gate that guards
  file content and reflect messages): prompt text passes
  `worker::clean_turn_text` — drop empty / sub-2-char / slash-command-only
  turns, truncate to 1500 chars on a char boundary, then redact — BEFORE the
  row is written. Nothing raw ever lands.
- Crash-safe by construction: every row is one incremental INSERT on the
  worker thread as events arrive; an app crash leaves the `start` row (and
  every turn up to the crash) as the trail.
- Retention on Tick: per-project cap 500 rows + 14-day TTL
  (`prune_activity`; MAX_TURNS / RESUME_TTL_DAYS precedents). A prune that
  drops rows refreshes that project's gist artifact so it never quotes rows
  the store no longer holds.

### Ingest legs

1. **Turns** — new `BrainEvent::Turn` + sync `brain_record_turn` command
   (enqueue-only, the pure in-memory command class). Fired by `AgentBusBridge`
   beside its existing `addTurnForLeaf` fan-out, so the ONE existing hook
   channel feeds both. pty → cwd → project resolution is the `handle_agent`
   chain (live `PtyState::session_cwd`, falling back to the LiveSession's
   remembered cwd).
2. **Files** — the existing `Fs` handler fans real indexed changes
   (`stats.indexed > 0`) into ONE coarse `files` row per project per minute
   (debounced; payload = sorted/deduped/capped JSON array of rel paths).
   Attribution is deliberately project-global last-touch, not per-session —
   see ceilings.
3. **Boundaries** — `handle_agent` writes `start`/`end` rows for
   `started`/`exited` (status-only kinds skipped).

### Hands-off freshness

An `exited` signal now ALSO enqueues a targeted `Rescan{Some(project)}`
(`enqueue_exit_reconcile`): the session may have written files no watcher
event landed for (editor buffers, git plumbing). The existing Rescan arm
re-indexes and then refreshes the gist artifact (ADR-019 emitter, byte-compare
intact) — so by the next turn in any other live session, the exited session's
work is indexed and visible, with no manual rescan. The Librarian boundary
flag (settle-now) is unchanged on top.

### The two invariant preservations (the load-bearing part)

1. **Gist**: a `## Recent activity` section renders after `## Memory`
   (trimmed first under budget, pushed as one block), derived off the SAME
   pinned snapshot as every other gist input. Its digest is folded into the
   cache key exactly like the overdue-note set (ADR-011 precedent, gist/mod.rs
   `overdue_digest`): a SET digest, never a clock. Damping: day-bucketed,
   COUNT-FREE lines — per UTC day (from stored `ts_ms`, no wall-clock read)
   the sorted capped set of session agents + files touched; turns mark day
   presence only. So a turn that adds nothing new to the day's sets renders
   byte-identical gist bytes under an identical key, and a genuine set change
   (new day / agent / file entry) rotates the key exactly once. The ADR-019
   artifact byte-compare then keeps unchanged artifacts untouched — prompt
   caches survive session chatter. Pinned by tests
   (`gist_key_stable_over_turns_and_rotates_once_on_new_file_entry`).
2. **Reflect**: the trail reaches the model via the SEND-ONLY split —
   `append_recent_activity` mirrors `append_already_proposed`
   (reflect/mod.rs): appended to the sent user message at both call sites
   (sync + offloaded prepare), NEVER folded into `build_digest`, whose hash is
   the autonomous delta gate. Ingesting 50 turns provably leaves the digest
   hash byte-identical (`turn_ingest_never_moves_the_delta_gate_hash_but_rides_the_send`)
   — session activity can never buy a paid round by itself; it only enriches
   rounds that real memory changes already earned. (The optional day-bucket
   pin fold was considered and skipped: the boundary-triggered rescan already
   re-fires rounds at the moments that matter.)

### Notifications (owner-approved design)

- Worker → frontend Tauri event `koden:brain-activity`, payload
  `{project, project_name, kind: applied|reflected|reverted, count,
  spent_usd}` — COALESCED at the seams: one event per `auto_apply_sweep`
  batch, per completed reflect round (reason Ok, manual + offloaded), per
  revert. Never per-proposal (`build_activity_event` is the tested seam;
  no-op applied/reverted batches are suppressed, a completed reflect emits
  even at 0 proposals — it spent a call).
- `BrainActivityBridge` (always-mounted, the AgentBusBridge pattern): terse
  sonner toast titled "Librarian" ("<project>: N memory update(s)") with a
  View action, plus a `NotificationBell` entry (`source: "brain"`, `kind:
  "memory"`) so missed toasts stay reviewable. View / bell rows land on the
  Brain pane's MEMORY view via a one-shot view request
  (`requestBrainView("memory")` + the existing `openOrchestrationTab("brain")`
  mechanism).
- Gated on the new `memoryNotifications` preference — DEFAULT ON (an
  autonomous Librarian must be visible by default; the toggle in the Librarian
  settings tab is the opt-out). Full settings-store pattern: DEFAULT_PREFERENCES
  + identity key map + cross-window propagation via `writePref`.
- Status bar: `BrainActivitySegment`, an ambient mono dot+label near the
  usage-guard pill — muted idle, pulses while indexing (existing
  `useBrainStatus` polling), brief primary flash on an activity event, hover =
  "reflect 3m ago · $0.0021 · 2 applied" + index state. ALWAYS on (ambient
  chrome, not pref-gated). Svart tokens only (primary / muted-foreground /
  `--terminal-ansi-yellow`).

## Consequences

- The gist now answers "what was I doing here" — cold sessions land with the
  last days' sessions and touched files in context, in any terminal (ADR-019
  channel unchanged).
- Reflect rounds see the session trail without paying for it; the delta gate's
  $0 steady state is untouched by any amount of session chatter.
- A crash leaves a queryable trail (start row + turns + files) alongside the
  P4 resume cards.
- One-time gist cache-key rotation on first post-upgrade launch: the key
  FORMULA gained the activity-digest component (same one-time
  agent-prompt-cache miss as a schema bump, without dropping tables —
  documented in schema.rs).
- Ceilings, accepted:
  - Files-touched attribution is COARSE — project-global last-touch, debounced
    per minute, not per-session (`session_pty` is NULL on `files` rows).
    Precise per-session attribution would need write-provenance the watcher
    doesn't have.
  - The gist injection channel inherits ADR-019's bounded 12-level hook walk
    and its CC-version stdout caveat.
  - Mid-day TTL prune can briefly leave a stale artifact between Tick and the
    next event; the prune-triggered refresh bounds this to one Tick.
  - `exited` fires per armed-agent command end (OSC 133;D), so a rapid
    start/stop loop enqueues several targeted rescans; the blake3 hash-skip
    makes converged rescans cheap no-ops.
- Follow-up candidates: per-session file attribution once a provenance signal
  exists; surfacing the trail in the Brain pane's memory view; a
  reflect-in-progress pulse on the status segment (needs a dispatch-time
  event; today the segment pulses on warming and flashes on completion).
