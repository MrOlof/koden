# ADR-005: Koden Brain — workspace memory + code intelligence integration

Status: **Superseded — 2026-06-20** by **ADR-006 — Koden Brain native in-process architecture**
(`./ADR-006-koden-brain-native-architecture.md`). ADR-005 still wrapped Conductr as a Koden-managed
**Node child over stdio MCP**; after a 16-agent research pass the decision reversed to a **native Rust,
in-process** brain (no Node, no subprocess, no MCP) that borrows Conductr's mechanisms rather than running
its code. Retained for history. Original status was "Accepted (architecture)"; it superseded ADR-004 and
Conductr ADR-033 (both also remain Superseded).

> **Branding (load-bearing).** The brain is **"Koden Brain"** in every user-facing surface — pane
> name, settings, wizard copy, docs. The word **"Conductr" never appears to a Koden user.** Conductr
> remains Kosta's separate upstream project; Koden consumes a build of its source as the engine behind
> "Koden Brain." Internal storage dirs (`.conductr`, `.rulesync`) are implementation detail for v1
> (cosmetic rename deferred — it's an upstream change).

## Context

Koden runs 5–10 agent terminals (Claude Code / Codex / Gemini / GLM). The goal is to make it the
"ultimate agentic workspace": a single root workspace as source of truth, a brain that knows every
project in it (memory + code intelligence), token-saving context injected into every new agent flow,
a background upkeep daemon on its own key, and crash-resilient resume (driven by a real power-outage
that lost multi-terminal Claude/Codex state).

Conductr already implements the brain: a stdio MCP server (`conductr mcp`, stdout-pure per its ADR-026)
exposing `brain_context`, `code_search`, `code_graph`, `code_impact`, `memory_search`, `temporal_changes`
(read-only) + `memory_propose`; a free/offline lexical BM25 index + regex-derived code/memory graphs;
a token-bounded "gist"; and `import --global` to seed memory. **Index + gist cost zero tokens; only
memory-reflect calls an LLM.** Storage relocates via `CONDUCTR_SYNC_ROOT` env + `--input-root` flag.

Three verified constraints shaped the design:
1. **Koden is GUI-resident only** (no tray/headless) — **accepted by design**: Koden Brain starts when
   the workspace opens and runs while it's open. No always-on daemon needed.
2. **Koden has no in-process Node.** Even vendored Conductr source must run as a child process. So
   "from source, in-code" means *Koden builds and owns the engine and drives its real `mcp` server over
   a clean pipe* — not a Rust port, not an opaque shipped binary, and not direct function calls
   (Conductr's `src/index.ts` exports only `generate/import/convert` — the brain tools are reachable
   only via the MCP boundary today).
3. **Silent BYOK burn is a known scar** — the upkeep daemon must run on its own key with a visible budget
   and never auto-apply.

## Decision

**Run Koden Brain as a Koden-managed long-lived child process** (`node dist/cli/index.js mcp`) using the
existing child-supervisor pattern in `src-tauri/src/modules/shell/background.rs` (`SharedChild` +
`Stdio::piped` + `BoundedRingBuffer` + kill-on-Drop + Windows Job-object) — **not** `portable-pty` (a PTY
corrupts newline-delimited JSON-RPC), **not** `tauri-plugin-shell`/`externalBin` as the boundary. A minimal
in-Rust stdio MCP client (initialize / tools-list / tools-call; `serde_json`; id→oneshot map on a named
reader thread) exposes the brain to the webview as typed `#[tauri::command]`s.

**Decisions locked (2026-06-20):**
- **Boundary:** stdio MCP child (rejected: Rust port = months + throws away the engine; widening Conductr
  exports = still a process boundary anyway, deferred to a possible v2).
- **Node runtime:** dev-machine only for v1 — discovered `node>=22` on PATH (free on Kosta's mise box).
  SEA/bun self-contained bundle is the *shipping* fallback only, deferred until Koden goes public.
- **Librarian power:** **lexical-only, no LLM key for v1.** Index/gist/search are free/offline and deliver
  the core value. The own-key memory-reflect path lands in P4 behind a visible budget.
- **Brand:** Koden Brain everywhere user-facing; Conductr invisible.

### Phased build plan

| Phase | Goal | Key items | Gate |
|---|---|---|---|
| **P0** | Lock contracts | Freeze `NormalizedAgentEvent` schema `{schemaVersion, session(KODEN_SESSION), agent(claude\|codex\|gemini\|glm\|unknown), source(osc133\|process\|osc777\|hook-<agent>), kind(started\|working\|attention\|finished\|exited\|milestone), exitCode?, subagentId?, milestone?, ts}`; keyring layout (terminals=`koden-ai`, librarian=separate service e.g. `koden-brain`); confirm decisions above | Schema written down (keystone for 4 subsystems) |
| **P1** | Resident brain (reads, zero-token) | New `src-tauri/src/modules/conductr/{mod,service}.rs` modeled on `background.rs`; in-Rust stdio MCP client; `#[tauri::command] koden_brain_*` passthroughs; start from `.setup()` gated on known root; `.manage(BrainState)` | `koden_brain_context(query)` returns a gist over the live pipe; zero tokens; no orphaned node.exe on root-switch (Windows); malformed stdout line doesn't crash |
| **P2** | Setup wizard + root workspace | Add `tauri-plugin-dialog` (missing); 4-step wizard (`src/modules/onboarding`); persist `{rootWorkspace,setupCompleted,registeredProjects}` via `tauri-plugin-store`; one-shot `conductr init`/`import --global`/`code index` with `cwd=root --input-root=root CONDUCTR_SYNC_ROOT=root --json`; project discovery (scan root depth-1 via the `ignore` crate for `.git`/manifest markers) | Fresh boot lands `.rulesync`+`.conductr` UNDER chosen root; wizard shows "Seeded N notes", refuses N==0 |
| **P3** | Agnostic grounding + Brain pane | Before any agent PTY spawn, call `koden_brain_context` and concat the gist into the existing `--append-system-prompt` prompt-file (vendor-agnostic on day one); "Koden Brain" pane (non-PTY) showing status dot + stderr ring buffer + a brain query box | Launching codex OR gemini in a registered project shows gist in its system prompt; pane shows live status proving "first-class, not a black box" |
| **P4** | Own-key upkeep daemon + visible budget | New `src-tauri/src/modules/upkeep/` cloned from `usage/poll.rs::spawn_poller`; subscribe to existing `agent_detect` transitions; cost classifier (free `code refresh`/`brain_context` on-hook vs long-debounced paid runs, hard 600s cap, never on a sync hook); separate keyring key → `CONDUCTR_LLM_*` env on the child only (never inherit `ANTHROPIC_API_KEY`); `koden:upkeep-signal` budget event; **never** calls `apply-proposals` (proposals stay gitignored, human-gated) | Milestone storm → no overlapping paid runs (single-runner + debounce + hourly cap); budget indicator visible before ship; cap exceed downgrades to free ops |
| **P5** | Crash-resilient resume | Tier 1 (agent-agnostic, ships alone): reader thread appends `NormalizedAgentEvent` JSON lines (O_APPEND) to `~/.koden/checkpoints/`, keyed by **durable key `cwd+agent`** (NOT pty/leaf id — both reset on restart); RecoveryBridge extends `AgentBusBridge` tail-cursor + `subagentBus` tolerant-parse to show recovery cards by cold tab. Tier 2 (gated on `agent==claude` + boot CC-version probe): map `cwd`→`~/.claude/projects/<enc>/<uuid>.jsonl`, add `--resume` slot to `agentCommand.ts` | Power-cut sim loses ≤ last partial line; recovery card next to matching cold tab; Tier-2 Resume only for verified claude session |
| **P6** | Per-vendor hook adapters (XL, last) | Refactor `agent.rs` (Claude OSC-777/`~/.claude/settings.json`) into a `HookInstaller` trait; config-driven `AgentRegistry`; file-bus as the single normalized sink; new Codex/Gemini/GLM adapters (each verifies live config vs a scratch HOME, no clobber) | Schema drift fixed end-to-end for claude first; each adapter tested against scratch HOME; no-hook agents degrade to events-only |

**Sequencing insight:** P1+P2+P3 deliver the core vision (brain + workspace + agnostic grounding) before
any daemon or resume exists. P4/P5 build on that foundation; P6 never blocks earlier phases.

## Consequences

- **Reuse-heavy:** the supervisor (`background.rs`), the background-thread template (`poll.rs`), the
  durable-log tail+recovery (`AgentBusBridge`/`subagentBus`), the prompt-file injection, and Conductr's
  entire engine are reused. Genuinely new: the Rust MCP client, the wizard + `tauri-plugin-dialog`, the
  upkeep daemon, the checkpoint sink, and (P6) the vendor hook adapters.
- **Top risks:** PTY-vs-pipe corruption (must use piped Stdio, not portable-pty); `inputRoot` vs
  `CONDUCTR_SYNC_ROOT` (the `.conductr` index derives from `inputRoot`/cwd, so EVERY spawn must pass
  `--input-root` too); key isolation (child env must carry ONLY the librarian `CONDUCTR_LLM_*`, never the
  terminal agent's key); 0-notes trap (empty corpus reports healthy-but-empty — verify seed count and
  fail loud); per-vendor hook surfaces are version-volatile (verify live, never trust memory).
- **Open decisions (non-blocking for P0–P3):** process topology for multiple windows/roots (one child per
  active root recommended); whether to re-advertise brain tools to in-terminal agents as their own MCP
  endpoint (v2); daemon idle/cooldown defaults + hard caps; Koden-space↔project mapping (decoupled
  recommended); checkpoint location (`~/.koden` machine-global recommended); GLM client identity (distinct
  z.ai CLI vs Claude-Code pointed at a compatible base URL); where the budget HUD lives.
- This supersedes ADR-004/ADR-033's "tokio task + `maintain --if-milestone` sidecar" shape: the daemon is
  a `std::thread` (not tokio), the boundary is a managed MCP child (not `externalBin`), and the new
  Conductr `maintain` gate command is **not needed** for v1 (Koden's own milestone signals + one-shot
  child invocations cover it).
