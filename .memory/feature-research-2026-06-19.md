---
title: terax-workspace — feature research (Warp/tmux/Zellij/WezTerm/iTerm2/Zed) + agent-visibility audit
created: "2026-06-19"
updated: "2026-06-19"
status: research only — READ-ONLY pass, no source code changed
method: 16-agent read-only workflow — 5 codebase mappers + 6 external scouts (Warp, tmux, Zellij, WezTerm/Kitty/Ghostty, iTerm2/Zed/VSCode, notes/tasks+agent-viz) → synthesis → 3-lens skeptic panel (anti-bloat / duplication / daily-value) → editor
audit_merge: "2026-06-19 — 8-agent repo-wide ponytail debloat audit folded in (see §Debloat cuts); 28 cuts, net −487 lines / −3 deps; 2 cuts (AgentBusBridge, deriveEdges) are decision-gated against the agent-visibility items, not delete-now"
philosophy: small, anti-bloat workflow features (tasks/notes/panes/palette); reuse existing machinery, no new deps/servers/settings empires
---

# terax-workspace — feature research (2026-06-19)

## TL;DR

- **Direction:** Stay terminal-first and anti-bloat. Every recommended item is a small, self-contained QoL win that reuses what already exists (the command palette, the pane algebra in `panes.ts`, the orchestration store, `docsStore`, the spaces serialize layer, the shortcuts registry). No new dependency, server, or settings empire is recommended.
- **Can you see the agents? Yes — mostly.** A real "Agents" sidebar view exists (third rail icon, live count badge), pre-registers every open terminal as an idle node, shows live status colors, has a topology graph, a Director view, notifications, and taskbar flash. This is **wired and works** for plain `claude` terminals and the Director.
- **The "always empty" fear is already fixed:** `terminalsToRegister` (`src/modules/orchestration/lib/terminalAgents.ts`) seeds a dock node the instant a terminal pane opens (`App.tsx:712-732`), so you don't have to talk to Claude first.
- **Two real gaps remain:** (1) the per-pane subagent bus is dead-wired — `AgentBusBridge` reads `~/.terax/agent-bus.jsonl` but nothing writes it; (2) `getAgentCommand()` still defaults to `cm`, whose PowerShell `$PROFILE` function drops `@args`, silently losing `--append-system-prompt`/`--agents`.
- **Highest-leverage structural win:** bridge `managedAgentsStore` → `orchestrationStore` so AI-SDK-launched coding agents stop being invisible in a third store.
- **Caveat:** nothing on the branch is runtime-GUI-verified — all behavior claims are static-only. Verify in `pnpm tauri dev` before trusting live status transitions.
- **Stale-memory correction:** `INDEX.md`/ADR-001 list per-pane `TERAX_SESSION` as "deferred / not done," but it **is** injected at `session.rs:137`. Memory is out of date on this point.
- **Debloat audit merged (2026-06-19):** a repo-wide over-engineering scan found **28 cuts, net −487 lines / −3 deps** — mostly dead Tauri commands, dead exports, dep trims, and Lazy.tsx boilerplate. Two of them (`AgentBusBridge`, `deriveEdges`) are the *same* items as the agent-visibility decisions below — **decide before deleting**. Everything else is safe. See §Debloat cuts.

## Can you see the agents? (hooks)

**Verdict: PARTIAL — but "partial" means "works, with two known gaps," not "absent."** The hooks to see agents exist and most are genuinely wired.

