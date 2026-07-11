# ADR-018 — Autonomous memory curation with snapshot undo

- **Status:** Accepted (Kosta 2026-07-11: approved flipping the Librarian from
  propose-only to fully autonomous curation with undo).
- **Supersedes:** the propose-only clause of ADR-006/ADR-008 ("proposals are
  NEVER auto-applied; approval is the exclusive writer") and ADR-017's "suggests —
  never writes — memory updates" phrasing as applied to the ENGINE. ADR-017's
  chat constraint still holds unchanged: the chat toolset stays read-only (no
  revert/mode/curation tools exposed to the model).

## Decision

The Librarian's curation engine applies its own memory proposals. A persisted
**curation mode** on the `brain_librarian` singleton selects between:

- **`autonomous` (the default):** every proposal enqueued on the worker (reflect
  finish, manual reflect, doctor, curate/contradiction — including the boot
  doctor seed) is APPLIED immediately by a worker-side sweep
  (`worker::auto_apply_pending` → the existing `SqliteIndex::apply_proposal`,
  `auto_applied = 1`). Nothing waits in an inbox; the review surface becomes a
  revertible **"Memory changes"** feed.
- **`review`:** the pre-ADR-018 behavior, byte-for-byte — proposals park as
  `pending` and a human approves/rejects in the inbox.

Every apply — in BOTH modes, human-approved or autonomous — snapshots its
**inverse into the proposal row first**, and a new `brain_revert_proposal`
command restores it.

## Why now

- The proposal pipeline earned trust: the live gauntlet's proposal-stream work
  landed recall/precision at 1.00 with the three-layer dedup (reject-signature,
  semantic near-dupe Jaccard, exact PK) plus the D2 actionability gate.
- The substrate was already undo-shaped: a single-writer worker, idempotent
  apply (`status != 'pending'` → no-op), crash-safe atomic file writes, and the
  canonical-tail journal that replays human decisions through header-destroying
  corruption. Snapshot-undo composes from existing parts.
- Review-inbox friction was the residual cost: with proposal quality measured
  at 1.00, gating every archive/create on a human click was pure latency. Undo
  is a cheaper safety mechanism than approval when precision is this high.

## Undo design (snapshot-before-apply)

`apply_proposal` records, BEFORE `materialize` writes anything:

| action | inverse recorded | revert does |
|---|---|---|
| create | `undo_created_id` (the minted slug — `materialize` now returns it) | delete `<id>.md` |
| update | `undo_prior_path` + `undo_prior_bytes` (FULL prior file) | restore bytes verbatim |
| archive | `undo_prior_path` + `undo_prior_bytes` | restore bytes verbatim (a prior file may have NO `status:` key — only full bytes restore that; there is no remove-key edit) |
| supersede | `undo_created_id` + target's `undo_prior_path`/`undo_prior_bytes` | delete the minted note + restore the target |

Revert (`SqliteIndex::revert_proposal`, worker-side via
`BrainEvent::RevertProposal`) is idempotent (`applied` → acts; anything else →
no-op), re-scans the notes table from disk, flips the row to **`reverted`**
(`reverted_ms` stamped), journals the flip, and **persists the
reject-signature** — an undone change must not be re-proposed and re-applied
next round (the undo-fights-the-librarian loop). Rows applied before ADR-018
carry no snapshot: revert refuses with a clear soft error and the UI lists them
as non-revertible.

Status vocabulary grows by exactly one value (`reverted`); the resolved-dedup
lookback and the "already proposed" advisory include it. `pending` semantics are
untouched, so `remove_note`'s cascade, the curate pending-dedup gate, and the
inbox query all behave as before.

## Schema / migration

Additive-canonical columns (the `brain_librarian_pin` idiom, extended to
columns): the base DDL carries them for fresh stores, and
`migrate::ensure_additive_columns` issues a guarded
`ALTER TABLE … ADD COLUMN` per missing column on every open. **No
SCHEMA_VERSION bump** — deliberately: bumps only drop/rebuild DERIVED tables
(these columns live on CANONICAL `proposals`/`brain_librarian`, which a bump
never touches) and would rotate every gist cache key for nothing. A lockstep
test (`additive_columns_match_the_ddl`) pins DDL ↔ ALTER-list equivalence.

New columns: `proposals.applied_ms/reverted_ms/auto_applied/undo_created_id/
undo_prior_path/undo_prior_bytes`, `brain_librarian.curation_mode`
(default `'autonomous'`). All journal-compatible: journal lines are full-row
column maps (`SELECT *`), and old lines replay with DDL defaults filling the
new columns.

## Self-feeding-loop guard

`build_digest`'s invariant was "an ENQUEUE never moves the delta-gate hash" —
but an APPLY does (it writes `.koden-memory` files, which are the digest
corpus). Without a guard every autonomous round would chain a paid round on the
Librarian's own writes. Fix: after any sweep that applied ≥1 proposal (and
after a revert), the worker re-pins the post-apply corpus digest
(`reflect::pin_corpus_digest`), so the next round short-circuits Unchanged/$0.
Trade-off accepted: a user edit landing in the same window is skipped for that
round and picked up on its next change. Pinned by test
(`post_apply_digest_pin_keeps_the_delta_gate_at_zero`, with an unpinned
control proving the pin is load-bearing).

## Surfaces

- **Commands:** `brain_revert_proposal` (async, worker reply like resolve),
  `brain_memory_changes` (recent applied/reverted rows + `revertible` flag),
  `brain_set_curation_mode`; `LibrarianStatus` carries `curation_mode`.
- **BrainPane:** autonomous → "Memory changes" feed (relative time, action,
  auto badge, Revert button with the same optimistic-guard pattern as resolve;
  reverted rows shown dimmed); review → the classic inbox, unchanged.
- **Settings › Librarian:** a Curation radio (Autonomous / Review first),
  mode-aware descriptions; the activity panel's "N pending" line becomes
  "applies changes autonomously — see Memory changes" in autonomous mode
  (pending reads ~0 there by design).
- **Onboarding + chat-tool copy** rewritten to be mode-honest; chat tools
  remain read-only (`brain_proposals` now also reports recent changes).

## Ceilings / follow-ups (known, deliberate)

1. The sweep re-scans the memory folder per applied proposal (correct + simple;
   batches are small — ≤ MAX_PROPOSALS + doctor findings). Batch-scan once if a
   profile ever shows it.
2. `undo_prior_bytes` stores whole prior notes in the proposals table and its
   journal lines; fine at memory-note sizes, revisit if notes grow huge.
3. Review-mode applies are also snapshot-undoable, but the review-mode UI keeps
   the classic inbox only (no changes feed) — deliberate scope cut; flip the
   `autonomous` gate in `MemoryView` if reverting should surface there too.
4. In autonomous mode a mid-flight `RemoveProject` skips the sweep (root gone);
   enqueued-then-pruned rows are already handled by the existing prune path.
