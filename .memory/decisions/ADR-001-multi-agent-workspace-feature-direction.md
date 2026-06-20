---
created: "2026-06-16T01:52:13+02:00"
updated: "2026-06-16T02:30:00+02:00"
temporalSource: git
---

# ADR-001 — Multi-agent workspace feature direction

- **Status:** Proposed
- **Date:** 2026-06-16
- **Scope:** Agent visibility, notifications, tasks, crash-safe docs
- **Supersedes / relates to:** `ROADMAP.md` fork note; the orchestration layer described in `WORKSPACE.md`

This is the first ADR for Terax Workspace. It captures a diagnosis-led design
direction across four loosely-related feature threads the user wants, the root
cause that ties most of them together, and the order to build them in. Individual
threads may spawn their own follow-up ADRs when they get implemented.

All findings below come from a read-only recon pass against the codebase on
2026-06-16 (four parallel Explore agents). Nothing was built. The app was running
live during the recon, so none of this has been verified at runtime yet — see
"Open questions" for the one discrepancy that needs a live check.

---

## Context

The user brainstormed five loose ideas, which collapse into four threads:

1. **Agent visibility** — agents should show in the left panel for *any* running
   terminal session, not only Director / team-template launches. (ideas 1 + 5)
2. **Notifications** — the agent list should be the canonical status board; add a
   real app-level signal and optional desktop notifications. (idea 4)
3. **Tasks** — clickable checkbox tasks, as a dedicated tab kind and/or as
   markdown checkboxes inside notes. (idea 2)
4. **Crash-safe docs** — notes (and future tasks) should survive a power cut like
   Notepad does. (idea 3)

The user's own sequencing instinct was correct: the topology graph can't do
anything useful until agent detection works, so the graph comes last.

### Current architecture (the important nuance)

There are **three separate stores** independently tracking "agent-ish" state, and
they are not unified:

- `modules/orchestration/` — the rich store (Director / workers / native
  subagents, with role, model, tokens, task, activity). Renders the **AgentDock**
  and topology. Has its own roll-up via `statusForDock()` (`orchestration/lib/topology.ts`).
- `modules/tabs/` — `useTabStatusStore` (`tabs/lib/tabStatus.ts`), drives the
  per-tab status **pills**.
- `modules/agents/` — `agentStore` + `NotificationBell` + toasts + desktop
  notifications (`agents/lib/notify.ts`, `agents/lib/route.ts`,
  `agents/components/AgentNotificationsBridge.tsx`).

