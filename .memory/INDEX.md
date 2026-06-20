---
title: terax-workspace — memory index
created: "2026-06-17T19:02:09.000Z"
updated: "2026-06-17T19:02:09.000Z"
---

# terax-workspace — memory index

Retrieval entry point for the Terax fork. Read this first; open linked files on demand.

## Purpose
A fork of `crynta/terax-ai` (a terminal-first, local-first AI-native dev workspace on
**Tauri 2 + Rust + React 19**) that evolves it into a **unified multi-agent workspace** —
adding planning boards, notes, tasks, and agent orchestration on top of the base terminal.

## Current status
In-progress fork (base Terax **v0.8.0**). Active branch
**`overnight/agents-tasks-persistence-2026-06-16`**; `main` left untouched. The four
ADR-001 threads are coded and type/test-verified but **not yet runtime-GUI-verified**;
recent commits pushed the agent topology graph (constellation/forest layout, pan/zoom) and
pane-split work further. On 2026-06-19 five more additions (ADR-002) were built on the same
branch (pane split-dropdown + 4-way splits, per-pane/per-type title colors, smart
clickable/copyable terminal output + selection-visibility fix, ported claude-auto-retry,
graph visual redesign) — static checks all green, **GUI verification still pending**. Nothing
on the branch is committed yet.

**2026-06-20 (overnight):** the fork is being rebranded **Terax → Koden** (KOsta
waDENfalk; "koden" = "the code"). This session shipped terminal/agent UX (history
search + "Find in terminal", scrollback Claude-turn capture, single-sidebar default,
grid launcher, tab/pane context menus, AGENTS group/filter, hover scrollbar), fixed
the GLM subagent-visibility bug (`AgentBusBridge` now recovers subagents from corrupt
`agent-bus.jsonl` by `tool_use_id`), cut the "Ask Terax" popup (gesture kept), and
executed the Koden **identity rename** (all user-facing strings → "Koden"; bundle id +
every runtime contract KEPT as the internal `terax` codename so user data is not
orphaned; crynta attribution preserved). The auto-updater is repointed at `MrOlof/koden`
behind a default-off `autoUpdateCheck` pref (no upstream footgun). All static-verified
(tsc + 360 vitest). HELD for Kosta: bundle-id change (resets appdata), minting the
minisign key + creating the repo + publishing, contract migration, cutting Whisper/Mod+J.

## Key decisions (`decisions/`)
- **ADR-004 — Conductr as Koden's upkeep/memory daemon** *(Proposed 2026-06-20; pointer — canonical = `Conductr/.memory/decisions/ADR-033`)*. Host Conductr's milestone-driven upkeep (librarian + code index) inside Koden's resident Tauri/Rust backend — no standalone CLI daemon, no cron; v1 = Tauri sidecar spawning `conductr maintain --if-milestone` on agent-session-end/commit/debounced-save. **Sequencing-gated:** land both projects off their feature branches first.
- **ADR-001 — Multi-agent workspace feature direction** *(Proposed → partially implemented)*.
  One cross-cutting root cause: terminal→agent registration is best-effort and late, there
  is no per-pane identity, and three separate stores track agent state without a single
  source of truth. Plan, in order: crash-safe docs persistence → agent-visibility foundation
  (pre-register a placeholder agent per terminal leaf, inject `TERAX_SESSION=<leafId>`,
  generalize the subagent bus) → notification roll-up + app-level signal → Tasks tab →
  topology graph last. **Shipped:** crash-safe docs, Tasks tab, agent pre-registration,
  worst-wins tab roll-up + taskbar flash.
- **ADR-003 — Usage guard, retry fix, command minimap + ADR-002 iterations** *(Accepted;
  implemented + statically verified 2026-06-19 overnight, GUI/real-run pending)*. Proactive
  usage guard (Rust OAuth-usage poller + time-fallback + soft spawn-gate), reactive auto-retry
  FIXED for Claude Code v2.1.168 (modern banner + Windows TZ + Esc menu-dismiss), command
  minimap (OSC-133 tick strip), OKLCH readable pane colors, configurable smart-link categories,
  graph focus/lock, visible scrollbar, + a fake-claude sandbox harness (`scripts/`). No new deps.
