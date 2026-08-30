---
title: Cate deep dive, what sits under the README (parity research for Koden)
created: "2026-08-30"
source: shallow clone of 0-AI-UG/cate @ 1.6.1-beta.4 (Electron, MIT), read in the session scratchpad
status: research, no decision yet; feeds the SSH-env / worktree / resume plan discussed 2026-08-30
---

# Cate deep dive

Cate (0-AI-UG/cate, "an infinite canvas IDE for parallel coding agents") is
Electron + React 18 + node-pty + Monaco + simple-git. This note records what
the README headlines actually mean mechanically, and which of it is worth
carrying into Koden. Read together with `To-Do\Workbench-Setup\KODEN-REMOTE.md`.

## 1. "Local and remote are the same path"

Cate installs a Node daemon (the "runtime") on whichever host owns the
workspace: `~/.cate/runtime/<ver>/<platform>` with an `.ok` marker. Transports:
`localTransport`, `sshTransport` (system OpenSSH client since 1.6.0),
`wslTransport`. The daemon exposes *capabilities* over RPC:

| capability | what it owns |
|---|---|
| `process` | PTYs, shell resolution, per-PTY env, agent process scan |
| `file`, `fileWatcher` | read/write/stat, chokidar watching |
| `vcs` | git via simple-git: status, diff, worktrees, PR checkout, findRepos |
| `agentHooks`, `agentPresence` | hook endpoint + pid registry (see 2) |
| `extensions` | serves extension webviews |
| `tunnel` | bidirectional port forwarding with a credit window (dev server on host -> local browser panel, and reverse) |
| `server` | the RPC server itself |

So "same path" means: every feature that touches the filesystem, git,
processes or agents runs through the same capability interface regardless of
host; the editor, browser and canvas render locally on content pulled over
RPC. Remote workspaces persist in `remote-workspaces.json` as
`{host, user, port, remotePath}`; secrets live in a separate secret store.

Koden equivalent: `WorkspaceEnv` is already threaded through every `fs_*`,
`git_*`, grep/search and PTY command. That IS the capability seam, without a
daemon. The brief's "no runtime on the host" call stands; the one Cate idea
worth keeping is `tunnel` (port forward) for the preview pane, already M4 in
the brief.

## 2. Agent-aware terminals, the real mechanism

Canonical registry `src/shared/agents.ts`: 7 agents (claude-code, codex,
cursor, grok, kiro, opencode, pi). Each row declares `command`,
`matchProcess`, `resumeArgs(sessionId)`, `codingAgentArgs(prompt)`,
`codingAgentFollowUp`, and its skills dir. Adding an agent is one row; the
`Record<AgentId, ...>` tables elsewhere make omissions compile errors.

Detection is **hook-anchored, not process-tree scanning**:

- Every PTY gets `CATE_HOOK_ENDPOINT`, `CATE_HOOK_TOKEN`, `CATE_TERMINAL_ID`.
- Per agent, Cate writes a *project-scoped* hook file whose bridge command
  POSTs the raw hook JSON to the daemon:
  `.claude/settings.local.json` (SessionStart, UserPromptSubmit,
  Notification, PostToolUse, Stop, SessionEnd), `.codex/hooks.json`
  (adds PermissionRequest), `.cursor/hooks.json`, `.pi/extensions/cate-hook.ts`,
  `.grok/hooks/*.json`, `.opencode/plugin/cate-hook.js`, `.kiro/hooks/*.json`.
- Payloads normalise to ONE event stream: `session-start`, `session-end`,
  `turn-start`, `turn-end`, `permission-wait`, `turn-resume`.
- Presence: the hook post's pid is walked up the process table to the nearest
  ancestor whose comm matches the agent; that pid is registered. A 1 Hz scan
  says "still alive with the same comm". The falling edge = finished, and it
  clears the resume stamp. No other presence source, by design.
