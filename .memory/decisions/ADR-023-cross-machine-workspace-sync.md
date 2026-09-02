# ADR-023 — Cross-machine workspace sync (docs + spaces + tab layout)

Status: Accepted (Kosta 2026-09-01: scope "everything incl. layout", build overnight)
Date: 2026-09-01
Builds on: ADR-022 (ssh plumbing, space manifests), WP3 (serialize/restore), ADR-017 (docs store write discipline)

## Problem

Notes/tasks/boards (`koden-workspace-docs.json`), the spaces list and per-space
tab layouts (`koden-spaces.json`) are per-machine Tauri app-data. Kosta runs
Koden on 3+ machines (HQ Windows, laptop, ai-server-adjacent). Notes written at
home don't exist on the laptop (live incident 2026-09-01: notes recovered from
HQ over ssh by hand). Terminals already roam via ssh Spaces + tmux; workspace
docs and layout do not.

## Decision

A frontend-only sync engine (`src/modules/sync/`) with **ai-server as the
canonical home** (always-on, on the tailnet), transported over the **existing
ssh commands — zero new Rust**:

- `ssh_write_space_manifest` / `ssh_read_space_manifest` (atomic tmp+mv,
  stdin-piped so no shell quoting, cross-platform incl. Windows clients,
  16 KB/manifest, key `[A-Za-z0-9_-]{1,60}`) carry **chunked envelopes**
  under reserved keys `sync-<domain>` (index) + `sync-<domain>-p<i>` (parts)
  in `~/.koden/spaces/` on the sync host. No collision with real Space
  manifests (those are `p<fnv36>…` tmux keys).
- Index manifest `{v, gen, of, bytes, hash, deviceId, at}`; unchanged `gen`
  makes a poll cost exactly one ssh handshake (no ControlMaster exists).
  Parts carry `gen`; reader validates gen-uniformity + fnv1a hash, retries
  once. Envelope is base64 of the JSON (stable chunk sizes, no escaping
  blowup), sliced ≤ ~10.8 KB/part.
- `ssh_home(host)` is the liveness probe; failures → offline state with
  backoff, never errors into the UI.

### Domains and merge rules

**docs** (notes/boards/tasks): bidirectional, live. Per-entry LWW on the
existing `updatedAt` (the store already stamps every write). Applied through a
new `mergeRemoteDocs` store action (never raw file IO — ADR-017), which also
persists adopted winners. No doc-deletion tombstones: the store has no delete
actions today.

**ws** (spaces + layouts): push continuously, **pull at boot only** — swapping
a live layout mid-session is user-hostile; the real usage pattern is
close-at-home/open-at-work. Envelope: `{spaces, states, stateMeta, tombstones}`.

