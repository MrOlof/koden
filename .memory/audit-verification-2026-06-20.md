---
title: "Audit verification — feature-research + fork-rebrand docs vs. the current tree"
created: "2026-06-20"
status: "verification of feature-research-2026-06-19 + fork-rebrand-and-onboarding-2026-06-19 against the live tree as of 2026-06-20 (static-only, not GUI-verified)"
verifies:
  - feature-research-2026-06-19.md
  - fork-rebrand-and-onboarding-2026-06-19.md
method: "per-claim file:line re-check of the two baseline docs + the installed ~/.claude/settings.json + the PowerShell $PROFILE; line numbers re-anchored to this session's edits"
note: "Both baseline docs are from 2026-06-19 and predate this session's work. Where a doc claim is now wrong, this file is authoritative — trust reality over the stale docs."
---

# Audit verification (2026-06-20)

This addendum records the result of verifying the two 2026-06-19 baseline docs
against the **actual current tree** after this session's overnight + same-day
edits. The baselines were static-only and their line numbers have shifted.
Where they diverge from reality, **this file wins.**

Scope check note: the name is **decided — Koden** (KOsta waDENfalk; "koden" =
"the code" in Swedish), per `CLAUDE.md`. GitHub owner likely `MrOlof`; npm
reservation is `@mrolof/koden` (bare `koden` blocked by npm similarity filter,
`@koden` org taken — placeholder `v0.0.1` published 2026-06-20). The bundle-id
triple `app.<org>.koden` + repo slug remain a **decision for Kosta** (the
identity/endpoints work lives in the overhaul-plan lane, not here).

---

## 1. CHANGED-SINCE-DOCS — what this session already did

These items in the baseline docs are now **RESOLVED / STALE**. Both docs
predate them.

- **Per-pane subagent bus is now WIRED + RESILIENT.** `AgentBusBridge` is the
  live per-pane subagent + status path; the missing writer now exists in the
  installed `~/.claude/settings.json`; corrupt `agent-bus.jsonl` lines are
  recovered by the new `subagentBus.ts`. → feature-research **Open Question #1
  / gap #1 RESOLVED**; debloat **cut of AgentBusBridge is now WRONG** (load-bearing).
- **Updater auto-check is OFF.** `useUpdater.ts:18` `AUTO_UPDATE_DISABLED = true`,
  gated at `:157`. → rebrand "demote the updater" item is **partially done**
  (endpoint/key repoint still owed).
- **Scrollback search / "Find in terminal" shipped.** feature-research Tier-1 #7
  is **DONE** (match counter + next/prev cycle in `TerminalHistoryPopover.tsx`).
- **Turn capture shipped.** `commandMarks.ts` gained `scanTurns` + a broadened
  `extractTurnPrompt` (recovers Claude `>`/`❯` prompt turns from the buffer when
  no OSC-777 mark exists).
- Also shipped this session (noted, not in the original Tier list): **single
  sidebar as the default layout**, **grid launcher**, **tab + pane context
  menus**, **AGENTS group/filter** in the sidebar, **hover scrollbar**.

Net: 1 decision-gated cut flipped to load-bearing, 1 feature-rec (#7) + the
updater demote landed, and the two cheapest open wins (`cm`/@args, `deriveEdges`
wire-or-delete) remain.

---

## 2. Agent-visibility verification (per claim)

### Gap #1 — per-pane subagent bus — **NOW-DONE (RESOLVED)**
Was: "`AgentBusBridge` reads `~/.terax/agent-bus.jsonl` but nothing writes it"
(feature-research:18, :39; debloat:106). **That is now FALSE.**

- **Reader (live + resilient):** `src/modules/orchestration/components/AgentBusBridge.tsx`
  parses `agent-status` and `subagent-stop` per line, and recovers
  `subagent-start` via a tolerant scan. Mounted at **`App.tsx:2184`**
  (`<AgentBusBridge busPath={agentBusPath} />`); `agentBusPath` defined at
  **`App.tsx:375`**.
- **Resilience layer (new this session):** `src/modules/orchestration/lib/subagentBus.ts`
  `extractSubagentStarts(content, seen)` recovers Tasks by unique `tool_use_id`
  (dedup `Set`), surviving the non-atomic hook's interleaved / doubled / `}{`-glued
  JSON. `AgentBusBridge.tsx:5,152` consumes it. Unit-tested (`subagentBus.test.ts`).
- **Writer (the missing half) now EXISTS** in the installed `~/.claude/settings.json`
  (installed out-of-band this session, NOT via the in-app installer). Verified
  verbatim:
  - `UserPromptSubmit` / `PostToolUse` → `{"cmd":"agent-status","id":$TERAX_SESSION,"state":"working"}` → `agent-bus.jsonl` (settings.json:40,124)
  - `Notification` → `…"state":"attention"` (settings.json:12)
  - `Stop` → `…"state":"finished"` (settings.json:88)
  - `PreToolUse` `Task|Agent` → `{"cmd":"subagent-start","parent":$TERAX_SESSION,"task":<stdin>}` (settings.json:69)
  - `SubagentStop` → `{"cmd":"subagent-stop","parent":$TERAX_SESSION}` (settings.json:106)
  - These match the reader's contract exactly (`leafIdForPty(id|parent)`,
    `AgentBusBridge.tsx:91-143`).
