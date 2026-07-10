# Day report — hook pipeline fix + validation (2026-07-11, afternoon)

**TLDR: your "inputs don't work 100%" bug is root-caused, fixed, and proven
end-to-end against the live app — 4/4 turns now appear in the Inputs popover
(screenshot: `svart-verification/svart-e2e.png`). Sidebar says WORKSPACE now.**

## The bug (two independent causes, both fixed in `49aface`)

1. **The reliable channel was severed:** `AgentBusBridge` tailed
   `agent-bus.jsonl` (0 bytes, only ever truncated) while every hook writes
   `director-bus.jsonl`. The turn channel built to catch every prompt
   contributed nothing, ever.
2. **Why you saw exactly 2 of 4:** Claude Code 2.1.206 (vs the 2.1.168-era
   assumption baked into comments) now *partially* honors `terminalSequence`
   on UserPromptSubmit — its emitter is UI-lifecycle-gated inside the CLI and
   silently drops. It emitted OSC-777 for your first two submits (minting
   marker turns "hi", "5+5"); the mere presence of marker turns then
   **suppressed the scrollback-scan fallback** that would have found "hiii"
   and "30 countries…". Two lucky markers hid the two failures.

Also fixed while in there (latent, would have bitten later): turn history
died on renderer-pool pane rebinds (now in a session-lifetime `turnStore`),
the scrollback-scan cache froze once the buffer hit cap, subagent lifecycle
lines carried no session identity (hooks now stamp `parent`), and the
Director dispatch is now scoped so the two bridges can't double-materialize.

## Proof (not vibes)

- Verifier rebuilt the **pre-fix code in a throwaway worktree and reproduced
  your exact screenshot** (`['hi','5+5']`), then showed the new tests fail on
  old code and pass on new.
- **Live e2e** against the running app: typed a loop into the real PTY (so the
  pane's own `KODEN_SESSION` stamps the lines, same file+shape as the hooks),
  opened the Inputs popover — **4/4 synthetic turns render**, shell command
  marks intact alongside. `svart-verification/svart-e2e.png`.
- New-shape hooks confirmed installed in `~/.claude/settings.json` after boot.
- Gates: tsc clean · vitest 395 passed (25 new tests) · `cargo test --lib
  agent` 28/28.

## Things you should know

- **Two dev instances were running simultaneously** (this worktree's app AND
  the main-tree app — your brain session's GUI validation). Both rewrite the
  global Claude hooks at boot; **last boot wins**. Old-shape hooks remain
  compatible for turn capture (the reader accepts both — the e2e passed under
  old hooks), but subagent `parent` stamping needs the new installer to have
  run last. There's no single-instance guard in dev; the two instances also
  share appdata/stores. Worth a guard or at least awareness. I killed/relaunched
  only the worktree instance; the main-tree one was left alone (it self-healed
  after my earlier too-broad WebView sweep — apologies to the other session).
- Claude sessions started before a hook reinstall keep their old hook commands
  until relaunched (CC reads settings at launch).
- Deliberately NOT fixed: `agent-status` bus lines have no writer (status still
  rides OSC-777, hostage to CC's gated emission — flagged in the commit);
  SubagentStop carries no per-subagent identity so retire stays FIFO.
- Your worktree app is left **running** (with new hooks installed last).

## Today's full branch delta (after the overnight Svart revamp)

`345c4fa` Brain tab control room + de-Terax AI copy · `5a960e0` **the chat is
the Librarian** (brain-grounded persona + brain_search/brain_notes tools,
Librarian/Brain tab split, ADR-017 at `5fd382d`) · `b1ac270` sidebar
Tabs→Workspace · `49aface` hook pipeline fix · plus this report + e2e evidence.