**What's wired (works today):**
- **Sidebar "Agents" view** with a live count badge — `SidebarRail.tsx:40-45`, rendered at `App.tsx:1968-1977`, count at `App.tsx:1992`.
- **Pre-registration of every terminal** as an idle dock node the moment its pane opens — `terminalsToRegister` (`src/modules/orchestration/lib/terminalAgents.ts`), wired at `App.tsx:712-732`. This is the fix for the historic "list is always empty" complaint; no need to run `claude` first.
- **Live status detection** from a heavily-tested Rust OSC state machine — `src-tauri/src/modules/pty/agent_detect.rs` → `terax:agent-signal` → `OrchestrationActivityBridge.tsx` (drives dock colors: blue=working, amber=waiting, green=done, red=error).
- **Per-pane identity injected** — `TERAX_SESSION=<ptyId>` at `session.rs:137` (note: `INDEX.md`/ADR-001 wrongly call this "deferred" — the memory is stale).
- **Notifications + attention:** `NotificationBell`, Sonner toasts, OS notifications when unfocused, and taskbar flash via `OrchestrationAttentionBridge.tsx`; per-tab worst-wins pills via `useTabStatusStore`.
- **Topology / flow / Director surfaces:** `AgentTopologyView.tsx`, `MessageFlowInspector.tsx`, `DirectorView.tsx` all render from `orchestrationStore`.
- **Director file bus:** `director-bus.jsonl` → `DirectorBusBridge` (mounted while a Director is live) spawns/retires native subagent nodes.
- **Retry/usage visibility:** `retry_detect.rs` + `RetryBridge` + `retryStore`; `UsageBridge` + `usageStore`.

**What's missing:**
1. **Dead-wired per-pane subagent bus.** `AgentBusBridge.tsx` (mounted `App.tsx:2130`) polls `~/.terax/agent-bus.jsonl`, but **nothing writes that file** — `agent.rs` emits status via OSC 777 and writes subagent lines only to `director-bus.jsonl` (untagged), and `App.tsx:493` only ever truncates `agent-bus.jsonl`. So a plain (non-Director) `claude` terminal spawning `Task` subagents surfaces nothing in the dock.
2. **Status-transition reliability caveat.** `session.rs:137`'s own comment says the installed Claude Code does **not** honor the OSC 777 terminalSequence path — so live working/attention/finished transitions from real `claude` sessions may not light up even though the idle node appears. Needs GUI verification.
3. **`getAgentCommand()` defaults to `cm`** (`src/modules/orchestration/lib/agentCommand.ts:5`) — the `$PROFILE` `cm` function runs without `@args`, dropping `--append-system-prompt`/`--agents` on every default-launched agent.
4. **Three stores, no single source of truth** — `orchestrationStore` (9 statuses), `agentStore` (working|waiting), `managedAgentsStore` (review-loop). AI-SDK agents in store #3 never appear in the dock.

**How to verify live in the GUI:**
1. `pnpm tauri dev` (kill any running `terax.exe` first — the watcher does not reliably rebuild Rust).
2. Open a terminal tab, click the third sidebar rail icon ("Agents"). Badge should be ≥1 and the terminal should immediately appear as an idle node named by its cwd (proves `terminalsToRegister`).
3. Run `claude`, submit a prompt → node should go blue/working, tab pill should escalate. Trigger a permission prompt → amber/waiting; with window unfocused, taskbar should flash. **If status does not change live, that's the OSC-777-not-honored caveat (gap #2).**
4. Toggle the dock's list/graph button → `AgentTopologyView` should render.
5. From the Director view, start a Director and have it spawn a `Task` subagent → subagent nodes should appear via `DirectorBusBridge`.
6. **To prove gap #1:** with a plain (non-Director) `claude` session, run a `Task` subagent → confirm NO subagent node appears and `~/.terax/agent-bus.jsonl` stays empty.

## Recommended features

### Tier 0 — correctness/housekeeping (cheapest wins, do first)

These aren't features; they're zero-to-low-bloat fixes the skeptics flagged as higher priority than several candidates.

