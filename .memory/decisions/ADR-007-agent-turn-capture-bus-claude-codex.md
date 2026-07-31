# ADR-007: Agent turn capture via the Koden bus — Claude + Codex hooks, startup install, marker-free Inputs

Status: Accepted — 2026-06-22 (live-confirmed by Kosta for both Claude and Codex)

## Context

The terminal **Inputs** popup (per-pane list of the user's prompts to an agent, ChatGPT-style)
only ever showed the *first* turn of a session. Two further goals landed in the same thread:
make turn capture work for **Codex**, not just Claude, and answer how a fresh Koden install
gets the capture wired at all.

The capture pipeline: an agent's `UserPromptSubmit` hook appends one JSONL line to the
**append-only bus** `~/.koden/director-bus.jsonl`:
`{"cmd":"user-turn","id":<KODEN_SESSION>,"data":<raw hook stdin json>}`. The frontend
`AgentBusBridge` polls the bus (400 ms), routes by pty id via `leafIdForPty`, and calls
`addTurnForLeaf` → `CommandMarks.addTurn` → the Inputs list. `KODEN_SESSION` (pty id) and
`KODEN_TERMINAL=1` are injected on every pty (`session.rs`, `shell_init.rs`). NOTE the
env/path are **`KODEN_*` / `~/.koden`** now — the older INDEX text saying `TERAX_SESSION`
is stale; the rename is done in code.

Diagnosis (all evidence-backed, not guessed):
1. The installed `~/.claude/settings.json` still held **pre-rename TERAX hooks** (gated on
   `$TERAX_TERMINAL`, writing `~/.terax/agent-bus.jsonl`) — inert under Koden. Koden only
   ran `agent_enable_claude_hooks` when *it* launched an agent, never for a manual `cm`.
2. The bus itself was **perfect** — every Claude and Codex turn captured with correct text.
   The loss was purely frontend: `addTurn` anchored each turn to `registerMarker(0)`, and a
   repainting agent TUI drives those marker lines to `-1`, so `getMarks` filtered all but one
   out. Unit tests missed it because the mocked marker never goes negative.
3. The bridge replayed the **entire** append-only bus on every (re)mount, so a previous run's
   turns could bleed into a new pane on the same pty.

## Decision

- **Install hooks on app startup**, not just on Koden-launched agents: a `useEffect` in
  `App.tsx` calls `agent_enable_claude_hooks` + `agent_enable_codex_hooks` + `ensureKodenDir`
  every launch. So manual `cm`/`codex` sessions get the current hooks. (A session must start
  *after* the install — agents read their config at launch.)
- **Migrate stale TERAX hooks**: `OWNED_MARKERS` in `agent.rs` recognizes `notify;Terax` and
  `.terax/agent-bus.jsonl` so a re-install removes them instead of leaving dead cruft.
- **Codex support** (`agent_codex.rs`, new): Codex (≥~0.116, local 0.138) has a
  `UserPromptSubmit` lifecycle hook — stdin JSON with a `prompt` field, a structural twin of
  Claude's. We register a **capture-only** hook (no stdout — Codex treats hook stdout as
  injected context) that appends the *same* bus line, so `AgentBusBridge` needs zero changes.
  Config is **appended** to `~/.codex/config.toml` (never parse+reserialize — preserves the
  user's MCP servers/model settings/comments), idempotent via a marker, atomic temp+rename,
  no-op if `~/.codex` is absent. POSIX inline `command` for mac/linux; Windows uses a tiny
  `~/.koden/koden-codex-turn.ps1` to avoid pwsh/JSON quoting inside TOML.
- **Inputs fix**: store bus turns **marker-free** — a plain `{id,text}` list in a high line
  band (`TURN_LINE_BASE`), arrival-ordered, sorted after real command marks. They carry their
  own prompt text, so no buffer anchor is needed and a repainting TUI can't drop them. The
  scrollback scrape stays the no-signal fallback only.
- **Bridge priming**: adopt the bus's current end on first read (`primed` ref); only process
  appends from then on. No replay of previous runs. (Chosen over truncating the bus, which
  carries live subagent events.)

Alternatives rejected: Codex `notify` (fires at turn-*complete*, kebab-case argv payload —
weaker than the hook); `node`-based Codex hook (end users may lack node on PATH);
`AGENTS.md`/`CLAUDE.md` injection for gist (writes into the user's repo — violates the
no-native-artifacts rule).

## Consequences

- Live-confirmed: Claude **and** Codex turns all list in the Inputs popup, in order. Codex's
  hook env inheritance (the one documented unknown) turned out fine — the bus showed correct
  `KODEN_SESSION` and `prompt`. No Codex hook-trust prompt blocked it in practice.
- Regression test added (`commandMarks.test.ts`): records 3 turns, asserts all 3 surface in
  order — the case the mocked-marker tests structurally couldn't catch.
- **Known caveat — `id:"1"` collision**: every observed bus line was tagged pty id `1`
  (sequential single-pane reuse). Harmless as-is, but two agents in *concurrent* panes would
  collide into one pane. Unverified; revisit pty-id assignment in `session.rs` only if it bites.
- **Deferred — Codex gist injection**: the clean seam is the same hook's stdout (becomes Codex
  developer context) but it's per-turn token cost. Not built.
- The bus is still append-only and never cleared (grows over time); replay is avoided by
  priming, not by trimming. Size-cap within a session remains a follow-up.
- Commits: `4b0fbac` (Claude startup install + TERAX migration), `5d3c803` (Codex hook),
  `876ee70` (marker-free Inputs + bridge priming). Branch
  `overnight/agents-tasks-persistence-2026-06-16`.
