# Sandbox harness: test auto-retry and the usage guard without a real limit

Two zero-dependency Node scripts let you exercise Koden's real rate-limit
detection and auto-resume pipeline without burning a real Claude quota or
waiting hours for a real 5-hour limit to hit.

- `fake-claude.mjs` -- impersonates a Claude Code run by writing the exact OSC
  markers and limit-banner bytes that drive the **real** Rust detectors
  (`src-tauri/.../pty/agent_detect.rs` + `retry_detect.rs`). Run it inside a
  Koden terminal so its stdout flows through the real PTY reader.
- `fake-usage-endpoint.mjs` -- a tiny HTTP server that returns the same JSON
  shape as Claude's `/api/oauth/usage`, with a settable utilization, so the
  usage guard can be tested against a known number.

Nothing here is wired into the app build; they are dev-only test drivers.

## Why the banner is phrased the way it is

The retry detector (`retry_detect.rs::parse_reset`) only fires when the
ANSI-stripped output contains one of `usage limit reached`, `limit reached`, or
`out of extra usage`, plus a derivable time (`resets <clock>`,
`resets at <clock>`, or `try again in N <unit>`). The bare phrase
"you've hit your session limit" does **not** contain "limit reached", so it
would arm the agent but never schedule a retry. The default banner therefore
keeps the modern on-screen format (middle-dot `·` separator, lowercase am/pm,
IANA tz in parens) while including "limit reached" so the pipeline actually
fires end to end. Use `--reset "<text>"` to inject any other phrasing, including
ones that intentionally do **not** match (to test the negative path).

---

## 5-step test: auto-retry

1. **Build/run the app** (already running in this session; otherwise
   `pnpm tauri dev` from the repo root). Open a terminal tab.

2. **Enable auto-retry for the tab.** In the AgentDock, flip the per-tab
   auto-retry toggle on (or set the global default `autoRetryEnabled` in
   Settings -> General). Without this, the bridge detects the limit but does not
   resubmit.

3. **Fire a near-future limit** inside that Koden terminal tab:

   ```
   node scripts/fake-claude.mjs
   ```

   This arms the session as `claude` (via `OSC 133;C;claude`), prints the limit
   banner with a reset ~2 minutes out, prints the auto-opened menu, then waits.
   The AgentDock should show the tab go to the "attention/limited" state and a
   pending retry should be scheduled for reset + 60s.

4. **Watch the resume arrive.** When the scheduled time passes, Koden injects
   `Continue where you left off. The previous attempt was rate limited.` into
   the tab. The harness echoes it back as a
   `[harness] received: "...<CR>"  hex: ...` line, so you can confirm the exact
   bytes Koden sent (single-line resume = `text` + CR, no bracketed paste).
   To avoid the 2-minute wait, use a shorter window:

   ```
   node scripts/fake-claude.mjs --minutes 1
   ```

   or inject a relative reset that parses immediately to a short wait:

   ```
   node scripts/fake-claude.mjs --reset "please try again in 1 minute"
   ```

5. **Test the variants and the no-shell path:**

   ```
   node scripts/fake-claude.mjs --variant weekly
   node scripts/fake-claude.mjs --variant opus
   node scripts/fake-claude.mjs --no-shell           # arms via OSC 777 self-arm
   node scripts/fake-claude.mjs --reset "resets at 11:30pm"   # absolute clock
   node scripts/fake-claude.mjs --reset "compiling, all good" # negative: no retry
   ```

   Ctrl-C the harness when done. The negative case should arm the agent but
   schedule no retry (no "received" line ever appears).

Stop the harness with Ctrl-C.

---

## 5-step test: usage guard (no real quota)

1. **Start the fake usage endpoint** (default port 8473):

   ```
   node scripts/fake-usage-endpoint.mjs --pct 0.4
   ```

   It logs (to stderr only) that it's listening and prints the env var to set.

2. **Point Koden at it.** Set the env var in the shell that launches the dev
   app, then start it:

   - PowerShell:
     ```
     $env:KODEN_USAGE_ENDPOINT = "http://127.0.0.1:8473"; pnpm tauri dev
     ```
   - bash/Git Bash:
     ```
     KODEN_USAGE_ENDPOINT=http://127.0.0.1:8473 pnpm tauri dev
     ```

3. **Verify the shape** the guard consumes:

   ```
   curl -s http://127.0.0.1:8473/api/oauth/usage
   ```

   Returns: `{"five_hour":{"utilization":0.4,"resets_at":"<ISO8601>"}}`.

4. **Ramp utilization without restarting** the server, using the query param
   (it wins over the flag per request):

   ```
   curl -s "http://127.0.0.1:8473/?pct=85"     # -> utilization 0.85
   curl -s "http://127.0.0.1:8473/?pct=0.97"   # -> utilization 0.97
   ```

   `--pct`/`?pct=` accept either a 0..1 fraction or a percent (>1 is divided by
   100), clamped to [0,1]. Drive the guard's thresholds by changing the number.

5. **Simulate a fresh vs nearly-exhausted window** with the reset controls:

   ```
   node scripts/fake-usage-endpoint.mjs --pct 0.95 --window 5      # resets in 5 min
   node scripts/fake-usage-endpoint.mjs --pct 0.2 --resets-at "2026-12-31T23:59:59Z"
   ```

Stop the server with Ctrl-C.

---

## Exact run commands (copy/paste)

Inside a Koden terminal tab:

```
node scripts/fake-claude.mjs
node scripts/fake-claude.mjs --minutes 1
node scripts/fake-claude.mjs --no-shell
node scripts/fake-claude.mjs --variant weekly
node scripts/fake-claude.mjs --variant opus
node scripts/fake-claude.mjs --variant sonnet
node scripts/fake-claude.mjs --reset "please try again in 1 minute"
node scripts/fake-claude.mjs --reset "resets at 11:30pm"
node scripts/fake-claude.mjs --reset "compiling, all good"   # negative path
node scripts/fake-claude.mjs --help
```

Usage endpoint (any shell):

```
node scripts/fake-usage-endpoint.mjs --pct 0.4
node scripts/fake-usage-endpoint.mjs --pct 95 --window 5
node scripts/fake-usage-endpoint.mjs --pct 0.2 --resets-at "2026-12-31T23:59:59Z"
node scripts/fake-usage-endpoint.mjs --help
```

Launch the dev app pointed at the endpoint:

```
# PowerShell
$env:KODEN_USAGE_ENDPOINT = "http://127.0.0.1:8473"; pnpm tauri dev
# bash
KODEN_USAGE_ENDPOINT=http://127.0.0.1:8473 pnpm tauri dev
```
