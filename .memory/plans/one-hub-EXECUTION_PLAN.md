# One-hub execution plan (ADR-023 + ADR-024) — status ledger

Written 2026-09-02 under Kosta's full-delivery mandate ("plan perfectly,
test, validate, full rework, loop until it works 100%"). This is the living
ledger: phases, what shipped, what Kosta verifies at a keyboard, what
remains. Update it as phases land.

## Phase 0 — recon + unification  ✅ DONE (2026-09-02)

- Full persistence-architecture recon (Explore agent): every store file,
  writer, cadence, identity, and the three overlapping sync systems mapped.
- workspace-sync branch backed up to the homelab mirror, merged into
  released main (`1432606`) — zero textual conflicts, 702/702 tests green
  on the merged tree.

## Phase 1 — teardown of the replication layer  ✅ DONE

- `-docs.json` manifest sync deleted (remoteDocs.ts + App effects +
  `applyRemote*`); the engine's docs domain is the only docs mechanism.
- Kept: title manifest (live titles), tmux adoption (terminal existence),
  clean-exit, fill-character.
- Host hygiene: stale `<key>-docs.json` files on ai-server removed at
  deploy.

## Phase 2 — liveness (ADR-024)  ✅ DONE (code)

- Fast visible-window cadences; `notifyWsChanged` push signal from
  `saveState`; `liveAdopt.ts` additive adopter + engine `liveWsAdopt`
  (never advances ws lastGen — ADR-023 boot invariants untouched).
- Defaults: sync ON, host ai-server.
- Suite: 701/701 + tsc + biome clean.

## Phase 3 — validation

Automated (done): liveAdopt planner suite (idempotence, pane-existence,
rename discipline); engine merge/chunk/pathMap suites from the adversarial
review; whole-suite green.

**Two-device GUI verification (Kosta, next session at a keyboard) — the
release is not "verified" until this passes:**

1. Update both machines to v0.12.0, restart BOTH once more (identity fold +
   first ws sync happen at boot).
2. Statusbar shows the sync segment "synced Xs ago" on both.
3. Desktop: type in the note pane → laptop shows the content within ~15 s
   (note appears as a tab there if it was pane-only).
4. Laptop: create a new note tab in a shared space, name it → desktop grows
   the same tab within ~15 s.
5. Rename a doc tab on one device → other follows without restart.
6. Close Koden on the laptop, reopen → identical space list, names, tab
   layouts incl. splits (boot merge). The Nordomatic 1-vs-2-terminals case
   specifically: both devices show the same grouping after both rebooted.
7. Delete a space on one device → gone on the other after its next boot,
   and it stays gone.
8. Kill the network mid-session → statusbar flips offline, app stays
   usable; reconnect → recovers alone.

## Phase 4 — remaining work (specced, not started)

- **Title-manifest consolidation**: fold live terminal-title sync into the
  ws domain, delete the F2 title manifest (last replication remnant).
- **Push channel**: second hardcoded events file on the host + widened tail
  (M2.8 pattern) to replace polling; drops latency to ~3 s and handshake
  load to ~zero.
- **Split ratios + tab order**: not persisted at all today (SerializedNode
  has no ratio field) — net-new schema, then they sync for free via ws.
- **Host-authoritative end-state** (ADR-024 north star): revisit if LWW
  losses show up in practice.
- **Control plane** (KODEN-REMOTE.md M3): hub pushes actions to devices.
- **Sub-tabs / space groups** (Kosta's notes, 2026-09-02): separate feature
  track, design against the synced schema from day one.
