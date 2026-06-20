# ADR-003: Proactive usage guard, reactive auto-retry fix, command minimap, and the ADR-002 iterations

Status: Accepted (implemented + statically verified — live GUI/real-run verification pending) — 2026-06-19 (overnight)

Branch `overnight/agents-tasks-persistence-2026-06-16` (main untouched, uncommitted). All static gates green: `pnpm check-types`, `pnpm lint` (only pre-existing warnings), `pnpm test` (318 pass; the 1 failing file is the known pre-existing `eager-budget.test.ts` env issue), `cargo build --locked` + `cargo test` (retry_detect 22, usage 16; 192 lib tests, 1 pre-existing Windows symlink-privilege test). No new npm/cargo deps (libc was widened from `cfg(unix)` to also `cfg(windows)` — same crate, already in Cargo.lock).

## Context

Follow-on to ADR-002. Kosta iterated on the five additions, then asked for two genuinely new capabilities (a usage guard like `shirley-xue-2025/usage-guard` but for Windows/Terax, and a ChatGPT-style command minimap), and explicitly wanted the rate-limit handling "battle tested." Built overnight with full autonomy via multi-agent workflows, each phase verified before the next.

## Decisions

### Iterations on ADR-002

- **Pane dropdown UX:** bigger always-faintly-visible `⋯` trigger; the "Title color" item became a submenu with **Custom…** (per-pane native picker) + an **Auto-color** radio (Off/Muted/Vibrant/Pastel) that flips the global `paneColorMode`/`paneColorPalette` inline. Switching palette now **recolors existing panes** too (App.tsx effect, gated on `prefsHydrated` to avoid clobbering restored colors on boot — a confirmed boot-race fix).
- **Readable auto-colors (the purple bug):** generation moved from HSL to **OKLCH** (`paneAutoColor.ts`) — perceptually-uniform L means every hue (incl. blue/purple) clears WCAG ≥4.5:1 on the dark `--card` (`#161b1d`). Dependency-free `oklchToHex` with binary-search chroma gamut-mapping; bands muted/vibrant/pastel differ by chroma + L. Title typography bumped to 12px / weight 500 / +0.012em tracking. The test enforces the contrast floor across all hues.
- **Smart links → configurable categories** (`linkDetect.ts`): 8 categories `path/filename/ip/email/guid/secret/sid/winuser`, each with a per-type **Off/Copy/Open** setting (`linkTypes` pref, `DEFAULT_LINK_TYPES`). Fixed the fixture misses/false-positives: IPv4 (octet-validated), email/UPN, SID, `DOMAIN\user`, consistent bare filenames (curated ext allowlist), Windows paths with **interior spaces**, `$env:VAR\…`; and killed false-positives (`/upn` flag no longer a path, `.msi` no longer a "secret", bare `key`/`token` labels dropped). Provider maps action→reveal/copy/openUrl. Background-highlight hover (cyan=copy, blue=open).
- **Terminal scrollbar** made visible + themed (`globals.css`) overriding the global scrollbar-kill.
- **Graph (topology):** **double-click a node to focus+lock** (view centers and follows it through reflows; ring + "Locked: <name>" pill with unlock). Auto-fit now runs once then **stops once the user pans/zooms/focuses** (the "snaps around" fix), and programmatic moves animate instead of snapping.

### Reactive auto-retry — FIXED (it was dead against the installed CLI)

Research (machine-verified) found the v2.1.168 banner is `You've hit your <session|weekly|Opus|Sonnet> limit · resets <time> (<IANA tz>)` (middle-dot U+00B7); the legacy `5-hour limit reached` string is **gone from the binary**, so the old `retry_detect.rs` predicate never fired. Fixes: broaden `has_limit` (`you've hit your`, `you're out of extra usage`, `now using extra usage`; legacy kept as fallback), add `resets` to the early-out, harden `parse_clock` against the trailing `(tz)` paren and date-day numbers (`Jun 21, 3pm`→15:00), and implement a **real Windows TZ offset via libc** (was UTC-only → hours off). `RetryBridge` now sends **Esc before the resume** to dismiss the auto-opened `/rate-limit-options` menu.

### Proactive usage guard (new)

