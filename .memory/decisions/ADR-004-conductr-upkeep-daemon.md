# ADR-004: Conductr embedded in Koden as the upkeep / memory daemon (pointer)

Status: **Superseded — 2026-06-20** by **ADR-005 — Koden Brain integration**
(`./ADR-005-koden-brain-integration.md`), which is the canonical record. ADR-005 reframes this as a
Koden-branded subsystem ("Koden Brain", Conductr invisible to users) and corrects the design after a
code-verified pass: stdio MCP child (not an `externalBin` sidecar), a `std::thread` daemon (not tokio),
and no new `maintain --if-milestone` gate for v1. Everything below is retained for history only.

## Summary

Host Conductr's milestone-driven upkeep (memory librarian + code indexing) **inside Koden's resident
Tauri/Rust backend**, instead of a standalone CLI daemon or a clock-based cron.

Why Koden is the right host: it is a **Tauri app** with an already-resident Rust backend whenever it's
open, so the "daemon" is just a tokio background task scoped to app lifetime — no OS service, no
supervision, no keepalive. Koden also orchestrates the Claude/Codex agent sessions itself, so it owns
first-class **milestone signals** (agent-task-complete, session-end) that a bare CLI + git hooks can't
see. The standalone-CLI-daemon objection (a process to install and supervise) does not apply here.

## Shape (v1)

- Rust backend detects a milestone (agent-session-end / commit / debounced save crossing a threshold)
  → cheap **synchronous gate** → if it trips, spawn upkeep **detached** (never block UI/commit).
- **Wiring:** Tauri **sidecar** — bundle the `conductr` binary as `externalBin`, spawn
  `conductr maintain --if-milestone` (a small new Conductr gate command). Reuses Koden's existing
  agent-process spawning. MCP-client wiring is v2.
- **v1 scope:** gated milestone upkeep only — **not** continuous real-time re-indexing.

## Gates before building (do not start the seam yet)

- **Sequencing:** both projects are on feature branches (Koden on
  `overnight/agents-tasks-persistence-2026-06-16`, main untouched; Conductr `dist/` unrebuilt, 31
  unpushed commits). Land/merge each to a stable state first, then integrate — else every bug is
  ambiguous across Koden / Conductr / the seam.
- **Conductr prerequisite:** the memory half (librarian/reflect) has no `.rulesync/memory/` corpus on
  this machine yet → 0 notes until bridged. The code-index half works regardless.
- **Rebrand note:** per Koden's CLAUDE.md, internal identifiers stay `terax` (env/bus/store). The
  sidecar + gate command are new external surfaces, unaffected by the rename.

See the Conductr ADR-033 for full alternatives (CLI daemon / git-hooks-only / cron / MCP-first — and
why each lost), consequences, and the open-work checklist.