- Injection mode per agent: `auto` (only if `.claude`/`.codex`/... exists in
  the repo), `on`, `off` (strips Cate's entries).
- Hook contracts are pinned live by an integration test against the installed
  CLIs so a CLI update fails loudly pre-release.

Koden today: global `~/.claude/settings.json` hooks gated on
`KODEN_TERMINAL`, OSC 777 through the PTY, `director-bus.jsonl`, Rust-side
`agent_detect.rs` on OSC 133, plus `agent_codex.rs`. Leaner for one user and
works in tmux/ssh (OSC rides the PTY). Worth taking from Cate: the single
normalised event enum, `permission-wait` as a first-class state, and
session-id capture from the hook payload (see 3).

## 3. Agent sessions survive restarts

- Per-project state: `<root>/.cate/workspace.json` (panels, worktrees,
  layout) + `<root>/.cate/session.json` (live session incl. resume stamps),
  written continuously (not only at shutdown), atomic + corrupt-file
  quarantine, one `.cate/.gitignore` written for you.
- Resume stamp per terminal = `{agentId, sessionId, cwd}`, set from hook
  events only. Claude and Kiro are stamped from the first prompt-submit
  because their SessionStart id precedes a transcript on disk; the others
  from session-start. Cleared when the agent pid disappears.
- On restore the terminal spawns a plain shell and Cate *types* the resume
  command into it: `claude --resume <sid>`, `codex resume <sid>`,
  `opencode --session <sid>`, ...
- Scrollback: xterm SerializeAddon (text + styling + cursor) captured per
  panel id into a scrollback store and replayed verbatim into the fresh xterm
  on next launch; `terminalScrollback` default 2000 lines.

Koden today: tabs + pane tree + cwd restore, PTY fresh, no scrollback across
restarts (SerializeAddon is loaded but only used for renderer-slot stealing).
The Rust `brain/resume/` journal already produces `RecoveredPane` +
`resume_command()` (`claude --resume <id>` only when an id was captured) and
`brain_recovered_panes` exists, but **no frontend consumer** (only
`brain/lib/bindings.ts` references it). Wire that, add the scrollback
snapshot on Space save, and this line item is closed.

## 4. Worktrees, the full model

- `WorktreeMeta { id, path, color, label?, prNumber? }` registry per
  workspace; panels carry `worktreeId` (never a path).
- Checkouts live under `<root>/.cate/worktrees/`.
- Create from: local branch, remote branch, or an open PR
  (`gh pr checkout <n> --branch cate-pr-<n>`, needs `gh auth`; error text
  maps gh failures to friendly messages).
- `worktreeSymlinkPaths` setting: workspace-relative paths (node_modules,
  .venv) symlinked/junctioned into every new worktree so nothing rebuilds.
- Canvas: WebGL "territory" layer groups panels by worktree with the
  worktree colour; sidebar and minimap group by worktree; a terminal's
  submenu lists live worktrees; restored terminals return to their worktree
  cwd; a running agent moves with its worktree.
- Lifecycle: remove (+ `closeWorktreePanelsOnDelete`), "Clean up" prunes,
  `worktreeMerge` (apply rechecks clean + mergeable), compare URL for a PR
  (`github.com/<owner>/<repo>/compare/<branch>?expand=1`).
- Skills and agent hooks are mirrored into *linked* worktrees only, so agent
  config is identical across checkouts.

Koden mapping: a worktree is a Space (root = checkout, colour = accent,
tabs persist). Needs `git_branches`, `git_worktree_list/add/remove`, and a
"New worktree..." action. PR-as-base is a `gh` shell-out, cheap but optional.
The symlink-paths setting is a one-liner worth copying.

## 5. The `cate` CLI, the agent-facing control surface

Seeded as a skill into `.claude/skills/cate-cli` so Claude knows it exists.
Permission matrix in Settings: surface x {Read, Control} for browser,
terminal, panel, editor, notify, agent. A per-terminal CLI session isolates
the selected target panel.

```
cate panel list | set <id> | current | clear | create terminal|canvas | close <id>
cate editor open src/app.tsx:42
cate terminal read | type <text> | press enter          (any panel, incl. foreground TUIs)
cate browser open|navigate|snapshot -i|click @ref|fill|wait --url|screenshot|console|errors
cate agent create "<prompt>" [--agent codex] [--title X] [--new-worktree agent/x | --worktree <id>]
cate agent list | wait <ids> --wait-timeout 10000 | inspect | send <id> "<follow-up>"
cate agent review <id> | apply <id> | keep <id> | discard <id> | stop <id>
```

`cate agent` = recursive orchestration: each worker is a real agent CLI in its
own terminal panel (optionally its own worktree), owned by the terminal that
created it; workers may create workers (a tree). Status derives from the hook
state machine: starting / working / waiting / ready / stopped / failed.
`MAX_CONCURRENT_CODING_AGENTS = 5`. `wait` is a 5-60 s long-poll; `background`
workers wake the supervisor on state change. `review` is read-only,
`apply` merges the worktree if clean and mergeable, `keep` retains the
branch, `discard` deletes worktree + branch without confirmation.
"Orchestrator mode" for the in-app agent is only a system-prompt toggle that
points it at this CLI.

Koden today: the Director uses Claude Code's *native* subagents inside one
session (`--agents` roster, Task hook -> bus). The Librarian has
`terminal_send/read/list` and `workspace_open_tab/split/focus`, but those are
in-app tools; nothing a `claude` running in a Koden terminal can call. This
is the largest untapped Cate idea for Koden: a thin `koden` CLI (or MCP) over
a local socket exposing terminal read/type, tab/pane open, and
`agent create --worktree` (= new Space + terminal running `claude "<prompt>"`).
Cross-provider workers (Codex reviewing Claude) fall out of it.

## 6. Smaller mechanisms worth knowing

- **Multi-repo source control**: `findRepos(dir, maxDepth)` walks down,
  skipping node_modules/dist/target/.venv/dot-dirs, stops at each repo; each
  nested repo gets its own SCM section. Koden's panel is one repo from the
  active cwd; the Snorlax tree is dozens of nested repos.
- **Processes and ports**: daemon reads `/proc` for listening ports per PTY
  pid; the sidebar shows ports per terminal, click opens a browser panel.
  Koden detects localhost URLs in output for the preview pill; ports-per-tab
  is a better signal (Windows: `GetExtendedTcpTable`).
- **Auto-suspend idle terminals** (daemon-level). Koden's `rendererPool`
  (max 5 live xterm) already covers the renderer side.
- **Terminal**: OSC 52 clipboard, `path:line` links open the editor, URL
  auto-open routes to a browser panel, file drops paste quoted paths.
  Koden has smart links; drag-and-drop is on its roadmap.
- **Hand-editable JSON state** under userData with an external-edit watcher
  and corrupt-file quarantine; `recent-projects.json`, `sidebar.json`,
  `remote-workspaces.json`, `layouts.json`.
- **Project-local layout** (`.cate/workspace.json`) means the layout travels
  with the repo. For a MEGA-synced tree this would make Space layouts follow
  HQ -> laptop -> workbench for free if Koden stored Space state in
  `<root>/.koden/` instead of `koden-spaces.json`.
- **Workspace trust dialog**: repo-controlled layouts and hooks need explicit
  trust before Cate acts on them. Relevant the moment Koden reads `.koden/`
  from a repo.
- **Notifications**: OS-level only, `notifyOnlyWhenUnfocused`. Koden's
  three-way router (suppress / OS / toast) is already richer.
- **Saved layouts**, snap-to-grid, placement picker, freehand drawings,
  nested canvases, detached windows: canvas concerns, no Koden equivalent by
  design.
- **Skills manager**: install SKILL.md bundles into each agent's dir
  (`.claude/skills`, `.codex/skills`, `.cursor/skills`, `.agents/skills`...),
  GitHub crawl of sources, per-workspace `.cate/skills.json`. This is
  Conductr's job in Kosta's setup; skip in Koden.
- **Extensions**: isolated webview panels with a `cate.*` host API (theme,
  workspace {rootPath, branch, worktree}, storage, panel list/focus/close,
  agent.send, files.onDrop), manifest scopes + consent. Off-ethos for a 7 MB
  binary; the host-API *shape* is a good reference if Koden ever exposes one.
- **Browser panel** on the `agent-browser` engine (CDP + accessibility
  refs `@s1e4`, visible agent cursor, user input pre-empts), credential
  profiles. Out of Koden scope ("not a browser").
- **Cate Cloud** (`plan.md`): EC2 per workspace, runner with outbound mTLS,
  their paid roadmap. Irrelevant.

## 7. What this changes in the Koden plan

Keep the 2026-08-30 order (SSH env lite -> resume wiring + scrollback ->
worktree Spaces -> open-folder Space -> russh layer -> multi-repo). Add:

1. `koden` CLI for terminal agents (terminal read/type, tab open,
   `agent create --worktree`). Biggest untapped idea; Koden already has the
   bus and hooks, the CLI is a thin client.
2. Project-local Space state in `<root>/.koden/` with a trust gate, so
   layouts sync through MEGA across machines.
3. `worktreeSymlinkPaths`-style setting when worktree Spaces land.
4. Ports-per-tab from the OS instead of URL sniffing (later).
5. Single normalised agent-event enum incl. `permission-wait`; stamp the
   Claude session id from the first UserPromptSubmit, not SessionStart.

Explicitly not chasing: canvas, browser panel, extension webviews, skills
manager, Kiro/Grok/Cursor/Pi rows, cloud workspaces.
