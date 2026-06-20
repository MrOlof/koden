# Koden autonomous test harness (dev-only)

A harness so an AI agent (or CI) can drive the **whole** Koden app and verify
flows over time, against a **disposable** profile. Design + full coverage matrix:
[`.memory/test-harness-design-2026-06-20.md`](../../.memory/test-harness-design-2026-06-20.md).

> **DEV-ONLY.** The control bus and (future) embedded WebDriver port must never
> ship. The bus is mounted only behind `import.meta.env.DEV` (see
> `src/dev/testBus.ts` + the effect in `src/app/App.tsx`); Vite strips it from
> release builds. Verify on every release: `grep -r __KODEN_TEST__ dist/` returns
> nothing.

## The control bus — `window.__KODEN_TEST__`

The primary surface is an in-app bus, **not** DOM scraping (the terminal is a
WebGL canvas; settings is a separate webview; much state is store-only). API:

| Method | Purpose |
|---|---|
| `ready()` | bus installed? |
| `openPalette(open?)` / `commandCount()` / `commandIds()` | palette items only exist while open |
| `runCommandById(id)` | execute any registered command by id (kills MRU/fuzzy DOM flakiness) |
| `runShortcut(id, index?)` | fire keybinding-only actions (`tab.selectByIndex`, `pane.focusNext`, …) |
| `tabsSnapshot()` | tabs + active kind/title + `paneCount` + `leafIds` (the non-store state) |
| `newGridTab(rows, cols, cwd?)` | grids have **no** command — bus is the only deterministic path |
| `reorderTab` / `duplicateTab` / `moveTabToSpace` | tab mutators without a command id |
| `submitToLeaf(leafId, cmd)` | type+run in a terminal |
| `serialize(leafId)` / `getBuffer(leafId)` | read scrollback (ANSI / plain) — the canvas read seam |
| `searchResultCount` / `commandMarkCount` | terminal search + OSC-133 marks |
| `getStores()` | read-only snapshots of every assertion-critical Zustand store |
| `settings.setThemeId/setTheme/setAutostart/…` | the **separate-settings-webview bypass** |

The three palette mode-switch items (`theme.pick`, `search.content`,
`history.open`) are `run: noop`; the bus throws a redirect message for them
instead of silently doing nothing.

## Run it

```sh
pnpm install                       # first time — installs the WDIO devDeps
node scripts/launch-sandbox.mjs    # boots `pnpm tauri dev` on a disposable profile
pnpm test:e2e                      # runs tests/e2e/**/*.e2e.ts through the bus
node scripts/launch-sandbox.mjs --teardown   # wipe the scratch profile
```

MVP specs (the walking skeleton): `terminal.e2e.ts` (type+read a command),
`grid.e2e.ts` (2×2 grid → 4 panes), `settings.e2e.ts` (set a pref via the
bypass). Every read is polled (`waitUntil`) — **never fixed sleeps**.

## ⚠️ Not runnable until the Phase-0 spike

`wdio.conf.ts` cannot create a session yet. The design picks the **embedded
`tauri-plugin-webdriver`** (W3C endpoints served inside the app behind a
`webdriver` Cargo feature) over the pre-alpha official `tauri-driver`, because it
sidesteps msedgedriver/WebView2 version-drift hangs and exposes real
`GET /window/handles` + `POST /window` for the settings multi-window path.

**Phase-0 spike (≈½ day) — do first:**
1. Add `tauri-plugin-webdriver` behind a dev-only `webdriver` Cargo feature in
   `src-tauri`; expose its host/port; set `KODEN_WD_HOST`/`KODEN_WD_PORT` and the
   real `capabilities` in `wdio.conf.ts` (replace the `browserName: "wry"` stub).
2. Prove the multi-window switch: open settings, assert
   `getWindowHandles().length === 2`, `switchToWindow`, read a settings-only node.
   If it fails, keep the store-setter bypass (state + persistence asserted; the
   settings widget's own `onChange` wiring is **not** — a broken control passes
   green) and run the UI-wiring suite as a low-frequency manual pass.
3. Confirm `data-testid` survives the prod minifier (it should; verify in `dist`).

## Isolation status (honest)

- ✅ WebView2 user-data (localStorage `koden-*`, IndexedDB `koden-bg-images`) —
  `WEBVIEW2_USER_DATA_FOLDER`.
- ✅ Provider API keys — `VITE_KEYRING_SERVICE=koden-sandbox`
  (`src/modules/ai/config.ts`); regenerate/teardown via the OS keychain.
- ◐ HOME/USERPROFILE redirected (shell cwd + OS-home fallbacks).
- ✗ Tauri plugin-store files (`koden-*.json`) are **not** redirected by the
  APPDATA env on Windows (the `dirs` crate ignores it). Full isolation needs a
  sandbox **bundle identifier** — see the TODO at the bottom of
  `scripts/launch-sandbox.mjs`.

## Coverage expansion

Each surface in the design's coverage matrix → one `*.e2e.ts` file. Wire
`scripts/fake-claude.mjs` / `fake-usage-endpoint.mjs` for AI/status/retry/usage
determinism. Out of scope until built: **search-across-conversations** (no
`chatStore.searchSessions`/`searchMessages` exists — confirm before relying on
it). Pure pointer-DnD (pane redock, gutter resize, graph pan/zoom) is covered by
asserting the *result* via bus mutators + screenshotting the gesture; the
`data-testid`s added to those controls are the selector anchors.