- **ADR-002 — Five workspace additions** *(Accepted; implemented + statically verified,
  GUI-verification pending)*. Pane split-dropdown (always-on header; type×direction;
  `sideToSplit`/before-insert), per-pane title colors + per-type default prefs (renamePane
  color-loss bug fixed; persisted via serialize reader/seeder), smart link providers
  (paths→reveal, secrets→copy; selection-alpha fix), claude-auto-retry **ported** off tmux
  to a Rust `retry_detect` per-session detector + JS `RetryBridge`/`retryStore` (per-tab,
  cap 3), and the topology graph restyle to the AgentDock idiom. No new deps.
- The **orchestration store is the authoritative model** (driven by real user actions +
  terminal-agent link, persisted to `terax-orchestration.json`).
- Status color convention: **amber** = needs-input/waiting, **blue** = working,
  **green** = done/idle, **red** = error.

## Important files / docs
- `TERAX.md` — base Terax architecture (two-process Tauri model, PTY, AI subsystem); read first. `CLAUDE.md` and `AGENTS.md` both just point here.
- `WORKSPACE.md` — the definitive spec of what the fork adds (orchestration spine, Agent Dock, Topology, Flow Inspector, Director, persistence).
- `decisions/ADR-001-multi-agent-workspace-feature-direction.md` — design + shipped/deferred table + open verification.
- `feature-backlog.md` — 12 proposed (unbuilt) features with effort sizes.
- **`koden-overhaul-plan-2026-06-20.md`** — Koden rebrand + soft-update-channel + bloat execution plan (decisions-needed box up top).
- **`audit-verification-2026-06-20.md`** — verification of the two 2026-06-19 research baselines against the current tree (done / stale / flipped).
- **`koden-update-channel-setup.md`** — actionable checklist to stand up the signed Koden update feed (mint key, CI secrets, test release).
- `feature-research-2026-06-19.md`, `fork-rebrand-and-onboarding-2026-06-19.md` — original research baselines (now partly superseded by the two dated docs above).
- `ROADMAP.md`, `README.md`, `package.json` (stack source of truth).

## Retrieval hints
- **Use this memory for:** the why / what / order of the multi-agent fork, the orchestration
  architecture and its three-store fragmentation, source-file pointers, what shipped
  overnight vs. deferred, and the proposed backlog.
- **Do not use it for:** runtime-verified behavior (ADR-001 explicitly says the GUI was
  never verified) or upstream base-Terax internals (those live in `TERAX.md`).

## Open questions / known gaps
- Live-verify agent registration + Tasks persistence in the GUI (not yet done).
- Tasks keybinding (the default `Ctrl+Shift+T` is taken).
- In-panel prompt answering — deferred to v2.
- Global-hooks phase 2: per-pane `TERAX_SESSION` is injected and the subagent bus is now WIRED + resilient (`AgentBusBridge` + `subagentBus.ts` recover subagents from corrupt `agent-bus.jsonl` by `tool_use_id`). Remaining: the subagent-start hook in `~/.claude/settings.json` is non-atomic (reader recovers; the writer still corrupts on parallel spawns), the dual-installer drift (`agent.rs` still writes the legacy OSC-777/director-bus path + stale `OWNED_MARKERS`), and store unification.
- ~~`getAgentCommand()` drops `@args`~~ **FIXED 2026-06-20**: `getAgentCommandWithArgs()` swaps `cm`→`claude` when flags are present so `--append-system-prompt`/`--agents` survive; the plain no-arg launch is unchanged.
- Known non-regression test failures: `eager-budget.test.ts` (env), Rust `authorize_spawn_cwd_blocks_symlink_escape` (Windows symlink privilege).
