---
title: Koden — autonomous agent-driven test harness design (complete action coverage)
created: "2026-06-20"
updated: "2026-06-20"
status: research only — READ-ONLY pass, no source code changed; design/plan for later build
method: 9-agent read-only workflow — 4 action/surface coverage mappers + 3 evaluators (tauri-driver feasibility / testability additions / approach comparison) -> completeness critic -> editor
goal: a harness so AI agents can autonomously + repeatably test the WHOLE Koden app over time; hard requirement = COMPLETE action coverage; run against a sandboxed profile
related: feature-research-2026-06-19.md, fork-rebrand-and-onboarding-2026-06-19.md
---
## TL;DR

- **Build a hybrid harness, weighted toward a dev-only in-app test bus (`window.__KODEN_TEST__`), with WebdriverIO + the *embedded* `tauri-plugin-webdriver` reserved only for the irreducible pointer/visual slice.** Pure WebDriver-over-DOM cannot reach complete coverage; the bus + sandbox closes the three hardest holes (canvas terminals, the separate settings webview, store-only assertions) at once.
- **The bus is cheap and reuses what already exists.** The full command array (`commandPaletteItems`, `App.tsx:1820`) and shortcut map (`shortcutHandlers`, `App.tsx:1183`) are already assembled every render; every assertion-critical store is a Zustand singleton; the `SerializeAddon` is already loaded per terminal (`rendererPool.ts:206`) and gives exact scrollback text. The dev-gate convention (`__teraxTerm`, `__teraxNewBlockTab`) is already in the codebase — verified.
- **Terminals are NOT the feared canvas-OCR problem.** `WebglAddon` (`rendererPool.ts:802`) confirms the buffer is canvas/DOM-unreadable, but `serialize(leafId)` recovers exact text deterministically. Drive via the hidden `term.textarea` + `submitToLeaf`, assert via `getBuffer`.
- **The separate `settings` webview is reachable WITHOUT a second WebDriver window** by importing the existing `src/modules/settings/store.ts` setters into the bus — they persist to `terax-settings.json` and emit `terax://prefs-changed`, which the main window already consumes. A WebDriver multi-window switch is the *optional* path for asserting the settings UI's own wiring.
- **Reuse `scripts/fake-claude.mjs` + `scripts/fake-usage-endpoint.mjs` (ADR-003)** for all agent/status/retry/usage determinism, and launch against a scratch HOME/APPDATA + sandboxed workspace + namespaced keyring.

**Coverage ceiling:** ~95% fully deterministic and drivable; the residual ~5% is genuinely undrivable (native OS dialogs, pure-gesture DnD, OS side-effects) or a *missing feature* (search-across-conversations does not exist — it cannot be covered until built).

**Can it drive everything?** Yes for every action that has a command, shortcut, store action, or terminal handle — which is everything except native dialogs, a handful of pointer-gesture-only visuals, and one unbuilt feature; those are bypassed at the app seam, screenshot-asserted, or honestly declared out of scope.

## Recommended architecture

**Drive layer.** WebdriverIO with `@wdio/tauri-service`, but the **primary control surface is the in-app bus, not DOM**. WebDriver's only essential job is (a) bootstrapping the session and typing into the xterm helper textarea, and (b) the pointer/visual residual. Use the **embedded `tauri-plugin-webdriver` (Choochmeque)** provider, *not* the official pre-alpha `tauri-driver` + msedgedriver: the embedded server runs W3C endpoints inside the app, sidesteps msedgedriver/WebView2 version-drift hangs, and exposes real `GET /window/handles` + `POST /window` for the multi-window settings path. **Gate it behind a `webdriver` Cargo feature, dev-only — it must never ship in a release build.**

