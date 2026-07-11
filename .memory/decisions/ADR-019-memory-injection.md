# ADR-019: Real-time memory injection into agent sessions

Status: Accepted — Kosta, 2026-07-12

## Context

The Brain's gist (ADR-006 P3) reached agents only at LAUNCH time, via
`--append-system-prompt` (Koden-spawned agents) — a session started before a
memory change, or started outside Koden entirely (`claude` in Windows
Terminal), never saw the Brain at all. Memory that the Librarian curates
mid-session (ADR-018 autonomous applies, reverts, reflect rounds, note edits)
was invisible until the next launch. The owner wants live Claude Code sessions
to pick up Brain memory in real time, in ALL his claude sessions.

Candidate channels:

1. **Session-start injection (CLAUDE.md / `--append-system-prompt`)** — no
   mid-session freshness; CLAUDE.md additionally makes derived content look
   user-authored and is read once.
2. **MCP server** — real freshness, but requires per-session/config wiring,
   puts a protocol round-trip on the hot path, and injects via tool results
   (cache-hostile, and only when the agent chooses to call it).
3. **`UserPromptSubmit` hook reading a worker-maintained artifact** — per-turn
   freshness, zero per-session config (the hook is already globally installed
   by Koden in `~/.claude/settings.json`), and the artifact is derived offline
   by the worker so the hook itself is a `cat`.

## Decision

Option 3. The worker maintains, per project, a DERIVED file

```
<project>/.koden-memory/.koden-gist.json
```

containing the COMPLETE `UserPromptSubmit` hook stdout document, pre-escaped by
serde in Rust (never by shell printf — gist bytes carry quotes/newlines/
redacted titles):

```json
{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"<gist markdown>"}}
```

A SECOND Koden-owned `UserPromptSubmit` command group (`agent.rs
gist_inject_hook_cmd`) does a bounded upward search from `$PWD` (max 12
levels), `cat`s the artifact if found, else emits nothing. Naming deviation
from the sketch: `.koden-gist.json` rather than `.gist-hook.json` — the
`koden-` prefix makes the reserved-basename exclusion collision-proof, and the
basename doubles as the hook group's ownership marker.

### Mechanics

- **Content** — the existing cache-stable gist (`build_gist_auto`, blank
  intent → deterministic cold-start synthesis, 800-token budget: the chat
  default). No new gist semantics; the artifact is a serialization AROUND the
  gist, so no `SCHEMA_VERSION` bump (v15 + ADR-019, additive-canonical only).
- **Cache-stability contract** — emission is write-only-if-bytes-differ
  (compare against on-disk bytes; the gist's ADR-006 P3 byte-identity makes
  re-derived bytes the compare key, restart-safe with no stored pin) and
  atomic (sibling temp + rename, LF-only — a CRLF/BOM'd artifact cat'd by the
  POSIX hook would embed literal `\r` inside the stdout JSON). Unchanged
  memory ⇒ untouched file ⇒ byte-identical turn context.
- **Refresh triggers (memory-material seams only)** — autonomous apply sweep
  (`auto_apply_sweep`, covering LibrarianDone/Doctor/Curate/Reflect/boot),
  review-mode approve (`ResolveProposal`), revert, note-file watcher events
  (paths under `.koden-memory/`), rescan convergence (targeted + full), first
  readiness at boot, and a once-a-day Tick check for the UTC-midnight
  overdue-label transition. Deliberately NOT on plain code edits: the full
  gist key folds the temporal digest, which moves on every real edit —
  re-emitting there would churn the file (and each session's turn context)
  near-constantly.
- **Exclusion guarantees (the self-feed hazard)** — the artifact's freshness
  line embeds the project fingerprint, so indexing it would rotate the
  fingerprint on every emit → rewrite → reindex, an unbounded oscillation.
  Exclusions hold at all four seams, keyed on one shared predicate
  (`gist::artifact::is_hook_artifact_name`, prefix-matching so the temp
  sibling is covered): the full walk (`walk.rs is_reserved_artifact`), the
  watcher's single-file gate (`worker::index_changed`; dir-event children
  funnel through the walk), the memory-note scan (explicit skip + non-`.md`
  name), and — transitively via the notes table — reflect digests, intent
  synthesis, the review inbox, and the doctor. The artifact's own write event
  still triggers a note re-scan + re-emit, which converges as a byte-identical
  no-write instead of oscillating.
