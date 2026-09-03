# ADR-026: Remote pane parity, tmux passthrough and generator-owned pane-events

Status: Accepted, 2026-09-03 (Kosta: "the entire functionality of tabs and
agents blinking when they are working, idle etc doesn't work when we use the
remote feature"). Shipped as v0.12.3; v0.12.4 the same afternoon fixes the
fan-out it introduced (addendum below).
Builds on: M2.8 (pane-events tail), ADR-019 (hook ownership classes).

## Context

Every activity signal a LOCAL terminal tab has comes in over the wire as an
escape sequence: OSC 133 command markers from the shell integration (busy,
idle, agent armed, agent exited), OSC 7 for cwd, OSC 777 from the Claude
Code hooks (working, attention, finished). A Koden ssh Space runs every
terminal inside a tmux window on the host, and tmux consumes unknown OSC
unless `allow-passthrough` is on. Koden never set it, so for a remote pane
none of those sequences ever reached the client. The only remote signal was
M2.8's pane-events tail, which feeds exactly one consumer, the small tab
status pill, and nothing else (no agents panel node, bell, toast, taskbar
flash, renderer busy hold).

Worse, the pane-events hooks themselves were not safe. They had been added
by hand on the host INSIDE the Koden-owned Notification and Stop hook groups
of the shared `~/.claude/settings.json`. Koden regenerates its hook groups on
every launch (`agent_enable_claude_hooks`) and replaces every group carrying
an owned marker, so each start of Koden on HQ deleted the pane-events hooks,
and the knowledge-sync then published the deletion to every machine.
Verified 2026-09-03 13:06: the file changed at Koden's restart, four
pane-events lines gone, nine generated lines added.

## Decision

1. **Koden generates the pane-events hooks itself**, one group per event
   (UserPromptSubmit, Notification, Stop, SessionStart), in their own
   ownership class (`pane-events.jsonl` marker, not in OWNED_MARKERS). They
   get their own stdin (the status hook already consumes stdin for the bus)
   and survive every re-install; legacy hand-added copies nested in owned
   groups are stripped with those groups and come back as their own.
   `agent_claude_hooks_status` now also requires the pane-events marker and
   the tmux wrapper, so pre-fix installs self-heal on the next launch.
2. **The hooks' OSC 777 rides a DCS passthrough inside tmux.** The hook
   shell picks the form at run time on `$TMUX`: raw outside, `ESC P tmux;`
   + the sequence with every ESC doubled + `ESC \` inside. Every byte is a
   JSON escape (`\u001b` for ESC, `\u005c` for the trailing backslash) so the hook
   text carries no literal backslash; a Rust test runs the generated text
   through `sh` with and without TMUX and parses what it prints.
3. **The shell integration bundles wrap the same way.** bash, zsh and fish
   gain a `_koden_osc` helper (DCS-wrap when `$TMUX` is set) for the printed
   OSC 133 A/C/D and OSC 7, and the PS1/PS0 markers are spelled in the tmux
   form in prompt-escape syntax. The bundle republishes itself to the host
   on the next spawn (content hash), no manual host edit. Verified on
   ai-server: bash wrapper and prompt markers emit exactly the right bytes.
4. **Koden turns passthrough on** when it creates the tmux session:
   `allow-passthrough on`, NOT `all`. See the 0.12.4 addendum: `all`
   broadcasts to every client of the session, and every Koden tab is a
   client. With `on` tmux delivers to the clients where the pane is
   visible, which in Koden's one-viewport-per-window model is exactly the
   pane's own tab (plus a second device viewing the same tab).
5. **Remote notification rule matches local.** pane-events "notification"
   escalates to orange whenever it arrives, as the local OSC 777 path does;
   the old mid-turn gate was why remote almost never showed amber while
   local always did (66 of 70 host notifications were idle pings).

With 2 to 4 the whole local pipeline runs unchanged for remote panes,
because every consumer listens on `koden:agent-signal`, which the Rust
detector now emits for remote panes too. pane-events stays as the second,
idempotent source and as the dashboard's feed.

## Consequences

- Remote tabs blink and pill exactly like local ones: busy on a command,
  armed when `claude` starts, working, attention, finished, cleared on exit,
  cwd tracked. Agents panel, bell, toasts and taskbar flash follow for free.
- Still missing for remote panes, deliberately out of scope here: the
  director bus. `KODEN_SESSION` does not cross ssh, so subagent nodes, turn
  capture and Brain session activity stay blind for remote panes until
  Koden either forwards a pane identity into the ssh environment or tails
  the host's `director-bus.jsonl` the way it tails pane-events. Also
  `pty_has_foreground_job` stays false for an ssh tab on Windows (it counts
  children of `ssh.exe`); OSC 133 makes it redundant for busy detection.
- zsh and fish wrappers are syntax-plausible but unverified on a real host
  (ai-server has neither shell). First zsh or fish user on a remote host
  should watch the prompt once.
- HQ's working-tree settings.json was restored from git; 0.12.2 will wipe
  it once more on its next launch, and 0.12.3 rewrites it correctly.
- Tests: 33 in the agent module (was 29), incl. the regression for the
  nested-legacy case and the sh round trip; Rust suite 563 pass, 1
  pre-existing Windows symlink-privilege failure unrelated; clippy clean;
  frontend 77 in spaces + tabs, tsc clean.

## Addendum, v0.12.4: the passthrough fan-out

Within the hour of 0.12.3 Kosta was "spammed with notifications on tabs
nobody is touching". The Windows notification database (wpndatabase.db)
showed the toasts arriving in bursts inside one second, at exactly the
moments the host logged ONE Claude notification in ONE pane: 4 toasts at
15:17, 6 at 15:42, 9 at 15:47, naming Exchange, Intune, Laptop, Finance,
"new". Tapping an idle pane's raw output proved it emits nothing at rest
(a forced resize produced a redraw with title updates and no notification
sequences), so the repeats were not the agent talking.

Cause: `allow-passthrough all`. tmux forwards a DCS passthrough to every
client of the session when the option is `all`, and every Koden tab holds
its own client (a grouped-session viewport). One OSC 777 from one pane
therefore reached every tab's terminal, each tab's detector self-armed on
the Koden marker and raised its own attention toast. The scout's caveat
that `on` only delivers to clients viewing the pane is exactly the property
we want here: each viewport views its own window.

Fixes:
- `shell_ssh.rs`: `allow-passthrough on`. Set on the live ai-server tmux
  immediately (stops the fan-out for existing tabs; a 0.12.3 client opening
  a new tab flips it back to `all` until it updates).
- `agent_detect.rs`: attention is a STATE, not a pulse. An agent already in
  Waiting that announces attention again (Claude Code's own ghostty/kitty
  notification arriving beside our hook marker, or any repeat) emits no
  second signal; `working` resets it. Test added; the legacy "OSC 9 after
  OSC 777" expectation updated to the new semantics.