- **Session reset (supersedes the old "App.tsx only truncates it" reasoning):**
  **`App.tsx:501-503`** truncates `agent-bus.jsonl` once at boot so the reader
  never replays a dead-pty run. (Old doc cited `App.tsx:493`/`:495-497` — line
  moved.)

**Root-cause cleanup STILL OPEN (new, post-resolution):** the `subagent-start`
hook is **non-atomic** — `{ printf prefix; cat | tr -d '\r\n'; printf '}'; } >> agent-bus.jsonl`,
three separate appends (confirmed verbatim, settings.json:69). Parallel Tasks
still corrupt the file; the reader *recovers* (`subagentBus.ts`), but the proper
fix is a single atomic write. **Plus a dual-installer split:** the in-app
`src-tauri/src/modules/agent.rs` (`agent_enable_claude_hooks`, `agent.rs:141`)
still writes the **LEGACY** path — OSC-777 `terminalSequence` for status
(`agent.rs:17`) + `director-bus.jsonl` (untagged) for subagents (`agent.rs:27`),
NOT `agent-bus.jsonl`. Its `OWNED_MARKERS` (`agent.rs:11`:
`["notify;Terax;", "terax;notify", "director-bus.jsonl"]`) do **not** recognize
the new `agent-bus` hooks, so re-running it would re-add the legacy groups
alongside the hand-installed ones. **Flag for cleanup:** reconcile `agent.rs` to
emit the `agent-bus.jsonl` contract, or it will drift.

### Gap #2 — OSC-777 status path not honored — **CONFIRMED (still a caveat, mostly mooted)**
- `src-tauri/src/modules/pty/session.rs:137` injects `TERAX_SESSION`; its own
  comment (`session.rs:133-137`) states *"the OSC 777 / terminalSequence path is
  not honored by the installed CC, so status flows over the file bus like the
  Director's subagents do."* CONFIRMED.