- Spaces merge per-id. New field `SpaceMeta.contentUpdatedAt` stamped by
  create/rename/setColor/setSshTmux — NOT by setActive, because `updatedAt`
  is deliberately an LRU clock (launcher Continue list) and would lose a
  rename to a mere visit. Winner = greater `contentUpdatedAt ?? 0`, tiebreak
  `updatedAt`, tiebreak local. Space ORDER stays local; unseen remote spaces
  append (ceiling: reorder doesn't sync in v1).
- Deletions: tombstones `{spaceId: deletedAt}` recorded on `useSpaces.remove`
  (via `sync/lib/syncSignals` — leaf module, no import cycle), merged as max,
  win over a space unless it was recreated (`createdAt > deletedAt`) or
  content-edited after (`contentUpdatedAt > deletedAt`). LRU `updatedAt`
  cannot resurrect a deleted space.
- Layout states (`state:<id>`) had NO timestamp — `saveState` now also writes
  `stateMeta:<id> = {at}` in the same store file (prefix cannot collide:
  `"stateMeta:x".startsWith("state:")` is false). Snapshot LWW per space on
  `stateMeta.at`; a side missing meta loses to a side that has it; both
  missing → local wins (pre-sync data).
- Worktree Spaces (`worktree` set) are machine-local by nature (absolute
  checkout paths) — excluded from push and from adoption both ways.
- `activeId` and scrollback snapshots (512 KiB/leaf pixels) never sync.

### Path portability

Leaf `cwd`, `SpaceMeta.root`, and editor/markdown tab paths are absolute and
machine-specific. On push, a configured local tree root (`syncPathRoot` pref,
e.g. `C:/Users/Snorlax/Snorlax` vs `/home/snorlax/Snorlax`) is rewritten to a
wire token `~SYNCROOT~`; on pull the token becomes the local root. Compare in
forward-slash form. Empty pref = no rewrite (sync still works; foreign paths
hydrate as cold tabs and fall back to the default folder on warm —
`workspaceAuthorize` failures are already `allSettled`-tolerated at boot).

### Scheduling

- Boot: `useSpacesBoot` gains one seam — after `loadAll()`+prefs, before
  `hydrate()`/`replaceTabs()` (the exact safe merge point; recon 2026-09-01),
  `bootPullWorkspace()` merges remote ws state and persists the merged result,
  bounded by an 8 s race so an offline boot costs nothing perceptible.
- Docs: pull on engine mount + window focus (30 s min-gap) + every 5 min;
  push debounced 15 s after any docs change, flushed on blur/hidden.
- ws push: 60 s poll of `loadAll()` signature + blur/hidden flush.
- Push is always merge-then-write (read remote gen first; if moved, pull+merge
  before writing gen+1). No server locking: a lost race costs one extra cycle,
  never data — per-entry LWW re-converges because losers re-push their copy.

### Consistency model (revised after the 2026-09-01 adversarial review)

The review confirmed 10 defects in the first cut; the fixes hardened the
model into these invariants:

- **docs `lastGen`** = "fully merged into the live store". Docs sync is gated
  on `hydrateDocs()` (a pre-hydration pull would let older remote entries
  shadow newer local disk state), and `docsDirty` is restored when a push
  throws so edits are never stranded.
- **ws `lastGen` is BOOT-ONLY.** Mid-session pushes fold newer remote layout
  into the pushed envelope but never advance `lastGen` and never touch local
  state — the next boot still pulls and adopts. (Marking it consumed both
  skipped adoption and let a later skip-merge push erase remote changes.)
- **`stateMeta.at` stamps only on real layout-content change**: `saveState`
  diffs serialized tabs against disk and excludes `activeTabIndex`, so tab
  switches and boot's seeded first flush can't make layout merge degrade to
  "last machine booted wins" — the layout twin of `contentUpdatedAt`.
- **Boot pull is cancellable and adaptive**: past the budget the merge is
  abandoned (no late writes from a pre-boot snapshot behind the booted UI),
  and after a failed boot the next boot waits 2 s, not 8 s.
- **Torn manifests self-heal**: a writer crashing between part writes and the
  index write leaves gen-mixed manifests; readers detect this (TORN) and
  schedule a push, which rewrites parts+index consistently.
- **Tombstones merge-write** (max-clock per id, 90-day TTL prune) — a delete
  recorded between a cycle's snapshot and its write survives. Ceiling: a
  machine offline > 90 days can resurrect a deleted space.
- **No boot chatter**: the docs subscription attaches after hydration and the
  ws push signature persists in `koden-sync-meta.json`, so an idle boot pushes
  nothing and peers aren't forced into re-pulls.
- Enabled-but-invalid `syncHost` surfaces as an error (statusbar + inline in
  Settings), never as a silent "disabled".

### Identity fold (added 2026-09-02, live incident)

Spaces created independently per device for the same real thing (each "Open
folder as Space" of `~/Snorlax`, each connect to the same ssh host+path) hold
different random ids, so id-keyed union produced side-by-side duplicates and
per-machine derived names ("Snorlax" from the folder basename) survived next
to the user's designated name. `mergeWorkspace` now folds spaces sharing a
machine-independent identity — `ssh:host:path`, `wsl:distro:root`, or
`local:root` after the wire-token rewrite (null roots and worktrees never
fold; Windows paths compared case-insensitively, separators normalized).
Survivor = oldest `createdAt` (tiebreak: smaller id) so every machine
deterministically picks the same one; content = clock winner across the
group; the best-stamped layout follows the survivor; the fold keeps the
user's local list position; `idRemap` lets the boot path re-point `activeId`.
Ceiling: local spaces fold only when both devices set `syncPathRoot` (roots
must be comparable); exact-tie clocks resolve slightly order-dependently.

### Identity, settings, surface

- `koden-sync-meta.json` (LazyStore): minted `deviceId`, last pulled/pushed
  `gen` per domain, pending tombstones. First per-machine identity in the app.
- Prefs (6-step recipe): `syncEnabled` (default **false** — opt-in; this is a
  fork feature that ssh-es to a host), `syncHost` (default `ai-server`),
  `syncPathRoot` (default empty). Settings → General, own "Sync" group.
- Statusbar segment (BrainActivitySegment pattern): dot + relative "synced Xm
  ago" / offline / error; click = sync now.
- Host string validated frontend-side (`[A-Za-z0-9._@:-]{1,255}`, no leading
  `-`) before use; it only ever reaches the Rust-validated `host` arg of
  existing commands (is_safe_ssh_host re-checks).

## Consequences / ceilings

- Clock-dependent LWW: machines are NTP-synced; a badly skewed clock can win
  merges it shouldn't. Documented assumption, not enforced.
- Layout pulls only at boot; two machines running simultaneously converge
  layouts on next restart, docs converge live.
- Space reorder and `activeId` don't sync. Scrollback doesn't sync.
- 256 KB output cap of shell paths is irrelevant (manifest transport), but
  the chunk protocol pays one handshake per 10.8 KB on change; docs files are
  expected < 100 KB for a long time.
- `~/.koden/spaces/sync-*` on the host is a pragmatic namespace; a proper
  `~/.koden/sync/` path needs a small Rust command — future follow-up, wire
  format already versioned (`v: 1`) for the move.
- Settings do not sync (machine-specific; separate decision if ever).
- GUI verification pending per house convention; cargo untouched (no Rust
  changes), so the Rust sweep is not required for this branch.