- **Feasibility verified on this machine:** `~/.claude/.credentials.json` is plaintext JSON (no DPAPI) → `claudeAiOauth.{accessToken, refreshToken, expiresAt}`. Endpoint `GET https://api.anthropic.com/api/oauth/usage` with `Authorization: Bearer`, `anthropic-beta: oauth-2025-04-20`, `User-Agent: claude-code/<version>` (mandatory or persistent 429) → `five_hour.{utilization, resets_at}`. Token refresh `POST https://console.anthropic.com/v1/oauth/token` (client_id `9d1c250a-e61b-44d9-88ed-5944d1962f5e`), atomic temp+rename writeback, **never logged**.
- **Rust** (`src-tauri/src/modules/usage/`): async `reqwest` poller driven from a `std::thread` via `tauri::async_runtime::block_on` (no tokio `time` flip, no chrono). Adaptive cadence (30m→…→60s by %), single in-flight, `TERAX_USAGE_ENDPOINT` override for the sandbox, 3-null→`telemetryLost` (fail-open, keep last good), **time-based fallback** window (stamped on first claude activity, persisted to `usage-window.json`) so the feature works even if the endpoint breaks. Emits `terax:usage-signal {percentUsed, resetEpochMs, thresholdCrossed, source, telemetryLost}`. Commands `usage_guard_set(enabled, warn_pct, pause_pct)` + `usage_guard_snapshot()`. **Default enabled=false.**
- **Frontend:** `UsageBridge` warns once per window, sets `usageStore.pauseActive` at the pause threshold (hysteresis), optional opt-in hard-stop (Ctrl-C, default off). The **soft-gate consumer** is wired: `App.handleSpawnTerminalAgent` refuses new agent spawns while `pauseActive`. `UsageBridge` calls `usage_guard_set` on mount + on pref change so the Rust poller actually honours the user's enabled state/thresholds (without this the poller stays at its safe disabled default — this was the last gap closed).
- Prefs: `usageGuardEnabled`(false) / `usageGuardWarnPct`(85) / `usageGuardPausePct`(90) / `usageGuardHardStop`(false), in Settings → Agents.

### Fake-claude sandbox harness (so a real limit isn't needed)

`scripts/fake-claude.mjs` emits the OSC-133 arming sequence (`\x1b]133;C;claude\x1b\\`) + the exact modern banner (near-future reset) + the `/rate-limit-options` menu, then echoes what Terax injects — driving the REAL detector pipeline inside a Terax terminal. `scripts/fake-usage-endpoint.mjs` serves a fake `/api/oauth/usage` (point Terax at it via `TERAX_USAGE_ENDPOINT`) to exercise warn/pause. The harness banner strings are verified byte-equal to the `retry_detect` test cases. See `scripts/README-sandbox.md`.

### Command minimap (new)

`commandMarks.ts` is a trimmed `BlockDecorations`: its own OSC-133 handler captures one mark per command (text from the `133;C` payload — zsh/fish/PowerShell; bash falls back to the echoed buffer line), `registerMarker(0)` tracks the line, `D` settles ok/fail. `CommandMinimap.tsx` renders a right-edge tick strip (hover preview, click → `term.scrollToLine`), hidden in alt-screen or with <2 marks, inset to clear the scrollbar. Gated by `commandMinimapEnabled` (default off); skipped in blocks mode (which already has block chrome).

## Consequences

- Needs live GUI / real-run to confirm (inherently un-headless-testable): end-to-end retry firing in a real PTY, the real OAuth poll + token refresh, Esc-dismiss timing, hard-stop, the minimap rendering/scroll/preview, and cross-window settings reactivity. The fake-claude harness is the tool for the first of these.
- Deferred (low): ADR-002-review finding #4 (legacy custom themes inheriting a `selectionForeground` they never set).
- Everything remains uncommitted on the overnight branch for Kosta's review.

### Post-build adversarial review (same night)

A read-only review of the newest code (usage/retry/minimap) found + fixed 4 confirmed issues, all verified (check-types + 192 Rust tests green): (1) **retry_detect false-positive** — `parse_reset` matched the limit phrase and the time clause anywhere in the 4096-char window, so Claude's own prose about rate limits (or building this feature) could fire a spurious retry; now anchored to require the clause within 80 chars of the limit phrase, with a UTF-8 char-boundary guard so slicing across the middle-dot can't panic the PTY thread. (2) **token-refresh CAS** — `refresh()` now re-checks the on-disk `refresh_token` before committing and adopts the on-disk creds if the Claude Code CLI rotated them concurrently (was last-writer-wins). (3) **UsageBridge last-agent reset off-by-one** — counts agents *other than* the exiting leaf (via `leafIdForPty`) instead of `length <= 1`, so a second running agent no longer gets its guard state wiped. (4) **minimap bare-`D` tick** — an absent exit code is now "ok" not "fail", matching BlockDecorations.
</content>