| Item | What | Touches | Effort |
|---|---|---|---|
| Fix `getAgentCommand()` `cm` default | Stop dropping `@args` so default-launched agents keep `--append-system-prompt`/`--agents`. Live correctness bug. | `src/modules/orchestration/lib/agentCommand.ts:5` | S |
| Reconcile persistence docs | `WORKSPACE.md`/`.memory/INDEX.md` still claim `terax-orchestration.json` persistence that the code deliberately removed (session-scoped). Update docs to match the non-goal so no one rebuilds phantom persistence. | docs only | S |
| Wire or delete `deriveEdges()` | The tested ownership+flow edge derivation is exported but `AgentTopologyView` ignores it, so the dashed weighted message-flow edges are invisible. Either render them or drop the dead export. | `src/modules/orchestration/lib/topology.ts`, `AgentTopologyView.tsx` | S |

### Tier 1 — do soon (strong small wins, high daily value)

| # | Feature | What it does | Inspiration | Module / files | Reuses | Effort |
|---|---|---|---|---|---|---|
| 1 | **Bridge `managedAgentsStore` → orchestration** | Mirror AI-SDK coding agents into the orchestration store so they appear in the dock/topology automatically instead of being invisible in a third store. | disler multi-agent observability | `src/modules/agents/store/managedAgentsStore.ts`, `src/modules/orchestration/store/orchestrationStore.ts` | The existing Bridge component idiom (orchestration barrel); both stores key by `leafId`. No new store/UI. | M |
| 2 | **Pane header/border as ambient agent-status signal** | Tint the focused pane's header/border by live agent state (amber=needs-you, blue=working, green=done, red=error) so the whole layout is a "who needs me" heat-map without opening the dock. | accessd tmux-agent-indicator | `src/modules/terminal/PaneTreeView.tsx` + `paneTitles.ts`, orchestration `STATUS_META` | Per-pane color field (ADR-002) bound to live `STATUS_META`; no escape parsing. | S |
| 3 | **Quick-add to Notes/Tasks via palette (`>note` / `>task`)** | A palette verb opens a one-line input that appends straight to the active space's Notes or Tasks without switching tabs. *(Drop the `#tag`/`!high` parsing to stay minimal.)* | Raycast / Todoist quick-add | `src/modules/command-palette` + `src/modules/workspace-docs/store/docsStore.ts` | `cmdk` CommandDialog, `parseQuery` sigil router, crash-safe `addTask`/`setNote`. No new store/window/hotkey. | S |
| 4 | **Jump to prev/next command mark (keyboard)** | Bind Mod+Up / Mod+Down to scroll the focused terminal to the previous/next OSC-133 command boundary — skip walls of build/agent output. | iTerm2 / VS Code / WezTerm | `src/modules/terminal/lib/commandMarks.ts`, `useTerminalSession.ts`, `shortcuts.ts` | `CommandMarks` ranges already tracked; `scrollToCommand` already exists (click-only today). One handler + two shortcut ids. | S |
| 5 | **Copy command / copy output from a command mark** | One action (palette + pane context menu) to copy the last command, its full output, or both, using the OSC-133 C..D range instead of hand-selecting wrapped lines. | Warp blocks / WezTerm | `src/modules/terminal/lib/commandMarks.ts`, `rendererPool.getBuffer`, `PaneTreeView.tsx` context menu | OSC-133 ranges + existing context menu + `getBuffer`. Defining daily multi-agent action (grab output to feed another agent). | S |
| 6 | **Zoom/maximize focused pane** | One key (e.g. Mod+Enter) expands the focused pane to fill the tab, same key restores the prior split; small "Z" badge in the header. | tmux `resize-pane -Z` / iTerm2 | `src/modules/terminal/lib/panes.ts`, `src/modules/tabs/lib/useTabs.ts`, `shortcuts.ts`, `spaces/lib/serialize.ts` | A `focusedZoom` leafId flag rendered as one leaf; composes existing pane algebra; add a zoom flag to `SerializedTab` to persist. | S |
| 7 | **Scrollback search: match-count + next/prev cycle** | Add an `n/12 matches` counter and next/prev cycle to the **existing** find UI — do not build a new find bar. | tmux copy-mode / WezTerm | `src/modules/terminal/lib/useTerminalSession.ts`, `rendererPool.ts` (SearchInline) | `SearchAddon` + `SearchInline` + `revealMatch` already shipped; only the counter + cycle is missing. | S |
| 8 | **Dedicated keybinding to open/focus Tasks (and Notes)** | Add a `ShortcutId` + default binding (e.g. Mod+Shift+M for Tasks, Mod+L for Notes — both currently free) since these are reachable only via palette/+menu today. | Zed / VS Code task shortcuts | `src/modules/shortcuts/shortcuts.ts` + `App.tsx` handler map | `tab.newTasks`/`newNotes` commands already exist; palette auto-renders the new key hint. *(Confirm Mod+Shift+M/Mod+L are free.)* | S |