**Control surface — `window.__KODEN_TEST__`.** One dev-only module (`src/dev/testBus.ts`, ~150–250 lines) mounted from a `import.meta.env.DEV`-gated `useEffect` in `App.tsx`, mirroring the existing `__teraxNewBlockTab` registration (`useTabs.ts:421`, verified). It exposes:
- `runCommandById(id, ...args)` — iterates `createCommandItems(ctx)` (`src/modules/command-palette/commands.ts`) and calls `item.run` directly. This is the **true "execute by id" bus the app lacks today**; it kills the fuzzy/MRU DOM-selection flakiness (`mru.ts` → localStorage `terax-palette-mru`). Map the three noop mode-switch items (`theme.pick`, `search.content`, `history.open`) to their real effects (`setThemeId`, set-query) so callers aren't misled.
- `runShortcut(id, index?)` — over `shortcutHandlers`, covering keybinding-only actions (`tab.selectByIndex` Mod+1–9, `tab.next/prev`, `pane.focusNext/Prev`, `terminal.clear`, zoom/zen/block-nav) without platform-sensitive synthetic keys (`MOD_PROP` meta-vs-ctrl, the 1–9 special case).
- `getStores()` — read-only `getState()` for `orchestrationStore`, `docsStore`, `chatStore`, `usePreferencesStore`, `paneTitles`, `planStore`, `todoStore`, `agentsStore`, `snippetsStore`, `retryStore`, `useTabStatusStore`, `useLayoutMode`.
- `tabsSnapshot()` — the **one piece of state not in a Zustand store**: tabs + `paneTree` + `activeLeafId` live in React-local state inside `useTabs` (`useTabs.ts:232`). Register a ref-backed snapshot from inside the hook (effect re-running on `tabs`/`activeId`) so it never returns a stale closure.
- Terminal handles `serialize(leafId)` / `getBuffer` / `submitToLeaf` / `whenSessionReady` / `getCommandMarksForLeaf` / `getSearchAddonForLeaf` — already re-exported from `src/modules/terminal/index.ts`, just not on `window`.
- `newGridTab(rows,cols,cwd)`, `movePane`, `reorderTab`, `moveTabToSpace`, `duplicateTab` — **grids and within-space reorder have no command/keybinding at all**; this is the only deterministic path.
- The `src/modules/settings/store.ts` setters + `agentsStore`/`snippetsStore` CRUD + `emitKeysChanged` — the settings-webview bypass.

