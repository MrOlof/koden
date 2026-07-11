# Merge report — feat/koden-brain × feat/koden-svart (2026-07-11)

**TLDR: merged, fully gated, adversarially reviewed, runtime-smoked — GREEN.
Everything both sessions built this week now lives on `feat/koden-svart`
(HEAD `7419bc6`, merge commit `307cd73`, safety pointer
`backup/svart-pre-merge`). Nothing pushed.**

## What got merged

- **Svart side (23 commits):** the design identity (ADR-014), Librarian chat
  (ADR-017 + your gist/review-inbox/self-report/brain_status tools + the
  self-contained Librarian window), settings restructure, hook-pipeline fix,
  click-to-scroll, Workspace rename.
- **Brain side (10 commits):** librarian offload off the worker thread,
  sidecar journal (`store/journal.rs`), proposal application
  (`memory/apply.rs` — approving now APPLIES memory), digest-pin persistence,
  proposal dedup, fs-walker gitignore parity, brain_cli harness, and the
  GUI-validation frontend fixes (live inbox polling, Ready-after-rescan,
  resolvable targets).
- Zero textual conflicts — verified afterward that the two sides changed
  **strictly disjoint file sets** (the one shared file, BrainPane.tsx, was
  brain-side-only).

## Verification (all on the merged tree)

| Gate | Result |
|---|---|
| `tsc --noEmit` | clean |
| `vitest run` | **398/398** (only the known eager-budget env file) |
| `pnpm build` | green |
| `cargo test --lib` | **424 passed** (1 known symlink-priv env failure) |
| `cargo test --tests --no-fail-fast` | **all 15 integration targets green** — brain_apply 7, brain_bench 4, brain_changes 6, brain_journal 4, brain_plan 5, brain_precision 1, brain_sandbox 51, brain_temporal 9, fs_search 29, git_operations 25, librarian_offload 2, librarian_pin_persist 1, librarian_rounds 3, secret_index 1 |
| Runtime smoke (CDP, merged binary) | boots; Svart tokens + Commit Mono live in BOTH webviews; settings tabs `General·Themes·Terminal·Shortcuts·Models·Librarian·Brain·About`; brain worker indexed all projects on boot; new parent-stamped hooks installed |

**Adversarial reviews (2 max-effort agents):**
- *Merge semantics:* every frontend `invoke()` matched against the Rust
  `invoke_handler` (all commands exist, arg names match after Tauri v2 case
  conversion — including your four new chat tools against the changed
  `brain_resolve_proposal` async semantics). No chat-vs-inbox double-apply:
  the chat's proposals tool is read-only, `brainResolveProposal` is called
  only from BrainPane, Rust apply is idempotent + single-writer serialized.
- *Invariants:* 16/16 verified — ADR-014 §7 (all eight), ADR-017 (no
  chat-side memory mutation; model-default gating survived), SCHEMA_VERSION
  15 consistent across both parents, secrets-redaction chokepoint still on
  the index path, journal+apply wired into store init, token discipline
  clean across all post-ADR commits including yours.

**Fixed from review (`7419bc6`):** Librarian tool descriptions steered the
model into bare `read_file` calls with project-relative paths (resolve
against terminal cwd → wrong file in multi-project workspaces) — now say to
join the project root from brain_status; bindings.ts IPC-casing comment was
factually wrong.

## Known / flagged (no action taken)

1. **kbd chip hex** (`kbd.tsx:8`): fixed dark values per ADR-014 §4 —
   renders dark under light themes. Themable = §7.7 triple-sync work.
2. **ADR numbering collision:** brain session's offload ADR is "ADR-014" in
   its commit message; the committed `.memory/decisions/ADR-014-*` file is
   Svart identity. Brain-side ADR docs are still UNCOMMITTED in the main
   tree (with INDEX.md edits) — reconcile numbering when committing those.
3. Main tree (`terax-workspace`) still has the other session's uncommitted
   `.memory` work + is checked out on `feat/koden-brain`.
4. Registered-but-unused Rust commands (brain_write_gist, plan_context, etc.)
   — harmless surplus, future chat-tool candidates.

## Branch topology for you

`feat/koden-svart` now contains EVERYTHING. When happy:
`git checkout feat/koden-brain && git merge feat/koden-svart` (or just rename
svart to your mainline). The worktree can be folded back whenever —
`git worktree remove` after switching the main tree to the merged branch.
