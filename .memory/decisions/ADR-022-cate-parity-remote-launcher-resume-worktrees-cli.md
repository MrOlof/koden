# ADR-022: Cate parity in one pass: SSH env, launcher, resume + scrollback, worktree Spaces, koden CLI

Status: Accepted (Kosta, 2026-08-30), built the same day by five parallel worktree agents and merged on `main`

## Context

On 2026-08-29 Kosta reversed the 2026-08-27 call: Koden stays the daily shell,
Cate (0-AI-UG/cate) is only the bridge for the workbench box until Koden can
open a remote workspace itself. The deep dive `.memory/cate-deep-dive-2026-08-30.md`
lists what Cate actually does under its README headlines. Kosta's ask on
2026-08-30: "every time you open up Koden you should have something similar to
Cate, where you can choose whether to establish a remote connection or open a
new terminal", plus worktrees, git, and "spawn at least five agents in parallel,
we are on a tight schedule". The remote design brief predates this ADR:
`C:\Users\Snorlax\Snorlax\To-Do\Workbench-Setup\KODEN-REMOTE.md`.

## Decision

Five packages, one shared seed, built in parallel worktrees and merged in the
order WP1, WP4, WP2, WP3, WP5. Each keeps Koden's shape: pure core + thin shell,
no new dependencies, `WorkspaceEnv` stays the single environment seam.

| Package | Ships | Key choices |
|---|---|---|
| Seed | `WorkspaceEnv` gains `{kind:"ssh", host, path}` in TS and Rust; scope key `ssh:<host>` | One type every package compiles against |
| WP1 SSH env | `src-tauri/src/modules/ssh.rs` (parse `~/.ssh/config`, strict host validation, timed exec helper, `ssh_list_hosts`, `ssh_home`), `pty/shell_ssh.rs` (rc bundle pushed once per content hash to `~/.koden/shell`, pure `remote_command` builder, optional tmux), env selector on every platform, `require_local_fs` gate on all `fs_*`, git `canonical_dir` and `authorize_spawn_cwd` | **System OpenSSH client in the PTY, no library, no remote runtime.** Cate 1.6.0 moved to the system client too. Files, git, search refuse ssh envs with one message instead of touching the local disk with a remote path. russh/SFTP is a later milestone (KODEN-REMOTE M2+) |
| WP4 worktree Spaces | `git/worktree.rs` (`git_branches`, `git_worktree_list/add/remove`, `git_link_paths`), `parser.rs` additions, `src/modules/worktrees/` (New worktree dialog: name to `feat/<slug>`, any base branch, checkout at `<repo>/.koden/worktrees/<slug>`, `worktreeSymlinkPaths` junctioned in), branch chip + Remove in the Space switcher | **A worktree is a Space.** No `WorktreeMeta` registry: `SpaceMeta.worktree?` carries repo root + branch. Branches are never deleted by remove. Junctions via `mklink /J`, no crate |
| WP2 launcher | `src/modules/launcher/` (`LauncherPane`, `launcherItems` model, key nav, `RemoteConnectForm`), `launcher` tab kind (never serialized), pref `showLauncherOnStart`, `Mod+N`, palette `launcher.show` / `space.openFolder` / `workspace.connectRemote`, "Open folder..." in the switcher, onboarding lands on it | **A tab, not a modal.** Opens on boot over cold restored tabs, whenever a Space has zero tabs, and on demand. Recent Spaces are the recents list (Spaces sort by last use now) |
| WP3 resume + scrollback | `brain/lib/resumeCards.ts`, `useRecoveredPanes`, `RecoveredPanesBanner` (Resume types `claude --resume <id>` only when an id was captured), `brain_resume_plan`, `brain_dismiss_recovered`, `spaces/lib/scrollbackStore.ts` (`koden-scrollback.json`, stable leaf keys, 512 KiB cap, FNV change gate, GC), `rendererPool.snapshotLeafForRestore` / `preloadRestoredBuffer`, `terminal/lib/restoreReplay.ts`, prefs `terminalScrollbackRestoreLines` (2000) and `autoResumeAgents` (false) | The dormant `resume/` journal finally has a UI. Scrollback lives in its own store so the layout file stays small. Replay order on first bind: restored text, `[restored]` separator, same-launch snapshot, DormantRing. Private, blocks, note and task panes never persist |
| WP5 koden CLI | see the WP5 section of `KODEN.md` once merged | **A subcommand of the `koden` executable, never a second `[[bin]]`** (release trap, memory 2026-07-31). Named pipe / unix socket per instance, per-process token planted on every PTY, Settings permission matrix |

Merge wiring added by the orchestrator: `LauncherPane extraSections={recovered.sections}` and resume closes the launcher tab; vitest excludes `**/.claude/**` so agent worktrees inside the repo never count against the tree.

## Consequences

- Cate is retired for the terminal workflow on the workbench as soon as WP1 is
  exercised against a real host (`ssh workbench`); nothing on the host beyond
  sshd, git, rg, fd, tmux (all in `bootstrap.sh`).
- Explorer, editor, git panel and search stay local-only for ssh Spaces until
  the SFTP/exec layer (KODEN-REMOTE M2, M3). The UI says so instead of failing.
- `.koden/` now exists inside repos (worktrees dir with a `*` gitignore). A
  later "project-local Space state" (cate-deep-dive section 7.2) will reuse it
  and needs a trust gate first.
- Gates on `main` after the merge: tsc clean, biome 104 warnings (baseline 109),
  vitest 55 files / 587 tests, eager graph unchanged, clippy 0, cargo lib 516
  passed with only the known `authorize_spawn_cwd_blocks_symlink_escape`.
- Nothing was runtime-verified in the GUI by the agents (a live instance was
  running; dev boots rewrite the global hooks). First manual pass: launcher on
  boot, connect to a host, New worktree Space, kill and relaunch with an agent
  running to see a resume card and restored scrollback, `koden ping` from a tab.

## Ceilings named in the packages

- ssh: fish on the host gets a plain shell; `KODEN_SESSION` does not cross the
  ssh boundary (no AcceptEnv), so the Director bus is remote-side work; OSC in
  tmux depends on the host's passthrough config; the tmux opt-in is plumbed
  (`sshTmux`) but no UI sets it.
- worktrees: no keyboard shortcut; `git_link_paths` local-only; deleting a
  worktree Space via plain "Delete space" keeps the checkout.
- resume: `brain_dismiss_recovered` appends its `exited` marker from the
  command thread rather than the worker (documented single-writer deviation);
  boot auto-resume reads the recovered list once.
- launcher: switching to a local/WSL item whose env differs resets every
  Space's tabs (existing env-switch behaviour).