**Terminals (xterm).** Drive: focus `term.textarea` (referenced `useTerminalSession.ts:638`, `PaneTreeView.tsx:256`) + sendKeys, or `submitToLeaf(leafId, cmd)`. Assert: `serialize(leafId)` (reuses the already-instantiated `SerializeAddon`, `rendererPool.ts:206` — verified). Gate every read on `whenSessionReady` then poll for a sentinel; never a fixed sleep. The per-pane "Split into" menu uses a **native capture-phase `contextmenu` listener** on `[data-pane-leaf]` (xterm canvas is outside React's fiber) — a synthetic React `onContextMenu` will not open it; dispatch a native `MouseEvent('contextmenu')` or prefer the React header ⋮ button (`aria-label "Pane options"`).

**Separate settings webview.** Default to the **store-setter bypass** (no second WebDriver window): call the imported setters, assert via `usePreferencesStore` + persisted `terax-settings.json`. Use the embedded driver's window-switch *only* for the low-frequency suite that proves the settings UI's own `onChange` wiring (a broken Radix Switch passes green under the bypass). The multi-window switch is an **unverified 1-hour spike** — run it before designing around it.

**Native dialogs.** Permanently undrivable; bypass at the seam: pre-seed the sandboxed workspace root so the folder picker is never invoked; set `<input type=color>.value` + dispatch synthetic `input`/`change`; theme-import + bg-image `<input type=file>` accept `sendKeys(path)`; stub `useWhisperRecording`/inject transcript for voice.

**Sandbox profile.** A launcher (`scripts/launch-sandbox.mjs`) following the existing `TERAX_USAGE_ENDPOINT` env model: scratch HOME/APPDATA (all `terax-*.json` LazyStores), a **redirected WebView2 user-data-dir** (localStorage `terax-palette-mru`/`terax.layout.mode`/`terax.sidebar.*` + IndexedDB `terax-bg-images` live here, *not* in APPDATA), a pre-seeded sandboxed workspace cwd, empty palette MRU, a **namespaced keyring service** (see additions), and `fake-claude`/`fake-usage` wired in. Teardown clears all four stores + the keychain entries + the autostart registry key.

## Complete action-coverage matrix

| Surface | Representative actions | Drive mechanism | Assert mechanism | Status |
|---|---|---|---|---|
| **Command palette** | open; run any registered command by id; content-search mode (`#`); history mode (`>`) | `setCommandPaletteOpen` + `runCommandById(id)`; `#terax-command-palette-input` for DOM path | `commandPaletteOpen`; per-command store/DOM; results list | ✅ |
| **Keybinding-only** | `tab.selectByIndex` (Mod+1–9), `tab.next/prev`, `pane.focusNext/Prev`, zoom/zen, block-nav, `terminal.clear`, `editor.undo/redo` | `runShortcut(id, index?)` (bypasses platform modifier matching) | store/DOM per action; `activeId`, `activeLeafId` | ✅ |
| **Terminal — type/run** | submit command, interrupt, retry-spawn-on-CR, word/line nav, copy/cut/paste | `submitToLeaf` / textarea sendKeys; keymap key dispatch | `serialize(leafId)`/`getBuffer`; OSC 133 marks; clipboard | ✅ |
| **Terminal — scrollback/cmd search/marks** | command-mark search (Inputs), find-in-terminal (next/prev), clear | header Search button; `getSearchAddonForLeaf`; `runShortcut('terminal.clear')` | `getCommandMarksForLeaf`; addon `resultCount`; empty buffer | ✅ |
| **Tabs** | new (terminal/block/private/editor/preview/notes/board/tasks/director/topology/flow/git-graph); close; switch; **rename, duplicate, close-others, move-to-space, reorder, pin** | `runCommandById('tab.new'…)`; bus `reorderTab`/`moveTabToSpace`/`duplicateTab`; DOM for rename/pin (`aria-label "Rename tab"`) | `tabsSnapshot()`; `[data-tab-active]` | ✅ (within-space drag-reorder via bus `reorderTab`; the *gesture* itself ⚠️) |
| **Panes / splits** | split right/down; add terminal/note/tasks pane (below+right); focus next/prev; rename pane; close pane; 4-way split-into | `runCommandById('pane.*')`; `runShortcut('pane.focusNext')`; bus `splitActivePane`; DOM header ⋮ for split-into | `tabsSnapshot()` paneTree leaf count; `paneTitles.titles[leafId]` | ✅ |
| **GRIDS (multi-pane R×C)** | create grid, per-pane launch cmd | **bus `newGridTab(rows,cols,cwd)`** (no command/keybinding exists); GridDialog DOM for UI-path | `tabsSnapshot()` leaf count == rows×cols; per-pane `getBuffer` | 🔧 (bus method) |
| **Pane move/redock + resize** | dnd-kit move, gutter resize | bus `movePane` for the *result*; synthetic pointer-drag for the gesture | `tabsSnapshot()` tree; panel sizes | ⚠️ (gesture flaky; result ✅ via bus) |
| **Auto-color** | Manual/Automatic × Muted/Vibrant/Pastel; per-type defaults | `setPaneColorMode`/`setPaneColorPalette`/`setPaneColor*` (bus) | `usePreferencesStore` + `terax-settings.json`; **assert color *presence*, not exact hex** (`Math.random()*360` seed) | ✅ (with seed-pin for exact hex 🔧) |
| **Notes** | create tab/pane; edit content | `runCommandById('tab.newNotes')`; textarea sendKeys | `docsStore.notes[docId]`; `terax-workspace-docs.json` | ✅ |
| **Tasks** | add/toggle/edit/move/delete/clear | DOM (`aria-label "Add a task"`, hover-gated move/delete) | `docsStore.tasks[listId]` | ✅ (hover-gate → force-hover) |
| **Board (kanban)** | add/edit/move/delete card; rename column | DOM inputs/textareas | `docsStore.boards[boardId]` | ✅ |
| **AI — chat** | send, stop, slash (`/plan`), `#snippet`, attach file, tool approve/deny, plan apply, continue-after-cap | composer DOM; **`fake-claude` for the run** | `chatStore`/`planStore`; `terax-ai-sessions.json`; FS writes (sandbox) | ✅ (requires fake-claude) |
| **AI — sessions** | new/switch/delete; **rename** | SessionPicker DOM; **rename: bus `chatStore.renameSession` (no UI)** | `chatStore.sessions` | ✅ (rename store-only) |
| **AI — model/agent/snippet select** | pick model, favorite, switch persona | ModelDropdown / AgentSwitcher DOM; or bus setters | `chatStore.selectedModelId`; `agentsStore.activeId` | ✅ |
| **Search-across-conversations** | — | **none — feature does not exist** | — | ⚠️ missing feature; build before covering |
| **Settings webview (~50 controls)** | theme mode, fonts, pane colors, usage guard, autostart, models/providers, shortcuts rebind | **bus setters** (bypass); embedded WebDriver window-switch for UI-wiring | `usePreferencesStore` + `terax-settings.json` + `terax://prefs-changed` | ✅ state/persistence; UI-wiring 🔧 (spike) |
| **Provider API keys** | save/clear cloud key | DOM ProviderKeyCard; OS keychain via `secrets_set` | masked-key DOM / Connected badge; `secrets_get` | 🔧 (namespace keyring) |
| **Theme management** | select (palette sub-page); create/edit (multi-window); import; remove; bg image | `runCommandById('theme.pick')`→`theme:<id>`; bus `setThemeId`; `<input type=file>` sendKeys | `themeId`; `terax-custom-themes.json`; IndexedDB `terax-bg-images` | ✅ select; ⚠️ create/edit multi-window choreography |
| **Sidebar** | toggle; select view (Files/SCM/Agents); resize; focus explorer; status badges | `runCommandById('sidebar.toggle')`; rail DOM; `runShortcut('explorer.focus')` | localStorage `terax.sidebar.*`; badge DOM | ✅ |
| **Agent dock** | start director (+team template); add-to-tab; clear roster; status filter; list/graph; group collapse; per-agent context menu | DOM (no commands); `fake-claude` for live agents | `orchestrationStore` (session-only); `retryStore`; localStorage `terax.agents*` | ✅ (data-testid recommended) |
| **Agent graph** | node click/double-click/focus; pan/zoom/fit | node DOM (clickable SVG buttons); zoom/Fit buttons | active tab/leaf; "Locked: <name>" badge; **transform = screenshot or new hook** | ⚠️ pan/zoom/fit visual-only |
| **Spaces** | overview, new, switch | `runCommandById('spaces.*')`; `spaces.switch.<id>` enumerated live | `useSpaces.activeId` | ✅ |
| **Tab status pills / OSC signals** | working/waiting/done/error | `fake-claude` emits OSC 133/777 through real Rust detectors | `useTabStatusStore.statuses[tabId]` | ✅ (via fake-claude) |
| **Native dialogs** | folder picker, color picker, mic | bypass: pre-seed root; `.value`+synthetic event; stub Whisper | n/a (bypassed) | ⚠️ undrivable; bypassed at seam |
| **OS side-effects** | launch-at-login, check-for-updates, openUrl | stub / exclude from gate | `prefs.autostart`; n/a for registry/network | ⚠️ stub or out-of-scope |

## What to add to the app

All dev/test-gated via `import.meta.env.DEV` (Vite strips the dead branch — proven by `main.tsx:22`), verified against `dist` after build.

1. **`window.__KODEN_TEST__` bus** — `src/dev/testBus.ts` (NEW) + a dev-gated `useEffect` in `src/app/App.tsx` (~30 lines) + ref-backed snapshot registrations inside `useTabs.ts` and `useTerminalSession.ts`. Exposes `runCommandById`/`runShortcut`/`getStores`/`tabsSnapshot`/terminal handles/`newGridTab`+mutators/`setCommandPaletteOpen`/settings setters. **Reuses:** `commandPaletteItems`, `shortcutHandlers`, the `__teraxNewBlockTab` gate pattern (verified at `useTabs.ts:421`), the loaded `SerializeAddon` (`rendererPool.ts:206`), all Zustand singletons. **Effort: M (1–2 days; mostly re-exports).** *This single file is the keystone — it converts the xterm-text gap, the settings-webview gap, and the missing execute-by-id bus from blockers into one-liners.*

2. **Env-overridable keyring service** — `src/modules/ai/config.ts`: change the hardcoded `KEYRING_SERVICE = "terax-ai"` (verified) to read `TERAX_KEYRING_SERVICE` with `"terax-ai"` default. **This is a SOURCE change, not a launcher env var** — Windows Credential Manager is per-OS-user, NOT per-APPDATA, so scratch HOME does *not* isolate seeded keys. Plus teardown that `secrets_delete`s all keys under the sandbox service. **Effort: S.**

3. **Disposable-profile launcher** — `scripts/launch-sandbox.mjs` (NEW): scratch HOME/APPDATA **+ redirected WebView2 user-data-dir** (localStorage + IndexedDB), pre-seeded workspace root, empty MRU, namespaced keyring, `fake-claude`/`fake-usage` wired via env; teardown clears APPDATA + WebView2 dir + keychain(sandbox) + autostart registry key. **Reuses:** `scripts/fake-claude.mjs`, `scripts/fake-usage-endpoint.mjs`, the `TERAX_USAGE_ENDPOINT` model in `scripts/README-sandbox.md`. **Effort: M.**

4. **`data-testid` on ~6 gesture/visual-only controls** — GridDialog steppers + Create; AgentDock status-filter / list-graph toggle / context-menu trigger; AgentTopologyView zoom/Fit/Clear-focus; `resizable-handle` + pane-move targets. **Why:** zero `data-testid` exist; the Terax→Koden rebrand + any i18n *will* break text/aria selectors. **Effort: M (mechanical, broad).**

5. **OPTIONAL — auto-color seed-pin** (`paneAutoColor.ts`) for exact-hex assertions, and **injectable clock** for `RetryBridge.tsx`'s bare `setTimeout`+`Date.now()` so retry/usage-guard timing asserts don't wait wall-clock (or just use fake-claude's `--reset 'try again in 1 minute'`). **Effort: S–M, nice-to-have.**

## How an agent uses it

**The "CLI" is `pnpm test:e2e`** running WebdriverIO specs in `tests/e2e/`, each spec a scenario that talks to the bus via `browser.execute(() => window.__KODEN_TEST__.…)` and asserts on the returned plain-JSON snapshots. There is no new transport — the bus rides the existing WebView eval / Tauri IPC.

**Regression-first loop (autonomous-over-time):**
1. Launcher boots Koden against the disposable profile with `fake-claude` on PATH.
2. Agent loads scenario files (one per surface, mirroring the coverage matrix) — e.g. `grids.e2e.ts`: `__KODEN_TEST__.newGridTab(2,2,cwd)` → poll `whenSessionReady` on each leaf → assert `tabsSnapshot().panes.length === 4` → `submitToLeaf` a sentinel → assert `getBuffer` contains it.
3. Runner reports pass/fail per scenario; on fail the agent reads the store snapshot + `serialize` output to localize.
4. Teardown wipes the profile. Run in CI on every push; the matrix *is* the regression suite.

**Interactive exploration mode:** an agent drives the same bus ad-hoc (`runCommandById`, `getStores`, `serialize`) to probe a new feature or reproduce a bug, without authoring a permanent spec — then promotes a confirmed flow into a scenario file. The bus is effectively a bespoke-CLI-grade remote control + assertion oracle; WebDriver is the thin escape hatch for the ~5% pointer/visual residual.

## Phased plan

**Phase 0 — Spikes (½ day, do first).** (a) Embedded-provider multi-window: open settings, assert `getWindowHandles().length === 2`, `switchToWindow`, read a settings-only node. (b) Confirm `data-*` survives the prod minifier. *Decides whether settings-UI wiring is asserted end-to-end or only via the store bypass.*

**Phase 1 — MVP walking skeleton (1–2 days).** Bus (`runCommandById` + `getStores` + terminal handles) + launcher + WDIO scaffold. Drive+assert three core flows: open palette → new terminal tab → `submitToLeaf` "echo koden" → assert via `getBuffer`; create a 2×2 grid; toggle a setting via bus setter and assert `terax-settings.json`. Proves the spine.

**Phase 2 — Full coverage (1–2 weeks, incremental).** One scenario file per matrix surface. Wire `fake-claude` for AI/status/retry. Add the keyring env override + the ~6 `data-testid`s. Land the pointer-gesture specs (or declare them screenshot-only / out of scope).

**Phase 3 — Autonomous-over-time (ongoing).** Run the matrix in CI on every push; agents add a scenario whenever a new action lands; quarterly low-frequency settings-UI-wiring pass via the embedded window-switch. The matrix doubles as the living coverage ledger.

## Risks & open questions for Kosta

- **Embedded multi-window switch is UNVERIFIED for separate Tauri WebviewWindows** (vs webviews-within-one-window). The whole "true settings-UI coverage in one session" claim rests on the Phase-0 spike. **Decision:** if it fails, accept that the store-setter bypass asserts state+persistence but *not* that a settings widget's `onChange` is wired — a broken control passes green. OK?
- **Keyring is machine-global** (`KEYRING_SERVICE` hardcoded, verified). Without the source-level env override (addition #2), the harness writes provider keys into the **real** machine Credential Manager under the same service as the real app. **Decision:** approve the source change, or accept guaranteed `secrets_delete` teardown as the only isolation.
- **Pure pointer-DnD** (pane move/redock, resize gutters, within-space tab reorder, mini-window geometry, graph pan/zoom/fit) is the one place "complete" is genuinely hard. **Decision:** cover the logical *result* via bus methods + screenshot the gesture, or declare the raw gestures out of scope?
- **Search-across-conversations does not exist** — the brief lists it as required coverage but `chatStore` has no `searchSessions`/`searchMessages`. **Decision:** it's a feature to *build*, not a test to write — confirm it's out of harness scope until then.
- **Dev bus = full store control + arbitrary command exec + terminal write; embedded WebDriver = an HTTP automation port.** Both MUST be compile/DEV-gated out of release. **Decision:** accept the requirement to grep `dist` for `__KODEN_TEST__` and the webdriver feature on every release build.
- **Flakiness watch:** msedgedriver/WebView2 version drift hangs silently (the embedded provider avoids it — another reason to prefer it); never use fixed sleeps (gate on `whenSessionReady` + sentinel poll); assert auto-color *presence* not hex unless seed-pinned; always read effective shortcut bindings live from `usePreferencesStore.shortcuts`, never hardcode chords.
- **Open question:** is `pnpm test:e2e` the desired entry point, or do you want a thin wrapper CLI (`koden-harness run <surface>`) for the autonomous agent to call? Either rides the same bus.