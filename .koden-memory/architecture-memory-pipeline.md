---
id: architecture-memory-pipeline
title: The memory pipeline — ADR chain and verify commands
type: architecture
status: active
created: 2026-07-12
---

# The memory pipeline — ADR chain and verify commands

Koden's memory system shipped across ADR-014..021 (`.memory/decisions/`), all
on `feat/koden-svart`:

- **ADR-014/017** — Svart identity; the chat IS the Librarian (brain tools are
  read-only for memory; workspace-doc writes are approval-gated per the 017
  addendum).
- **ADR-018** — autonomous curation: proposals auto-apply with snapshot undo;
  stacked changes revert NEWEST-FIRST (`NEWER_APPLIED_SIBLING_SQL`).
- **ADR-019** — injection: per-project `.koden-memory/.koden-gist.json`
  (derived, gitignored, byte-compare-gated) + a global UserPromptSubmit hook;
  every claude prompt carries the gist, any terminal.
- **ADR-020** — activity trail (`brain_activity`, redacted at ingest, never
  journaled), exit → targeted rescan + gist refresh, coalesced
  `koden:brain-activity` notifications.
- **ADR-021** — register-on-first-use (canonical-cwd walk, removal
  tombstones in workspace.json, boot re-discovery).

Verify: `cd src-tauri && cargo test --lib` (1 known failure:
`authorize_spawn_cwd_blocks_symlink_escape`, Windows symlink privilege) and
`cargo test --tests --no-fail-fast` (plain `--tests` fail-fasts on that lib
test). Frontend: `pnpm exec tsc --noEmit && pnpm vitest run` (1 known env
failure: `eager-budget.test.ts`). Full history: `.memory/MORNING-REPORT-*`
and `.memory/MERGE-REPORT-2026-07-11.md`.
