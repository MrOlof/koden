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

## Phase 3 — validation  (first run 2026-09-03: steps 1, 2, 6 PASS; 3-5 BLOCKED by ADR-025 bug, see below)

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

## 2026-09-03 — first two-device run: PASS 1/2/6, FAIL 3/4/5 → ADR-025 (fix built same day)

Both devices on 0.12.0 (laptop installed over ssh), host ai-server.
- Step 1 (fresh boot both), step 2 (sync dot), step 6 (boot merge: laptop
  reopened with HQ's name + note split intact): PASS.
- Steps 3/4/5 (two devices live): FAIL, root cause = per-space layout LWW
  plus derived writes stamping as edits. HQ created "TESTING TAB" with a
  note split; the laptop materialized the tmux window as a bare tab,
  stamped the whole space 3 s later and pushed; host lost the name and
  the split. Same again with tab "123". Full reconstruction + the fix in
  `decisions/ADR-025-layout-sync-cannot-lose-an-edit.md`.
- Fix: per-tab clocks (identity = oldest terminal pane key / doc id),
  adoption ledger (observers stamp 0, adopters carry the author's clock),
  self-healing push from the live poll, live renames for terminal tabs,
  journal + toast/Undo, F2 manifest reader deleted, tmux loop gated on
  visible not focused. Two-device simulation test (incident replay + 400
  seeded interleavings). Version bumped to 0.12.1 in all four files
  (Cargo.toml finally off 0.11.0). Release = `scripts/release-koden.ps1`
  after Kosta's OK; then RERUN steps 3-8 on 0.12.1.

## 2026-09-03 12:12, second run on 0.12.1: names live PASS; split FAIL; v0.12.2

- Tab created + named on HQ showed up named on the laptop live: PASS.
- Note split into that tab on HQ: never reached the laptop, and the host
  lost the split to the laptop's bare copy again. Cause: pane cwd/colour
  (machine-derived) counted as content, so the laptop re-stamped the tab.
  Fixed (derived fields ignored). The widened fuzz then found that one
  clock per tab still lets a rename erase a split; fixed with separate
  structure and name clocks. Splits now also adopt LIVE when additive.
  All in ADR-025 "Second run". v0.12.2.
- RESCUE for the split created under 0.12.1: on HQ rename the TEST tab
  (anything, then back) BEFORE updating, so HQ re-pushes the split with a
  newer clock; the laptop's duplicate "Notes" tab clears on its next boot.
- Then rerun steps 3-8 on 0.12.2.

## 2026-09-03 13:12, third run on 0.12.2: live split PASS (Kosta: "seems to work now")

Host verified: the new tab (terminal + note + tasks split) is on the host as
a split, last pushed by the laptop after adopting it, structure and name
clocks intact. Steps 1, 2, 3, 4, 6 PASS. Remaining: 5 (rename on either
side, other follows), 7 (delete a space, gone on the other after its boot,
stays gone), 8 (network drop, recover alone). The pre-0.12.2 TEST tab is
still bare on the host with its standalone Notes tab (rescue rename not
done; the note content survives in docs), harmless.
