# ADR-025: Layout sync that cannot lose an edit: per-tab clocks, origin-carried adoption, self-healing push, sync journal

Status: Accepted, 2026-09-03 (Kosta, first two-device GUI run of v0.12.0:
"we need a permanent fix, rework whatever we need to make this real,
people are gonna use this"). Implemented the same day as v0.12.1; the
first live run of v0.12.1 found two more holes of the same class within
the hour, fixed and widened in v0.12.2 (see "Second run" below).
Builds on: ADR-023 (sync engine, merge rules), ADR-024 (liveness, additive
live adoption). Supersedes the per-space layout LWW of ADR-023 and the F2
title-manifest reader kept alive by ADR-024.

## Context

First live two-device run (HQ + laptop, both on 0.12.0, host ai-server),
2026-09-03 11:09–11:20, reconstructed from the host envelopes, both
devices' `koden-spaces.json` / `koden-sync-meta.json`, and the code:

1. HQ creates a terminal tab, renames it "TESTING TAB", splits a note into
   it. HQ stamps the ai-server space `11:09:54` and pushes.
2. The laptop's tmux adoption loop (App.syncRemoteSpace) materializes the
   new tmux window as a bare tab. That is a layout change, so `saveState`
   stamps the SAME space `11:09:57` and pushes at `11:10:02` (merge-then-
   write: laptop stamp newer → laptop copy wins the whole space).
3. Host now holds a bare tab: no name, no note pane. The laptop's live doc
   adoption reads the host and finds no note to create. HQ's next boot
   would adopt the bare copy and delete its own tab. Rescue = rename on HQ
   to re-stamp, then reboot the laptop.
4. Repeat with tab "123": identical loss (`11:19:19` vs `11:20:12`). The
   F2 title manifest, the only live path for terminal-tab names, never
   carried "123" either: its writer is fire-and-forget (a failed write
   sets the "already pushed" signature; a 2 s debounce restarts on every
   tabs change) and its reader runs only while the window `hasFocus()`,
   and the other device is by definition the unfocused one.

Root causes, in order of blast radius:

- **R1: per-space whole-layout LWW.** One stamp per space; the last writer
  replaces every tab on the other device. A rename on one device and a
  materialization on the other are not a conflict, yet one of them loses
  everything the other did.
- **R2: derived writes stamp as user edits.** tmux materialization and
  live doc adoption go through the same persistence path as a user rename
  and mint `Date.now()`. A device that merely OBSERVED a change beats the
  device that MADE it.
- **H1: lost update after a push race.** `syncWorkspace` pushes only when
  the local signature differs from the LAST PUSHED local signature. A
  device whose push was overwritten sees no local change and never
  re-pushes; the host keeps the loser's absence until the next local edit.
- **G1: terminal tab names are not live.** Layout adoption is boot-only
  (ADR-023) and the F2 manifest is unreliable (above). Users read this as
  "I have to restart left and right".

## Decision

Keep the ADR-023 engine, transport and boot seam. Change WHAT is merged and
WHO gets to stamp it. Zero new Rust.

1. **Tab identity.** `tabIdentity(SerializedTab)`: terminal → `t:<oldest
   terminal pane's restore key>` (the key already names the tmux window
   `w-<key>`, so both devices agree without coordination; "oldest terminal
   pane" rather than "first pane" because the simulation showed a note
   split in at position 0 changing the identity mid-incident); notes/board/
   tasks → the shared doc id; editor/markdown → `f:<path>`; preview →
   `u:<url>`; singleton kinds → `s:<kind>`. Content equality and tie-breaks
   use the serialized tab minus per-device UI state (which pane is active).
2. **Per-tab clocks.** `SpaceStateMeta = { at, tabs?: Record<id, clock>,
   gone?: Record<id, closedAt> }`. `at` survives as the space-level clock
   (order, activeTabIndex, and the fallback clock for envelopes from 0.12.0
   clients that carry no `tabs` map; additive wire change, `v` stays 1).
3. **Stamping is a pure function** (`stampTabs` in `spaces/lib/tabClocks.ts`),
   called by `saveState`: unchanged tab keeps its clock; changed tab → now;
   closed tab → `gone[id] = now`; NEW tab → the adoption ledger's clock if
   one is registered for its identity, else now. Adoption writes (boot and
   live merge) pass meta verbatim, as today.
4. **Adoption ledger** (`sync/lib/adoptionLedger.ts`): whoever creates a tab
   on BEHALF of another device registers the clock it is adopting at;
   tmux materialization registers `0` ("I know nothing about this tab,
   anyone's version beats mine"), live adoption registers the remote tab's
   clock. Entries are consumed by the first save that persists the
   identity and expire after 60 s. This is the whole of R2's fix: an
   observer can never outrank an author.
5. **Merge per tab** (`sync/lib/mergeState.ts`, used by `mergeWorkspace` and
   by the identity fold): union by identity; winner = higher clock; an
   equal-clock tie resolves by content, deterministically, so both devices
   pick the same copy instead of each keeping its own (ADR-023's "local
   wins" tie could never converge); `gone` beats a tab unless the tab's
   clock is newer than the close; order = local order with unseen remote
   tabs appended; `at` = max; activeTabIndex local, re-clamped. A side
   without a `tabs` map gets every tab clocked at its `at`. A doc that the
   merged layout shows as a PANE drops its standalone doc tab (the live
   layer materializes panes as tabs; boot's split brings the pane, so the
   tab would be a duplicate).
6. **Live layer widened** (ADR-024 stays additive): after every live pull,
   besides new doc tabs, apply RENAMES of any tab whose remote clock beats
   the local one (terminal `customTitle`, doc `title`), through the
   ledger so the resulting save carries the remote clock. Closes, splits
   and reorders stay boot-only. New terminals keep arriving via tmux.
7. **Self-healing push.** The live poll already merges local against the
   host every 10 s while visible; if the merge says the host lacks
   something local wins on clock (`pushNeeded`), push. A lost race heals
   within one poll. Chosen over a CAS write (would need a Rust command and
   a two-phase part layout) because it gives the same guarantee at 10 s
   latency with the mechanism that already exists.
8. **Sync journal + undo.** Every remote change applied to an EXISTING
   local tab (boot or live) appends `{at, spaceId, tabId, field, before,
   after, fromDevice}` to `koden-sync-journal.json` (ring of 100). Live
   renames raise a toast with Undo; undo restores the value and stamps it
   now, so it wins the next merge. Nothing a peer does to your layout is
   silent any more, and nothing is unrecoverable.
9. **F2 title manifest reader deleted.** Titles ride the ws domain with
   clocks and retries. The manifest WRITER stays for the ai-server
   dashboard's window labels only. `syncRemoteSpace` runs while the window
   is visible, not only focused (the other device is never focused).
10. **Two-device simulation** (`twoDevice.sim.test.ts`): an in-memory host
    with gens, two device models running the real `stampTabs` +
    `mergeWorkspace` + live/boot cycles. Replays this incident step by
    step and fuzzes seeded random interleavings, asserting: an author's
    edit is on the host after both devices' next live cycle; no
    user-stamped field is ever replaced by a lower or ledger-0 clock;
    both devices converge after boot (400 seeded runs). This is the
    regression failsafe for the class, not the instance.

Rejected: full host-authoritative op-log (ADR-024 north star). Right
end-state, but it needs a host process and a control plane; the LWW map
gets every guarantee Kosta asked for today with ~400 lines and no daemon.
Revisit when the control plane (KODEN-REMOTE.md M3) lands.

## Consequences

- Loss is now bounded to ONE FIELD of ONE TAB, only under a true
  simultaneous edit of the same field on two devices, and it is journaled
  and undoable. Derived writes can never win over an author.
- Terminal tab names converge live (≤ 10 s poll + 3 s push) without a
  restart. Splits and closes still converge at boot (ADR-024 invariant:
  never rewrite a live pane tree, never close under the user).
- Old clients (0.12.0) keep merging at space level against the new
  envelopes; mixed fleets degrade to today's behaviour, never worse.
- Clock skew assumption unchanged (NTP). Ledger-0 is the only "unknown"
  clock and always loses.
- New files: `spaces/lib/tabClocks.ts`, `sync/lib/adoptionLedger.ts`,
  `sync/lib/journal.ts`, `sync/lib/twoDevice.sim.test.ts`. Modified:
  `spaces/lib/store.ts`, `sync/lib/{types,mergeWorkspace,engine,liveAdopt}.ts`,
  `app/App.tsx` (ledger registration, manifest reader removal, visibility
  gate).
- Ships as v0.12.1 through the normal release ritual (CI builds on tag
  push, no local build); the 8-step checklist reruns against it (steps
  3–5 were blocked by R1/R2). `src-tauri/Cargo.toml` finally bumped too
  (it was still 0.11.0, so koden-brain misreported its version).
- Not built yet: a Settings list over the journal (the file and the live
  toast+Undo exist). Add when someone asks "what did sync change".
- Verification: 110 tests in sync + spaces (85 before), whole frontend
  suite green, `tsc` clean, biome error-free; Rust untouched.

## Second run, 2026-09-03 12:12 (v0.12.1 on both): two more holes, same class

Tab names now arrived live. A note split into a tab on HQ did not arrive
at all, and the host again held the laptop's bare copy of the tab, stamped
12:13:26 against HQ's split at 12:12:14. The laptop had edited nothing.

- **R2b, derived pane fields counted as content.** The laptop's copy of the
  tab differed from HQ's in the shell-reported `cwd` (OSC 7) and the
  auto-assigned pane accent `color`. Equality saw a change, stamping saw an
  edit, the observer won again. Fix: `tabContentJson` strips `active`,
  `cwd` and `color` from every leaf; only structure, doc ids, restore keys,
  pane labels and the tab name are authored content. Comparison is
  key-order independent (a merge composes tabs from two sources).
- **R1b, one clock per tab still loses.** The fuzz, once it could churn
  cwd/colour and split notes, found seed 4: a rename on device B two ticks
  after a split on device A erased the split, because both are edits to
  the same tab and the later one carried the whole tab. Fix: two clocks
  per tab, `tabs` (structure) and `titles` (name); the merge picks the
  structure winner and the name winner separately and composes them.
  Losing an edit now needs the SAME field of the SAME tab edited on both
  devices inside one debounce window.
- **G1b, splits were boot-only and Kosta expects them live.** Added live
  structural adoption to the ADR-024 layer, still additive: a peer's tree
  is adopted mid-session only when it is newer on the structure clock AND
  keeps every pane this device runs (`planLiveTrees`). The tree is
  hydrated around the existing leaves (`hydrateTreeReusing`), so the
  terminal keeps its PTY and the note pane appears beside it; a doc that
  arrives as a pane is not also raised as a standalone tab. Journaled, with
  a toast and Undo. A peer that CLOSED one of our panes still waits for
  boot: the live layer never removes what you are looking at.
- Fuzz widened accordingly: cwd/colour churn, note splits, per-field
  oracle (a tab is alive iff any authored edit is newer than the last
  close; the expected name is the latest rename; an authored split must be
  on the host and on both devices after boot, and on the other device
  LIVE when only one device split). 400 seeds, 114 tests in sync + spaces.
- Ships as v0.12.2. Known ceiling: a standalone note tab the 0.12.1 live
  layer already raised on a device stays until that device's next boot,
  when the pane wins and the duplicate tab is dropped.