- **Hook coexistence** — `add_command_group` retained-then-pushed EVERY owned
  group, so a second Koden group per event was impossible. The retain is now
  per ownership CLASS: the gist group (marker = the artifact basename, added
  to `OWNED_MARKERS`) replaces only itself; the status/turn class replaces
  everything else owned (legacy /dev/tty + Terax migration unchanged).
  `agent_claude_hooks_status` now also gates on the gist marker — pre-ADR-019
  installs read "not installed" until the startup auto-install upgrades them.
- **UNGATED by `KODEN_TERMINAL` (deliberate)** — the status/turn hooks gate
  because they need a Koden pane (OSC consumer + bus). Memory injection is
  valuable in ANY terminal; the owner wants it in all his claude sessions, so
  the gist hook runs ungated — plain Windows Terminal sessions get Brain
  context too. That is a feature of this ADR, not an oversight.
- **Multi-project / worktree resolution** — the upward walk stops at the first
  directory owning a `.koden-memory` dir or a `.git` entry (file OR dir, so
  worktrees and submodules count) that lacks an artifact: a nested project
  never inherits the outer project's gist, and a `.claude/worktrees` agent
  (whose tree is never indexed) never injects main-tree context into its stale
  copy. Fail-open everywhere: no artifact ⇒ no output ⇒ exit 0.
- **Toggle** — `brain_librarian.inject_gist` (additive-canonical column,
  default ON), `brain_set_inject_gist` command, switch in the Brain settings
  tab. OFF deletes every artifact and stops regeneration — the hook then finds
  nothing, so sessions never see stale memory. Unregistering a project deletes
  its artifact too (root captured in the `RemoveProject` event before the
  registry entry vanishes).

## Consequences

- Live sessions pick up Librarian applies/reverts/note edits on their next
  turn, in any terminal, with zero per-session configuration.
- A new dot-file appears under each registered project's `.koden-memory/`
  (created if absent). **Gitignore posture: not auto-ignored.** `.koden-memory`
  is deliberately committable; the artifact is derived and safe (gist inputs
  are index-derived and pre-redacted), so we neither write ignore files nor
  hide it. Users who prefer it untracked can add
  `.koden-memory/.koden-gist.json` to `.gitignore`/`.kodenignore` — which the
  existing walker machinery then ALSO honors, harmlessly doubling the
  exclusion seam.
- Stdout contract caveat: each hook process emits ONE JSON document; the
  status group and the gist group are separate processes. Claude Code is
  documented to run all matching hooks and merge `additionalContext`, but the
  repo's own drift note (agent.rs: `terminalSequence` honored only
  intermittently in 2.1.206) is precedent that per-event stdout semantics move
  between CC versions — nothing in-repo can assert the merge, so verify
  against the pinned CC when bumping it. Injection is fail-open if a CC
  version drops it: prompts still work, context is just absent.
- Emission cost is a handful of SQL reads per memory-material event per
  project (bounded by the byte-compare; no watcher re-arm of paid Librarian
  rounds — a fully-gated artifact write yields 0/0 index stats).
- **Codex verdict: SKIPPED in v1 (Claude Code only).** Codex DOES have an
  injection-capable channel — its `UserPromptSubmit` hook stdout is injected
  as developer context — but Koden's installed Codex hook is deliberately
  capture-only (enforced by test), its config block is marker-frozen (a shape
  change requires the delete-block + re-run upgrade path), and the
  stdout-as-context behavior is unverified against current Codex. Wiring it
  is a contained follow-up: emit a PLAIN-TEXT sibling artifact (Codex takes
  raw stdout, not hook JSON) and a new marker-versioned block.
- Follow-up: MCP query access (`brain_search` et al.) for agents that want to
  INTERROGATE the Brain mid-task, complementing this ADR's push channel with a
  pull channel.