- Consequence: plain-`claude` live status now flows via the **file bus**
  (gap #1's resolution), so for **status** this caveat is **largely mooted in
  practice** — the `agent-status` lines in settings.json are the working path.
  The OSC-777 `terminalSequence` hooks (settings.json:20,96,132) are still
  present but **inert**. Still GUI-unverified.

### Gap #3 — `getAgentCommand()` default `cm` drops `@args` — **CONFIRMED, STILL A REAL BUG**
- `src/modules/orchestration/lib/agentCommand.ts:5` `DEFAULT_COMMAND = "cm"`
  (unchanged; line confirmed, not moved).
- The user's actual `cm` function (`C:\Users\Snorlax\OneDrive\Documents\PowerShell\Microsoft.PowerShell_profile.ps1:20-37`)
  ends with `& $cmd.Source` — invokes `claude` with **no arguments**, does not
  forward `@args`. So default-launched agents silently lose
  `--append-system-prompt` / `--agents` / `--model`. **Still the highest-value
  open correctness fix** (cheapest Tier-0 win).

### "What's wired" — re-verified, lines re-anchored
| Claim | Status | Current file:line (corrected) |
|---|---|---|
| Agents sidebar view + count badge | CONFIRMED (line moved) | `SidebarRail.tsx:68-70` (label "Agents", `badge: agentCount`); doc said `:40-45` / App `:1968-1977` |
| `terminalsToRegister` pre-registration | CONFIRMED (line moved) | loop `App.tsx:727`, block `721-739`; doc said `App.tsx:712-732` |
| Live status (Rust OSC machine → bridge) | CONFIRMED | `agent_detect.rs` → `terax:agent-signal` (`App.tsx:508-509`) → `OrchestrationActivityBridge` (`App.tsx:2182`) |
| `TERAX_SESSION` per-pane | CONFIRMED | `session.rs:137` |
| Notifications / attention / taskbar flash | CONFIRMED | `OrchestrationAttentionBridge` (`App.tsx:2183`) |
| Topology / Director surfaces | CONFIRMED | `AgentTopologyView.tsx`, `DirectorBusBridge` (`App.tsx:2185`), `MessageFlowInspector` |
| `AgentBusBridge` mount | CONFIRMED (line moved) | `App.tsx:2184`; doc said `:2130`/`:2161` |

Stale-memory correction from the baseline still holds: `INDEX.md`/ADR-001 call
per-pane `TERAX_SESSION` "deferred" but it **is** injected at `session.rs:137`.

---

## 3. Debloat-cut verification (per cut)

### Decision-gated pair — both CHANGED
- **`AgentBusBridge` → NOW-UNSAFE (was the #1 cut).** It is now the LIVE,
  load-bearing per-pane subagent + status path (see §2 gap #1). **Do NOT cut.**
  `[AgentBusBridge.tsx; subagentBus.ts; App.tsx:375,501-503,2184; orchestration/index.ts:40]`
- **`deriveEdges()` + `TopologyEdge` type → SAFE-still (wire-or-delete still OPEN).**
  `AgentTopologyView.tsx:18` imports only `isActiveStatus` from `topology.ts`; it
  builds its own local edge set from `parentId` ownership and ignores
  `deriveEdges`. `deriveEdges`/`TopologyEdge` are defined/exported
  (`topology.ts:33-59`, type at `lib/types.ts`, barrel `index.ts:10,22`) but
  consumed only by `topology.test.ts`. Still dead in the live graph → still "wire
  the dashed flow edges or delete." **Do NOT conflate** the dead `TopologyEdge`
  *type* with the used `TopologyEdge` *component* at `AgentTopologyView.tsx:506`
  (rendered at `:279`) — different thing, keep it.

### Dead Tauri commands — all SAFE-still (zero JS `invoke` callers)
Grep for `ai_http_request|usage_guard_snapshot|wsl_default_distro` across `src/`
returned **no matches** → confirmed zero JS callers for all three.
- `ai_http_request` + `HttpResponse` — `net.rs`, registered `lib.rs:244`. SAFE.
- `usage_guard_snapshot` + `UsageSnapshotDto` — `usage/mod.rs`, `lib.rs:247`. SAFE.
- `wsl_default_distro` — `workspace.rs`, `lib.rs:231`. SAFE.

### Dead JS exports — all SAFE-still (zero importers/callers; session used NONE)
- `unlinkByLeaf` — only defined (`orchestrationStore.ts:50,179`), **zero callers**
  in `src/` (grep). Teardown uses `removeByLeaf`/`removeWithChildren`. SAFE —
  **confirmed this session did NOT start using it.**
- `forEachSlot()` — exactly one occurrence in `src/` (its def,
  `rendererPool.ts:115`). SAFE — **confirmed not used this session.**
- `getSourceControlRemoteIndicator()` — `useSourceControl.ts:89` + barrel
  `source-control/index.ts:3`; no other use. SAFE.
- `compactModelMessages` wrapper — `compact.ts:146-151`; only `…Detailed` used. SAFE.
- `preloadLanguages()` — `languageResolver.ts:197`; no caller. SAFE.
- default `proxyFetch` export — `proxyFetch.ts`; only `createProxyFetch` imported. SAFE.
- `getMarker` on PromptTracker — `osc-handlers.ts:38`; no external caller. SAFE.
- Barrel/export trims — all SAFE-still: `flushDocs`/`installDocsCrashGuard`
  re-exports (`workspace-docs/index.ts:5-7`); `ThemeModePref` alias
  (`ThemeProvider.tsx:31`); `SearchInline` value re-export (`header/index.ts:3`);
  `PaletteItem.iconUrl` (`command-palette/types.ts:12`); `DragOverEvent`
  re-export (`dnd/index.ts:11`); `export` on `ParsedQuery` (`mode.ts:3`);
  `defaultBoard`/`defaultTaskList` (`docsStore.ts:44,53`); `SIDEBAR_RAIL_HEIGHT`
  re-export (`sidebar/index.ts:1`).

### Dep trims — all SAFE-still
- `@radix-ui/react-use-controllable-state` — still a direct dep (`package.json`). SAFE.
- `tempfile` duplicate (`Cargo.toml`, normal + dev). SAFE.
- `grep-matcher` direct (`Cargo.toml`). SAFE.

### Safe shrinks — SAFE-still
- 6× `*Lazy.tsx` confirmed present. SAFE.
- `OrchestrationAttentionBridge` re-inline focus, `run_git` fold,
  `run_blocking_inner` widener, `parentDir` dedup — not re-checked individually;
  no signal they changed; treat as SAFE pending a Rust-side pass.

---

## 4. Still-live high-value items (open)

In rough cheapest-first order:

1. **`cm`/@args bug (Tier-0).** Real, confirmed. Cheapest correctness win.
   `agentCommand.ts:5` (default `cm`); profile `cm` (`$PROFILE:20-37`) ends with
   `& $cmd.Source`, no `@args`. Contrast the sibling `glm` launcher
   (`$PROFILE:59`) which sets up env then runs `claude` properly.
2. **Wire-or-delete `deriveEdges` (Tier-0).** Still dead in the live graph
   (`topology.ts:33`, ignored by `AgentTopologyView.tsx`). Render the dashed
   message-flow edges or drop the export.
3. **Atomic `subagent-start` hook + reconcile `agent.rs` (new root-cause cleanup).**
   Reader recovers (`subagentBus.ts`), writer still corrupt (settings.json:69,
   3-write append). Also reconcile the in-app `agent.rs` installer
   (`agent.rs:11,17,27,141`) to emit the `agent-bus.jsonl` contract instead of
   the legacy OSC-777 + `director-bus.jsonl` path, or the two installers drift.
4. **#1 Bridge `managedAgentsStore` → orchestration.** AI-SDK agents still
   invisible (3-store fragmentation: `orchestrationStore` / `agentStore` /
   `managedAgentsStore`) — unchanged.
5. **#2 Pane header/border ambient status tint.** Open (now lower-risk since
   status flows via the file bus, gap #1).
6. **#3 `>note` / `>task` palette quick-add.** Open.
7. **#4 prev/next command-mark keybind; #5 copy command/output from mark; #6
   zoom pane; #8 Tasks/Notes keybinds.** Open. (#5 "copy-from-mark" relates to
   the copy-on-wrap idea; not shipped.)
8. **Setup wizard.** Greenfield, not started (`src/modules/onboarding/` does not
   exist). One persisted boolean `hasCompletedSetup` + in-main-window overlay,
   per the rebrand doc's design.
9. **Updater repoint + minisign keypair.** Auto-check is OFF (`useUpdater.ts:18`),
   but the endpoint + key still point at upstream `crynta/terax-ai`
   (`useUpdater.ts:9-10`, `tauri.conf.json`). Repoint + mint a new keypair owed.
10. **Reconcile persistence docs.** `WORKSPACE.md`/`.memory/INDEX.md` still claim
    `terax-orchestration.json` persistence that the code deliberately removed
    (session-scoped). Docs only.

---

**Bottom line:** the agent-visibility audit holds with line shifts, **except**
gap #1 is RESOLVED and so AgentBusBridge moved from "top cut candidate" to
load-bearing. One feature-rec (#7 scrollback search) and the updater demote
landed. The two cheapest open wins remain `cm`/@args (gap #3) and the
`deriveEdges` wire-or-delete. All other 26 debloat cuts remain safe with the
lines noted above. New cleanup surfaced: the non-atomic subagent-start hook +
the `agent.rs` ↔ live `agent-bus.jsonl` installer split.
