# ADR-023: Host-authoritative workspace state (one hub, devices are viewports)

Status: Proposed — 2026-09-02 (Kosta requested an architecture overview after
the v0.11.3–v0.11.5 dogfood day; supersedes the manifest-replication approach
of M2.5 F2 as an end-state, keeps it as the migration bridge)

## Context

Kosta's requirement, verbatim in spirit: *one hub for all my sessions — when
I connect to ai-server I get the SAME setup I have on other devices, more
like a remote session; and ai-server can control all devices when it needs.*

The field test that ended the patching approach: tab "Nordomatic Group" on
the desktop holds 1 terminal; on the laptop the same-named tab holds 2
terminals + 1 note pane. All the *content* exists on both (the terminals are
tmux windows, the note reached the docs manifest) — but the *structure* is
different per device, because structure is per-device state that nothing
owns.

### What the current architecture actually is (post v0.11.5)

| Layer | Where truth lives | 1:1? |
|---|---|---|
| Terminal sessions (scrollback, processes) | tmux on the host | yes — the solid part |
| Terminal existence (which leaves exist) | tmux windows (`w-<key>`) | yes, via adoption |
| Tab names | replicated: per-device state + `<key>.json` manifest (custom flag) | eventually, after 3 bug-fix releases |
| Docs content (notes/tasks/boards) | replicated: per-device docsStore + `<key>-docs.json` (whole-doc LWW) | eventually |
| **Window→tab grouping, splits, pane sizes, tab order, active tab** | **per-device only — never synced** | **no — the Nordomatic example** |
| Space list, space names | per-device (`koden-spaces.json`); identity being fixed on `feat/workspace-sync-2026-09-01` | no |

Every bug on 2026-09-02 lived in the replication rows: weak-title stamping
(v0.11.3), pane-notes not covered (v0.11.4), first-push deadlock (v0.11.5),
plus a CI two-draft race for garnish. Replication between N copies is the
fragile version of "one hub" — each new field needs its own sync protocol,
its own conflict rule, and its own field bugs.

## Decision

**For remote (ssh+tmux) Spaces, move the workspace state itself to the host.
One state document per Space on ai-server; every Koden instance renders and
edits THAT, not a local copy.**

Shape (details for the implementing session to refine):

- `~/.koden/spaces/<key>/state.json` — the full structural truth: tabs in
  order, each tab's pane tree (terminal leaves by restore key, doc leaves by
  docId, split dirs/ratios), names, active tab. Written via the existing
  tmp+rename exec channel; read at connect; watched via the existing 15 s
  poll (later: an exec-channel `tail`/inotify push).
- `~/.koden/spaces/<key>/docs/<docId>.json` — doc contents, one file per
  doc (replaces the packed `-docs.json`; per-doc files mean per-doc LWW and
  no 256 KB packing limit).
- Edits: client applies locally (optimistic) and writes through immediately.
  Conflict rule stays last-writer-wins **per file** — layout files are tiny
  and low-contention; the high-stakes state (terminal content) is already in
  tmux and never in these files.
- Local `koden-spaces.json` / docsStore become a **cache** for offline
  rendering of remote Spaces, never a second source of truth. Local and WSL
  Spaces keep today's local persistence — nothing changes for them.
- tmux stays exactly as is. This ADR moves the *description* of the
  workspace, not the terminals.

### Alternatives considered

- **A. Keep replicating, add layout fields to the manifest** — rejected:
  today demonstrated the marginal cost of every replicated field. Layout
  (nested trees, ratios, order) is the worst possible field to replicate
  between live copies.
- **C. Full remote UI** (server renders the app, devices stream it — the
  Cate/code-server/VNC family) — rejected: kills native feel and the local/
  WSL story, adds latency to every keystroke; Koden's whole premise is a
  native shell over remote *state*.
- **D. CRDT engine** (Yjs/automerge over the state) — rejected for now:
  correct-by-construction merging of concurrent edits, but a heavy
  dependency for state whose contention is one human with two screens.
  Revisit only if per-file LWW demonstrably loses user edits.

### Sequencing (coordinates three work streams)

1. **`feat/workspace-sync-2026-09-01` lands first** — space identity
   (identity fold, `syncPathRoot`) is the prerequisite: host state needs a
   stable answer to "which Space is this".
2. **Layout state host-side** (this ADR's core): write-through + render
   from `state.json`; adoption/reconcile shrinks to "render the file".
3. **Docs host-side**: docsStore routes remote-space docs to
   `docs/<docId>.json`; delete the `-docs.json` replication layer
   (v0.11.4/5 code) entirely.
4. **Control plane** ("ai-server can control all devices"): a small
   device-agent channel so the hub can push actions (open space X, update
   Koden, run task) to registered devices. Builds on the same state dir +
   the dashboard. Spec separately (M3 in KODEN-REMOTE.md).

## Consequences

- True structural 1:1: a split made on the laptop is a split on the desktop,
  because there is only one description of the workspace and both render it.
- Deletes code instead of adding: the tab manifest, docs manifest, custom
  flags, seen-sets and pull-gates all fold into "read file, write file".
- New failure mode to design for: editing a remote Space while offline —
  v1 answer: remote Spaces are read-only from cache when unreachable (they
  are mostly useless offline anyway; their terminals are on the host).
- Migration: first client with the new build writes `state.json` from its
  local layout; per-device divergence resolves by the identity-fold rule
  (most-recently-edited wins) once, then never exists again.
- The v0.11.3–5 replication layer stays in place until step 3, then dies.
  No further investment in it beyond critical bugs.
