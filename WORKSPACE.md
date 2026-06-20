# WORKSPACE.md

This fork evolves **Koden** (the lightweight AI-native terminal, see `KODEN.md`)
into a **unified AI workspace for multi-agent development** — terminal, planning
board, notes, and multi-agent orchestration in one local-first app. It keeps
Koden's speed, small bundle, local-first architecture, and terminal-centric
workflow, and adds a workspace layer on top.

Read `KODEN.md` first for the base architecture (two-process Tauri model, PTY
integration, AI subsystem, conventions). This file documents only what the fork
adds. The same quality bar applies: pure functional core + thin shell,
dependency-light, no heavy imports in the startup graph.

## What the fork adds

### 1. Terminal UX — desktop clipboard defaults

`modules/terminal/lib/keymap.ts` gained `terminalClipboardAction()` and
`isTerminalShiftEnter()` (both pure, both tested in `keymap.test.ts`). Wired in
`rendererPool.ts`'s `attachCustomKeyEventHandler`:

- **Ctrl+C** copies when there is a selection, otherwise falls through so xterm
  still sends SIGINT (`\x03`). Copy also clears the selection, so a second
  Ctrl+C interrupts.
- **Ctrl+X** cuts when there is a selection (terminal text is read-only, so this
  copies and drops the selection), otherwise falls through (`\x18`).
- **Ctrl+V** always pastes (bracketed paste via `term.paste`).
- **Ctrl+Shift+C / Ctrl+Shift+V** remain explicit copy / paste.
- **Shift+Enter** inserts a newline without executing.
- macOS keeps its native Cmd-based clipboard; Ctrl is never intercepted there.

### 2. Flexible layout — top vs sidebar tabs

`modules/tabs/lib/useLayoutMode.ts` is a localStorage-backed shell setting
(`"top" | "sidebar"`), matching how sidebar width/view are persisted (not the
settings-window Preferences schema). In `"sidebar"` mode, `App.tsx` renders a
resizable `VerticalTabs` panel (VS Code-style vertical workspace navigation) in
the main `ResizablePanelGroup` and hides the header tab strip (`Header`
`hideTabs`). Toggle from the command palette ("Toggle tab layout").

### 3. Non-terminal workspace tabs — `modules/workspace-docs/`

Two new tab kinds, content persisted in `koden-workspace-docs.json`
(`tauri-plugin-store`) via `docsStore.ts`, independent of the per-space tab
structure so reopening a tab restores its content:

- **`notes`** — Markdown scratchpad (`NotesStack.tsx`). One doc per tab, keyed by
  `docId`.
- **`board`** — kanban to-do / progress board (`BoardStack.tsx`). Columns with
  cards: add, inline edit, delete, move between columns, rename columns. Keyed
  by `boardId`.

Both are creatable from the header `+` menu and command palette, and renamable
(the TabBar rename path now covers any `isRenamableKind` tab, not just
terminals).

### 4. Orchestration core — `modules/orchestration/`

The spine for the multi-agent features. Pure, dependency-light core +
zustand store + persistence:

- `lib/types.ts` — `Agent` (role, status, task, model, token usage, cost,
  context/cost limits, permissions, tools, parent link, terminal link),
  `FlowEvent` (message / delegation / handoff / decision / review / audit /
  approval), `TopologyEdge`, plus the role and status enums.
- `lib/roles.ts` — per-role default config (model + permissions + tools), e.g.
  Coder gets a high-capability model and write/shell perms; Auditor a cheap
  model and a read-only tool surface.
- `lib/topology.ts` — pure derivations: `deriveEdges` (ownership + aggregated
  message-flow edges), `countActive`, `totalTokens`, `sortAgentsForDock`.
  Tested in `topology.test.ts`.
- `lib/roleMeta.ts` — presentation (icon, accent, tier) + `formatTokens` /
  `formatRelativeTime` (tested in `roleMeta.test.ts`).
- `store/orchestrationStore.ts` — the live store: `spawn`, `assign`, `setStatus`,
  `setTask`, `addTokens`, `updateConfig`, `linkTerminal`, `logFlow`, `remove`.
  Persisted to `koden-orchestration.json` (capped flow log, debounced
  autosave). `hydrateOrchestration()` is called once from `App.tsx`.

### 5. Agent Dock (sidebar) — `AgentDock.tsx`

A third sidebar view (`SidebarRail` gained "Agents", badge = agent count).
Lists every agent with role, status, model, current task, token usage, and
last activity. Lives in the sidebar so it stays visible regardless of the active
tab. Clicking an agent with a linked terminal activates its tab.

### 6. Agent Topology view — `AgentTopologyView.tsx`

A `agent-topology` tab. Layered graph (director on top, then architects /
reviewers / auditors / qa / devops, then workers) with SVG edges: solid =
ownership, dashed (weighted) = recent message flow. Status shown as a pulsing
dot. Nodes activate their terminal tab.

### 7. Message Flow Inspector — `MessageFlowInspector.tsx`

A `message-flow` tab. A readable, filterable timeline of agent conversations,
delegations, handoffs, decisions, reviews, audits, and approvals — rendered from
the orchestration flow log, not terminal output.

### 8. Director — `DirectorView.tsx`

A `director` tab, the primary command interface. Ensures a single root Director
agent exists, then lets you spawn agents (role, name, task, model, "run in a
terminal tab"), assign / route tasks, change status, approve work (logs an
approval flow event), remove agents, and edit per-agent configuration (model,
context limit, cost limit, permissions, tools). "Run in a terminal tab" opens an
agent terminal via the existing `newAgentTab` path and links the orchestration
record to that leaf.

### 9. Persistence

Spaces already persist their open tabs. `spaces/lib/serialize.ts` now
(de)serializes the new tab kinds (notes/board by id + title, orchestration views
by kind + title). Notes/board content lives in `koden-workspace-docs.json`; the
agent roster + flow log in `koden-orchestration.json`; layout mode + sidebar
view in localStorage. So layouts, agents, notes, boards, graph views, and open
terminals all survive a restart.

## Module map (additions)

```
modules/orchestration/      multi-agent spine + views (dock, topology, flow, director)
modules/workspace-docs/     notes + kanban board tab kinds, persisted content store
modules/tabs/VerticalTabs   sidebar-layout vertical tab rail
modules/tabs/lib/useLayoutMode  top/sidebar layout toggle
```

## Wired vs roadmap

Wired and working today: terminal clipboard UX, layout modes, notes, boards,
the agent dock, topology / flow / director views, the orchestration store
(spawn / assign / route / review / approve all mutate real state and persist),
and terminal-agent spawning that links a real PTY tab to an orchestration agent.

The store is the **authoritative model** and is driven by real user actions
(the Director) plus the terminal-agent link. What remains for a future pass —
documented in `ROADMAP.md`:

- Live token/cost metering: stream real usage from the AI SDK and Claude Code
  terminal agents into `addTokens`, and enforce the context/cost limits.
- Autonomous routing: have the Director's own agent actually delegate and route
  via the AI agent loop, rather than only via the user driving the dashboard.
- Bridge the existing `managedAgentsStore` (terminal coding-agents) into the
  orchestration store so externally-launched agents appear automatically.
- Per-space scoping of the orchestration roster.
- Drag-and-drop on the board and topology graph; richer node layout.