The `ROADMAP.md` fork note already flagged this split ("bridging
`managedAgentsStore` into the orchestration store"). **The fragmentation plus a
best-effort, late terminal→agent registration path is the cross-cutting root
cause** behind both the empty agent list and the muted notifications.

---

## Thread 1 + 5 — Agent visibility (the foundation)

### What already works

- `src-tauri/src/modules/pty/agent_detect.rs` scans **every** PTY equally and
  auto-arms on a bare `]777;notify;Terax;working` (via `ensure_armed()`, even
  without an OSC 133 shell-prompt marker). Detection is **not** Director-only.
- Status is routed implicitly: Rust knows which PTY emitted the bytes and stamps
  the PTY id onto every signal (`session.rs`, `t.into_signal(id)`), mapped to a
  `leafId` in React via `leafIdForPty()`.
- Plain `claude` terminals **do** auto-register as a worker agent on their first
  signal, in the `terax:agent-signal` listener at `src/app/App.tsx:459-512`.
- Removal works: when a PTY exits, its agent is cleaned up via `removeByLeaf()`
  (`App.tsx:581-607`); Director agents use `removeWithChildren()`.

### The real gaps

1. **Late registration (timing).** The listener only fires on the *first* OSC 777,
   and `working` is emitted by the Claude Code `UserPromptSubmit` hook, i.e. only
   when the user submits a prompt. A freshly-opened `claude` sitting at its prompt
   is invisible until you talk to it. The registration condition is also narrow:
   `App.tsx:478` only matches `started`/`working`, not `attention`/`finished`.
2. **Subagents are Director-only.** `handleDirectorCommand` filters on
   `parentId === directorId` (`App.tsx:1284-1322`), and the bus file
   (`~/.terax/director-bus.jsonl`) only exists for the Director. A plain terminal
   that spawns Task subagents surfaces nothing.
3. **No per-pane identity.** There is a `TERAX_TERMINAL=1` env var injected at
   `src-tauri/src/modules/pty/shell_init.rs` (~105-137, `apply_common`) but **no
   per-pane `TERAX_SESSION`**. Status routes fine without it (implicit by PTY), but
   subagents **cannot** be routed to the right parent terminal without an id on the
   bus lines. The same env-var gate is what would keep global bus hooks as no-ops
   when `claude` runs outside Terax.

### Decision

- **Pre-register a placeholder agent** the moment a terminal leaf is created
  (`src/modules/terminal/lib/useTabs.ts` leaf allocation, or the live-leaf effect
  in `App.tsx:581-607`), status `spawning` / "Agent (starting…)", then let the
  first real signal upgrade it. The existing listener already updates rather than
  duplicates an existing agent, so no double-registration.
- **Inject a per-pane `TERAX_SESSION=<leafId>`** into every spawned PTY; have the
  global Claude Code hooks echo it (in the OSC payload and/or bus lines).
- **Generalize the subagent bus** beyond the Director: same `PreToolUse`+Task →
  bus, `SubagentStop` → retire mechanism, but tagged with `TERAX_SESSION` so lines
  attach to the correct parent. Gate the global hooks on `$env:TERAX_SESSION` so
  non-Terax `claude` sessions stay clean.
- **Bridge the three stores** so the AgentDock is the single source of truth (the
  roadmap's `managedAgentsStore` → orchestration bridge).

### Key files
`agent_detect.rs` (162-215), `session.rs` (99-189), `shell_init.rs` (105-137),
`App.tsx` (459-512 register, 581-607 remove, 1265-1394 director commands),
`modules/orchestration/lib/bus.ts`, `modules/terminal/lib/useTabs.ts`.

---

## Thread 4 — Notifications (mostly built, gated by the foundation)

### Surprise: this is not "just little tab text"

Already present in the code:

- `NotificationBell` with a badge (waiting sessions + unread).
- Desktop notifications via `@tauri-apps/plugin-notification` v2.3.3 (installed in
  both `package.json` and `Cargo.toml` as `tauri-plugin-notification = "2"`),
  through `osNotify()` (`agents/lib/notify.ts:19-25`).
- In-app toasts when focused, OS notification when **unfocused**
  (`agents/lib/route.ts:36-42`).
- A preference toggle: Settings → General → Agents → "Coding agent notifications"
  (`agentNotifications: boolean`, default true). When off, routing early-returns.
- Jump-to-terminal from a notification already works
  (`NotificationBell.tsx:175-178`, `activateNotification()`).

It feels absent because desktop notifications only fire when the window is
unfocused, and the bell counts depend on agents reaching the `attention` state,
which depends on the registration foundation above.

### Genuinely missing

- **Per-tab roll-up.** A tab with N terminals shows *one* agent's status
  (first-encountered), not worst-wins. Need amber > blue > green aggregation to the
  tab pill, and to the app.
- **App-level signal.** No tray icon, no taskbar flash (`requestUserAttention()`
  is unused). This is the "something is waiting even when Terax is minimized" piece.
- **Prompt text.** Only the *state* is captured, never the message. To show
  "which ones do you want me to look into" in the bell, extend the OSC 777 payload
  to carry the text (`notify;Terax;attention;<msg>`) or grab the last N terminal
  lines on `attention`.

### Decision

Build on the foundation: per-tab worst-wins roll-up, app-level attention signal
(taskbar flash + optional tray badge), and prompt-text capture for surface +
jump-to. **In-panel answering** (injecting keystrokes back into the PTY) is a
larger lift and is **deferred to a v2**.

### Status colors (canonical)
amber = needs-input/waiting, blue = working, green = done/idle, red = error.
Consistent across `tabStatus.ts` and `orchestration/lib/roleMeta.ts` (`STATUS_META`).

### Key files
`agents/lib/notify.ts`, `agents/lib/route.ts`,
`agents/components/AgentNotificationsBridge.tsx`,
`agents/components/NotificationBell.tsx`, `tabs/lib/tabStatus.ts`,
`orchestration/components/AgentDock.tsx`, `agent_detect.rs` (OSC payload).

---

## Thread 2 — Tasks (clean, isolated, ~11 files)

### Decision

Add a dedicated `tasks` tab kind, parallel to `notes`/`board`, reusing the
workspace-docs store and its LazyStore persistence. Use a **`TaskItem { id, text,
done }`** model (not the board's stateless `{id, text}` cards).

Markdown checkboxes inside notes is a **separate optional phase**. Notes is
currently a raw `<textarea>` (`NotesStack.tsx:45`) with no rendering, but
Streamdown is already in the codebase, so clickable `- [ ]` / `- [x]` is feasible
via a custom Streamdown checkbox component. The two are not redundant: the tasks
tab is the durable tracked list; inline checkboxes are throwaway capture.

### Keybinding conflict (decision needed)

`Ctrl+Shift+T` is **already bound** to `tab.newBlock` (`shortcuts.ts:102`). Pick a
different chord for "new tasks" (e.g. `Ctrl+Shift+K`) or consciously reassign.

### Surface area (~11 files)
`useTabs.ts` (type + `newTasksTab` factory + `isRenamableKind`),
`spaces/lib/serialize.ts` (union + `isSerializableTab` + `serializeTab` +
`hydrateTab`), `app/components/WorkspaceSurface.tsx` (render mount),
`tabs/TabBar.tsx` (icon), `shortcuts/shortcuts.ts` (binding),
`command-palette/commands.ts` (palette entry), `App.tsx` (wire callbacks, ~4
spots), new `workspace-docs/TasksStack.tsx`, `workspace-docs/store/docsStore.ts`
(task record + mutations), `workspace-docs/index.ts` (export), and optionally
`NotesStack.tsx` for inline checkboxes.

---

## Thread 3 — Crash-safe docs (genuinely unsafe today, isolated, high-value)

### Diagnosis

`modules/workspace-docs/store/docsStore.ts` persists to
`terax-workspace-docs.json` via a Tauri **LazyStore** with `autoSave: 600` (600ms
debounce). Problems:

- **Not atomic.** LazyStore overwrites the full file in place; no temp-file +
  rename.
- **No `beforeunload` flush** for docs. (The spaces module *does* have one —
  `spaces/lib/useSpacePersistence.ts:94` — docs does not.)
- **No `.bak`**, and on load a corrupt JSON is **silently swallowed**
  (`hydrateDocs()` catch sets `hydrated: true` with empty state) → all notes/boards
  silently lost, no recovery, no user alert.

Two failure modes on power cut: (a) lose up to 600ms of typing; (b) far worse, a
cut *mid-write* truncates the whole file → **all notes/boards gone** on next boot.
The same vulnerability affects `terax-spaces.json` (500ms debounce, same engine).

### Decision

Deliver the Notepad-style guarantee:

1. **Atomic write** (temp file + fsync + rename), ideally a Rust command so the
   rename is OS-atomic.
2. **`beforeunload` flush** for docs (copy the spaces pattern; call `store.save()`).
3. **`.bak` rolling backup** + fall back to it on parse failure, and surface a
   recovery notice instead of silently starting empty.
4. Optionally shorten the debounce.

This fix should be written to also cover `terax-spaces.json`, since it shares the
engine and the risk.

### Key files
`workspace-docs/store/docsStore.ts`, `spaces/lib/useSpacePersistence.ts` (pattern
to copy), `settings/store.ts` (explicit-save pattern reference), new Rust atomic
write command under `src-tauri/`.

---

## Cross-cutting root cause

Most of the friction is **one** problem wearing different hats: terminal→agent
registration is best-effort and late, the per-pane identity needed to route
subagents and prompt text does not exist, and three stores track overlapping state
without a single source of truth. Fix that foundation and the agent list, the
notification roll-up, subagents-everywhere, and the graph all become tractable.

---

## Sequencing decision

1. **Persistence (Thread 3) first.** Isolated, low-risk, depends on nothing, and
   protects data the user is creating right now. Best standalone first win.
2. **Agent foundation (Threads 1 + 5).** Pre-register terminal-backed agents,
   per-pane `TERAX_SESSION`, generalize subagents, bridge the stores. Unlocks the
   rest.
3. **Notification polish (Thread 4).** Roll-up, app-level/taskbar/tray signal,
   prompt-text capture.
4. **Tasks (Thread 2).** Fully independent; can slot in anywhere.
5. **Topology graph expansion last** (depends on the foundation).

---

## Open questions / decisions outstanding

- **Live-verify the registration discrepancy.** Static reads say a plain `claude`
  terminal should register on its first signal, but the user observes nothing in
  the panel. Needs a ~30s runtime check (open terminal → run `claude` → send one
  message → watch the Agents panel) to tell "timing gap" from "`leafIdForPty()`
  returning null and silently dropping the signal." Not done — the live session was
  in use during recon.
- **Tasks keybinding** — replacement for the taken `Ctrl+Shift+T`.
- **In-panel prompt answering** — confirmed deferred to v2.

## Implementation constraints

- Every thread needs a **rebuild + relaunch** to test. Per the known dev gotcha,
  the `tauri dev` watcher does **not** reliably rebuild Rust on `agent.rs` /
  `agent_detect.rs` / `session.rs` edits — kill `terax.exe` and relaunch
  `pnpm tauri dev`. So implementation happens at a stopping point, not against a
  live working session.
- Verify per the project bar: `pnpm check-types`, `pnpm lint`, `pnpm test`, and
  `cargo test --lib <mod>` for Rust without relinking the locked binary.
- Known pre-existing test failures (not regressions): `eager-budget.test.ts`
  (env), Rust `authorize_spawn_cwd_blocks_symlink_escape` (Windows symlink priv).

---

## Implementation status (2026-06-16, overnight)

Status moved from **Proposed** to **Partially implemented**. Built on branch
`overnight/agents-tasks-persistence-2026-06-16` (a checkpoint commit of all prior
uncommitted fork WIP was taken first; `main` untouched). All work verified by
`pnpm check-types`, `pnpm lint` (new code clean), and `pnpm test` (256 unit tests
pass; only the pre-existing `eager-budget` env failure remains). **Runtime GUI
verification was NOT performed** — the live Terax dev session could not be safely
relaunched (see below) — so behaviour is logic/type/test-verified, not yet
observed in the running app.

### Diagnosis correction (the linchpin)

The ADR's "open question" — does a plain `claude` register on its first signal —
is resolved, and the root cause was **narrower than feared**:

- `terminalSequence` **is** a real, supported Claude Code hook-output field
  (requires CC ≥ 2.1.141; installed is **2.1.178**, and the native binary
  contains the field). `]777;notify;Terax;…` is an allowlisted OSC. So the hooks
  are correct and OSC 777 **does** reach the PTY; `agent_detect.rs` parses it
  correctly. **The hooks were never the bug.**
- The real gap was purely **late registration**: a node was only created on the
  first OSC 777, which `claude` emits only on prompt submit. A freshly-opened
  shell or an idle `claude` was therefore invisible. The Director worked because
  it is driven by the file bus (`director-bus.jsonl`), not OSC 777.

### Shipped

| Thread | Commit | What shipped |
|---|---|---|
| 3 — crash-safe docs | `556d7fa` | Staggered backup mirror + blur/hide/unload flush + recover-from-backup-on-corrupt (respects a cleanly-empty primary). Frontend-only; effective immediately, no relaunch. |
| 2 — Tasks tab | `556d7fa` | New `tasks` tab kind, clickable checklist, persisted + restored. Command palette + both new-tab menus. **No default keybinding** (Ctrl+Shift+T is taken). |
| 1+5 — agent visibility | `486ab6d` | Pre-register a worker node for every warm, non-note terminal leaf on open (pure tested `terminalsToRegister`). OSC 777 still upgrades status live. |
| 4 — notifications | `4ebc339` | Worst-wins tab-pill roll-up (`escalate`); app-level taskbar flash via `OrchestrationAttentionBridge` + `core:window:allow-request-user-attention` capability. |

### Deferred (need explicit approval — higher blast radius)

- **Per-pane `TERAX_SESSION` + generalized subagent bus + OSC prompt-text.** This
  is the "subagents on *every* terminal" + "prompt text in the bell" half of
  ideas 4/5. It requires editing the **global** `~/.claude/settings.json` hooks,
  which run for *every* `claude` invocation on the machine (not just Terax) —
  too high a blast radius to change unattended. Designed, not applied.
- **In-panel prompt answering** — confirmed v2 (inject keystrokes into the PTY).
- **Inline markdown checkboxes in Notes** — would turn the raw `<textarea>` into a
  rendered surface; a real UX change to existing Notes. Propose separately.
- **Store unification** (`managedAgentsStore`/agents module ↔ orchestration) and
  desktop-notification bridging onto the new foundation.

### Bug found (not fixed — out of scope / user's synced profile)

`teraxFunctionsPs1` emits `Director { <getAgentCommand()> … @args }` and
`getAgentCommand()` currently resolves to **`cm`**. The user's `$PROFILE` `cm`
function runs `& $cmd.Source` **without `@args`**, so the Director's
`--append-system-prompt` and `--agents <roster>` are silently dropped — the
Director runs as a plain `claude` under the user's *global* routing instead of
the steered, session-only team. One-line fix in the profile (`& $cmd.Source @args`)
or have `getAgentCommand()` resolve to the real `claude` binary.

### Open verification (the one thing left)

Relaunch Terax (`pnpm tauri dev` after closing the running instance) and confirm:
open a terminal → it appears immediately in the Agents panel as an idle node;
run `claude`, submit a prompt → it goes blue/working; a permission prompt →
amber + (unfocused) taskbar flash; create a Tasks tab, check items, relaunch →
they persist. The capability + any Rust-affecting change needs the relaunch; the
frontend changes were likely already HMR'd into the running dev session.
