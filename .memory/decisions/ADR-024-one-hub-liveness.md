# ADR-024: One-hub liveness — two devices, live, on top of ADR-023

Status: Accepted — 2026-09-02 (Kosta: "plan this out perfectly, do testing,
validations, then full rework so it's a true one hub … if I add a note to a
tab on laptop it populates the desktop as it's all from the same place";
full delivery mandate given the same evening). Supersedes the first draft of
this file, which was written before discovering that the workspace-sync
branch had already claimed ADR-023 — and already built most of the hub.

## Context

Kosta's requirement is a true one-hub workspace: connect "home" and
everything is exactly as left, AND two devices running simultaneously
converge live — a note added on the laptop appears on the desktop within
seconds, terminals likewise.

The day's archaeology (2026-09-02) found THREE overlapping remote-state
systems that did not know about each other:

1. **M2.5 F2 manifests** (v0.11.3–5): per-space title manifest + `-docs.json`
   docs replication — 3 bug-fix releases in one day, each fixing the
   previous; the structural gap (same tab = 1 terminal on desktop, 2 + note
   on laptop) unfixable in that layer.
2. **ADR-023 sync engine** (`src/modules/sync/`, built overnight 2026-09-01
   on the workspace-sync branch, adversarially reviewed, accepted by Kosta):
   docs domain (per-entry LWW, live), ws domain (spaces + full layouts +
   tombstones + identity fold + path portability) — but layout adoption
   deliberately **boot-only**.
3. **M2.8 pane events**: the only true host→client push channel.

## Decision

**Unify on ADR-023's engine and add the missing liveness — additive-only,
so every reviewed ADR-023 invariant stays intact.**

1. **Merge** the workspace-sync branch into released main (zero-conflict
   merge, commit `1432606`; the branch also gains the v0.11.3–5 fixes:
   custom-title manifest, clean-exit semantics, fill-character).
2. **Teardown**: the F2 `-docs.json` replication layer is deleted
   (`remoteDocs.ts`, the App push/pull effects, `applyRemote*`); the engine's
   docs domain is the only docs-sync mechanism. The F2 TITLE manifest stays
   (it is the only live title path mid-session; consolidation into the ws
   domain is future work). tmux window adoption stays (terminal existence is
   tmux truth, not sync truth).
3. **Liveness cadences** (visible window only; unchanged gen = one ssh
   handshake, no ControlMaster exists on Windows OpenSSH): docs pull 10 s,
   docs push debounce 2.5 s, ws live pull 10 s, ws push 3 s after a real
   layout edit (new `notifyWsChanged` signal from `saveState`) with the 60 s
   check as fallback. Hidden windows fall back to the slow timers.
4. **Live additive adoption** (`liveAdopt.ts` + engine `liveWsAdopt`): a ws
   pull mid-session materializes NEW doc tabs (including docs that exist as
   panes on the origin device) and applies doc-tab renames — never closes,
   never reorders, never rewrites pane trees of a live UI, and NEVER
   advances the persisted ws `lastGen`, so boot still does the full
   structural merge exactly as ADR-023 specifies. Session-local gen
   tracking; idempotent by construction (planner tested).
5. **Defaults**: `syncEnabled: true` (the hub workflow is the product; an
   unreachable host is a visible offline state, never an error),
   `syncHost: "ai-server"`. `syncPathRoot` stays empty by default — ssh
   Spaces (the daily driver) need no path rewrite; set it on each machine to
   enable local-space folding.

### End-state direction (recorded, not decided)

The first draft of this ADR proposed full host-authoritative state — one
state document per Space on the host, devices as pure write-through
viewports, no local source of truth. That remains the north star if
merge-based sync shows real-world losses (per-doc LWW can drop keystrokes
when the same note is edited on two machines in the same debounce window).
Revisit when: (a) Kosta reports lost edits, or (b) the control plane work
(KODEN-REMOTE.md M3) needs a host state registry anyway. The engine's
envelope schema is the natural seed for that document.

## Consequences

- A note/task/board created on either device appears on the other within
  ~15 s worst case (2.5 s push + 10 s poll) while both are visible; content
  edits converge on the same cadence; full structure (splits, order,
  removals) converges at next boot per ADR-023.
- Live adoption matches spaces by id, which is only guaranteed after the
  identity fold has run once on both devices — first sync after this
  release converges at each device's next boot, live thereafter.
- Poll load while visible: ≤ ~12 ssh handshakes/min (docs 6 + ws live 6),
  each ~100–500 ms. Acceptable on LAN/tailnet; revisit with a push channel
  (widen the M2.8 tail mechanism to a second hardcoded events file) if a
  cellular-hotspot workflow ever matters.
- Same-doc simultaneous typing on two machines: whole-doc LWW at debounce
  granularity — later save wins. Documented ceiling (see end-state).
- Tests: liveAdopt planner suite (idempotence, pane-existence dedupe,
  rename-tabs-only), on top of the engine's merge/chunk/pathMap suites;
  702-test whole-suite green at merge, 701 after teardown (+6 new, −7
  retired with the F2 docs layer).
