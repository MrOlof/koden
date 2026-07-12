# ADR-021: Register projects on first use + boot re-discovery + agent memory guidance

Status: Accepted — Kosta, 2026-07-12

Acceptance shape: "new projects must end in the proper flow."

## Context

The Brain only knew projects that were explicitly introduced: the wizard's
`brain_set_workspace` child scan, a manual `brain_add_project`, or the boot
seed. A repo cloned into the workspace yesterday — or scaffolded five minutes
ago in a terminal — was invisible: a session running there resolved
`cwd → None` and every ingest leg silently dropped its events. No activity
trail, no index, no gist artifact, no Librarian. The user's mental model
("everything under my workspace root is a project") and the registry's model
diverged the moment a new folder appeared, and nothing ever converged them
short of re-running the wizard.

Second gap: a genuinely fresh project's gist said "No memory notes in this
project." and stopped there — an agent had no idea the memory system existed
or where to write, so the loop that fills `.koden-memory/` never started.

## Decision

### Trigger matrix

New projects reach the registry through THREE converging paths, all sharing
the same qualification rule and the same registration path:

| Trigger | When | Mechanism |
|---|---|---|
| First use (signal) | An agent starts/exits in an unregistered dir | `handle_agent` resolution miss → register → retry |
| First use (turn) | A prompt is submitted in an unregistered dir | `resolve_pty_project` miss → register → retry |
| Boot re-discovery | Worker boot, workspace root set | `register_workspace_children` before the watcher arms |
| Wizard | `brain_set_workspace` | the same `register_workspace_children` loop |

- **Qualification** — `worker::qualifies_as_project(dir)`: a non-ignored dir
  that is a git repo or carries a manifest — the exact child-marker test
  `discover_workspace_projects` always used, extracted to its single-dir form
  so the walk and the discovery scan cannot drift.
- **Registration** — the exact `brain_add_project` path: `registry.add_root`
  (idempotent by stable id), then a `Rescan{None}` enqueue, which indexes the
  project, re-arms the watcher over it, persists `workspace.json`, and emits
  its first gist artifact once indexing completes. INFO log
  `brain: registered new project X on first use`, plus ONE coalesced
  `koden:brain-activity` event `kind: "registered"` (count 1) → sonner toast
  "Koden Brain — <project> registered — indexing", pref-gated on
  `memoryNotifications` like every ADR-020 notification.

### The nearest-ancestor-under-root rule

`first_use_candidate(workspace_root, cwd)`: walking UP from `cwd`, the FIRST
dir that qualifies STRICTLY below the workspace root is the candidate — a
nested-git-in-git cwd picks the INNER repo (the most specific project
containing the session), mirroring the registry's longest-prefix resolve. A
qualifying dir inside an ignored subtree is discarded when the walk crosses
the ignored component (`node_modules/<dep>/package.json` is a marker on every
npm dependency; the dep must attribute to the real project above it).
Comparisons use the registry norms (`to_canon` + Windows-only case fold), so
an OSC 7 `c:\ws\repo` cwd matches a stored `C:/ws` root.

### Retry — the triggering event is never lost

After registration the ORIGINAL resolution is retried
(`resolve_or_register_project` returns the re-resolved project), so the very
signal/turn that triggered registration lands in the new project's activity
trail: the session-boundary row, the first turn, the resume journal entry all
attribute correctly from event one.

### The one-vanilla-prompt ceiling

The FIRST prompt in a brand-new project runs before its index and gist
artifact exist — that turn is vanilla (no injection). The registration that
very prompt triggered enqueues the reconcile; by the time it completes the
artifact is on disk, so injection catches from prompt 2 (or from the next
turn after indexing finishes on a large repo). Accepted: pre-indexing a
project before its first use would require registering things the user never
touched.

### The agent-writable memory loop

A fresh project's gist known-unknowns block now carries one agent-facing line
after "No memory notes in this project.":

    - Memory lives in .koden-memory/*.md (markdown + YAML frontmatter). Leave concise notes there for future sessions.

The agent writes a note → the watcher's note re-scan ingests it (the existing
ADR-019 note-file path) → the next gist renders it. Byte-identity holds with
NO new key input: note FILES are indexed, so the zero-notes state (and its
flip) is already covered by the content fingerprint — the guidance line
derives only from key-covered state (`notes.is_empty()`), and the first note
rotates the key exactly once. Pinned by
`fresh_project_gist_carries_memory_guidance_until_first_note`.

### Boot re-discovery

At worker boot (step 5b, after the registry bootstrap and BEFORE the watcher
arms + warm walk), if a workspace root is set, `register_workspace_children`
re-runs the child scan and registers anything new — a repo cloned while Koden
was closed is watched + indexed + doctored + artifact-emitted by the normal
boot flow, no wizard revisit. Idempotent (add_root by stable id); logs the
count when > 0 and persists `workspace.json` only then.

### Guards

- **Never the workspace root itself** — the candidate walk is strictly below
  the root (exclusive break), and the root cannot slip in via any spelling
  (case-folded canonical compare).
- **Never inside an existing project** — impossible by construction: the
  first-use path only runs when `resolve(cwd)` returned `None`, and a
  candidate that equaled a registered root would have resolved.
- **Debounce** — structural: only the single worker thread registers, and a
  started signal + first turn arriving together serialize on it; the first
  registers, the second short-circuits on the resolve (one registration, one
  toast, one rescan). `add_root` is idempotent by stable id regardless.
- **Sanity gate** — the candidate passes the same `is_sane_root` gate as
  `brain_add_project` (no drive roots, no bare home dir).
- **Removal tombstones** — `brain_remove_project` tombstones the stable id in
  `workspace.json` (`removed: [...]`). Every AUTO registration path — boot
  re-discovery, first-use (signal/turn), and the `brain_set_workspace` child
  scan — goes through `registry.add_root_discovered`, which skips tombstoned
  ids, so a removed project whose dir still qualifies on disk (the normal
  case) is NOT silently re-registered at the next launch or next session in
  that dir. The explicit `brain_add_project` path (`registry.add_root`) clears
  the tombstone — re-adding is the documented opt-back-in. The command's
  rollback (`registry.restore` after a failed prune enqueue) clears the
  tombstone too, since that removal never happened. Without this, removal was
  session-scoped for workspace children: boot 5b resurrected it invisibly
  (INFO log only, no toast), undoing the user's confirmed action.

## Consequences

- A `git clone` into the workspace followed by `claude` in that dir is fully
  adopted with zero ceremony: registered on the first signal, indexed,
  watched, trail-recorded from event one, injected from prompt 2.
- The wizard remains the only path that SETS the workspace root; first-use
  and boot re-discovery only ever add children under it. No root configured →
  behavior unchanged (unresolvable events still drop).
- `brain_set_workspace` now shares `register_workspace_children` — its return
  (all qualifying children) and enqueue behavior are unchanged.
- One more event kind rides the ADR-020 notification surface
  (`registered`) — union updated in `bindings.ts`, toast titled "Koden
  Brain", status-bar segment renders the verb without a count suffix.
- Ceilings, accepted:
  - The one-vanilla-prompt window above.
  - A qualifying dir OUTSIDE the workspace root still never registers itself
    (deliberate — the root is the user's consent boundary; `brain_add_project`
    covers intentional outliers).
  - Boot re-discovery scans immediate children only (the
    `discover_workspace_projects` contract) — deeper nesting registers on
    first use instead.