### Tier 2 — nice later

| # | Feature | What it does | Inspiration | Module / files | Reuses | Effort |
|---|---|---|---|---|---|---|
| 9 | **Run/insert from command-history palette mode** | Make selecting a `>` history result actually run in the focused terminal (Enter=run, Alt+Enter=insert to tweak). History mode already inserts via `insertCommand`; the delta is run-on-submit. | VS Code Run Recent Command | `command-palette/hooks/useCommandHistory`, terminal `submitToLeaf` | History search + `>` mode + `submitToLeaf` all exist; only the submit path is stubbed. | S |
| 10 | **Reopen last closed tab** | Palette command + keybind to restore the most recently closed tab (pane tree, title, cwd) from a small capped in-memory stack; PTY respawns fresh. | VS Code / browser | `src/modules/tabs/lib/useTabs.ts`, `spaces/lib/serialize.ts` | `serializeTab`/`hydrateTab` capture the shape; capture on close into a capped stack. | S |
| 11 | **Persist + balance split ratios** | Persist each split's size across restart (today the tree survives but splits snap to 50/50) + an "Even out splits" command. | tmux `select-layout` | `terminal/PaneTreeView.tsx` (ResizablePanelGroup), `spaces/lib/serialize.ts`, command-palette | Add `onLayout` capture + one `SerializedNode` field (test harness locks the shape) + one command. | M |
| 12 | **Last-pane toggle (Mod+;)** | Bounce focus between the two most-recently-focused panes. *(Reshaped: ship only the toggle, drop the explicit mark+dot per skeptics.)* | tmux `last-pane` | `terminal` focus model, `tabs` `focusPane`, `shortcuts.ts` | Track `previousFocus` leafId; reuse existing `focusPane`/`focusPaneDelta`. | S |
| 13 | **Gated desktop notification on needs-input / finished** | One OS notification (pane name) when an agent hits needs-input/done while Terax is unfocused. *(Reshaped: reuse the existing "Coding agent notifications" toggle as the ONLY setting; debounce hard; unfocused-only.)* | zsh-notify / Claude Code Notification hook | `src/modules/agents` (`notify.ts`/`route.ts`, `AgentNotificationsBridge`) + orchestration status | OS-notify plumbing + `agentStore` notifications + `OrchestrationAttentionBridge` transition already exist. | M |
| 14 | **Extended Tasks state: in-progress `[/]`** | Add ONE extra state (in-progress, amber to match `STATUS_META`), cycled by Space. *(Reshaped: skip cancelled/4-state per skeptics.)* | Obsidian extended tasks | `src/modules/workspace-docs/TasksStack.tsx`, `docsStore.ts` | Stores the literal markdown char — no schema change; reuses status palette. | S |
| 15 | **Terminal command snippets (pinned-from-history)** | Saved one-liners injected into the PTY. *(Reshaped: no new store/Snippets group — let a starred/pinned history entry act as the snippet, reusing the history-mode plumbing.)* | iTerm2 snippets (local slice only) | `command-palette` history mode + terminal pty write | Existing `mru`/history idiom; no cloud/team sync. | M |
| 16 | **Activity log: "while you were away"** | A filter/mode over the **existing** `MessageFlowInspector` (started · needs-input · finished + jump-to-pane). *(Reshaped: not a new tab; sequence AFTER #1 and a small status-event source, or it's empty.)* | disler observability (shrunk) | `orchestration/MessageFlowInspector.tsx`, flow log, dock `resolveOpenTarget` | Reverse-chron `FlowEvent` timeline already renders; reuse dock jump-to-pane. **Depends on a FlowEvent status-event source.** | M |

## Deliberately skipped (anti-bloat)

- **Restore-last-session prompt on launch** — `useSpacesBoot` already hydrates the full layout (tabs+panes+notes+tasks) on every boot; a startup prompt duplicates auto-restore and adds friction. (If ever wanted, a one-off "Restore previous session" palette command, never a dialog.)
- **Clickable inline markdown checkboxes in Notes** — shifts Notes from a raw textarea to a rendered surface (UX regression risk); throwaway checklists already belong in the Tasks tab.
- **Yank/copy ring with palette picker** — multi-slot copy ring is occasional power-user territory; the most-recent clipboard covers ~95%, and it overlaps the copy-command-output win (#5).
- **Per-space scratch buffer (separate concept)** — folded into quick-capture (#3): make `>note` default to a per-space scratch docId so there's one concept, not two ways to dump a note.
- **Explicit pane mark + jump-to-mark + header dot** — reduced to the last-pane toggle (#12); the mark/register half is rarely reached when linear cycling + a toggle exist.
- **New graph library, new persistence subsystem, a 4th agent store, cross-tab pane tear-out, cross-pane search, synchronized-input broadcast** — all over the bloat line; the hand-rolled SVG layout, session-scoped orchestration, and per-leaf model are deliberate.

## Debloat cuts (ponytail-audit, 2026-06-19)

Read-only repo-wide over-engineering scan (8 agents, 7 hunters). **28 cuts, net −487 lines / −3 deps.** Nothing applied. Complexity-only — correctness/security/perf were out of scope. Two cuts collide with the agent-visibility work above; decide those first.

### Decide before deleting — "dead" only because a feature was never finished
These are the audit's two highest cuts AND the research's agent-visibility decisions — same code, two readings. Resolve the product call, then either delete or finish.

- **`AgentBusBridge`** — audit: dead-wired reader (nothing writes `agent-bus.jsonl`). = **Open Question #1**: finish the per-pane subagent bus (add a `TERAX_SESSION`-tagged writer) **or** delete the reader and keep subagent surfacing Director-only. `[src/modules/orchestration/components/AgentBusBridge.tsx; App.tsx:375,495-497,2161; orchestration/index.ts:40]`
- **`deriveEdges()` + `TopologyEdge`** — audit: exported but the live graph ignores it. = **Tier 0 "wire or delete"**: render the dashed message-flow edges in `AgentTopologyView`, **or** drop the dead export. `[orchestration/lib/topology.ts:27-59; lib/types.ts:100; index.ts:10,22]`

### Safe cuts — dead Tauri commands (registered, never invoked from JS)
- `ai_http_request` + `HttpResponse` — all HTTP goes through `ai_http_stream`. `[net.rs:312-341,221-226; lib.rs:244]`
- `usage_guard_snapshot` + `UsageSnapshotDto` — usage flows over the `terax:usage-signal` event. `[usage/mod.rs:268-285,240-252; lib.rs:247]`
- `wsl_default_distro` — default already on `wsl_list_distros`' `WslDistro.default`. `[workspace.rs:538-557; lib.rs:231]`

### Safe cuts — dead JS code & exports (zero importers/callers)
- `getSourceControlRemoteIndicator()` + type. `[source-control/useSourceControl.ts:52-58,89-137; index.ts:3]`
- `unlinkByLeaf` store action (teardown uses `removeByLeaf`/`removeWithChildren`). `[orchestrationStore.ts:49-50,179-189]`
- `compactModelMessages` wrapper (only `…Detailed` is used). `[ai/lib/compact.ts:146-151]`
- `preloadLanguages()`. `[editor/lib/languageResolver.ts:197-201]`
- default `proxyFetch` export (`createProxyFetch` is used). `[ai/lib/proxyFetch.ts:57-60]`
- `forEachSlot()`. `[terminal/lib/rendererPool.ts:115]`
- `getMarker` on PromptTracker (kills the marker/registerMarker/dispose bookkeeping too). `[terminal/lib/osc-handlers.ts:38]`
- barrel/export trims: `flushDocs`/`installDocsCrashGuard` re-exports `[workspace-docs/index.ts:5-7]`, `ThemeModePref` alias `[theme/ThemeProvider.tsx:31]`, `SearchInline` value re-export `[header/index.ts:3]`, `PaletteItem.iconUrl` `[command-palette/types.ts:12]`, `DragOverEvent` re-export `[dnd/index.ts:11]`, drop `export` on `ParsedQuery` `[command-palette/lib/mode.ts:3]`, `defaultBoard`/`defaultTaskList` `[docsStore.ts:44,53]`, `SIDEBAR_RAIL_HEIGHT` re-export `[sidebar/index.ts:1]`.

### Safe cuts — dep trims (−3 deps)
- `@radix-ui/react-use-controllable-state` → import from `radix-ui/internal` (umbrella already a dep). `[package.json:65]`
- `tempfile` duplicate under `[dev-dependencies]` — already a normal dep. `[Cargo.toml:54]`
- `grep-matcher` direct dep — transitive via grep-regex/grep-searcher. `[Cargo.toml:32]`

### Safe shrinks (same logic, fewer lines)
- 6× `*Lazy.tsx` → one `lazyDefault(loader, pick)` helper + one-line exports.
- `OrchestrationAttentionBridge` re-inlines focus tracking → reuse `useWindowFocus()`.
- `run_git` wrapper delegates verbatim → fold into `run_git_uncached` (rename, no cache exists).
- `run_blocking_inner` no-logic widener → make `run_blocking` `pub(crate)`.
- `parentDir` copied in 2 explorer hooks → import the existing one from `explorer/lib/watch.ts`.

### Sequencing note
The orchestration cuts (`unlinkByLeaf`, `deriveEdges`, `AgentBusBridge`) and the agent-visibility builds (Tier 1 #1–#2, Open Q#1/#3) touch the same module — do them in one pass so `orchestrationStore`/`topology` aren't edited twice.

## Open questions for Kosta

1. **Per-pane subagent visibility:** do you want the dead `AgentBusBridge` finished (needs a writer that tags subagent lines with `TERAX_SESSION` into `agent-bus.jsonl`), or should we delete the reader and keep subagent surfacing Director-only? This is the one "see the agents" gap that needs a product call.
2. **OSC-777 status path:** the installed Claude Code reportedly ignores the terminalSequence path, so live status transitions may not light up. Want a GUI verification pass before we invest in ambient pane-status tinting (#2), or proceed on the assumption it'll be fixed?
3. **Store unification scope:** beyond bridging `managedAgentsStore` (#1), do you want `agentStore.sessions` collapsed into a thin selector over orchestration (kills the working|waiting parallel store), or leave it for now?
4. **FlowEvent producer:** today only `delegation`/`approval` are written, so `MessageFlowInspector` and the activity log (#16) are mostly empty. Worth a tiny producer that logs `message`/`handoff`/`review` on transitions the bridges already see — yes or defer?
5. **Keybinding choices:** confirm Mod+Shift+M (Tasks), Mod+L (Notes), Mod+Enter (zoom), Mod+Up/Down (command jump), Mod+; (last-pane) don't clash with anything in your muscle memory before I wire defaults.
