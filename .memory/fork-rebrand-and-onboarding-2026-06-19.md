---
title: terax-workspace — fork rebrand inventory + fork-bloat review + setup-wizard design
created: "2026-06-19"
updated: "2026-06-19"
status: research only — READ-ONLY pass, no source code changed; new name still TBD (parameterized as <NEWNAME>)
method: 9-agent read-only workflow — 4 rebrand-sweep hunters (frontend / rust-tauri / config-identity / docs) + settings-&-onboarding map + Ask-Terax value eval + fork-bloat eval -> setup-wizard designer -> editor
related: feature-research-2026-06-19.md (features + agent-visibility + ponytail debloat audit)
---
## TL;DR

- **Rebrand effort is moderate but front-loaded with traps.** ~570 raw hits across four zones (frontend ~230, rust ~140, config ~38, docs ~160), but the bulk are cosmetic. The danger is concentrated in ~15 breaking runtime contracts that span frontend + Rust + the installed `~/.claude` hooks and must move in lockstep.
- **The single most consequential string is the bundle identifier `app.crynta.terax`** (`src-tauri/tauri.conf.json:5`). Changing it relocates appdata/keyring/store paths and changes updater identity — every persisted LazyStore + keychain secret "resets" unless you migrate.
- **The updater currently points at upstream** (`crynta/terax-ai` releases + crynta's minisign pubkey). As shipped, the fork phones home to a repo Kosta doesn't control. This must be repointed or disabled before any release.
- **Recommended split:** rebrand the *user-facing + identity* layer (titles, productName, logos, About, docs, repo/updater endpoints), but **keep `terax` as an internal codename** for the runtime contracts (env vars, OSC token, `~/.terax`, store filenames, keyring service, `terax:` events). Renaming those buys nothing functional and risks data loss + cross-zone desync.
- **Ask-Terax verdict: cut the popup, keep the plumbing.** The `SelectionAskAi` floating button (~132 LOC, base-Terax not fork) is safe to remove — the same gesture survives via Mod+J + command palette. But the AI/agent module underneath is load-bearing (git, autocomplete, commit messages, coding agents) and must stay 100% untouched.
- **Other bloat:** voice/Whisper → cut; auto-updater → demote (repoint, don't delete); web preview, git-history, local-LLM config sprawl → keep (cheap when unused, removal blast radius outweighs the savings).
- **Setup wizard is worth it — small, one-shot, ~1 day.** It's a *discovery* fix, not a config system. Build as an in-main-window overlay that writes through the existing `store.ts` setters; add exactly one persisted boolean (`hasCompletedSetup`).

## Rebrand: Terax → \<NEWNAME\>

### How to rename safely — recommended order

Do the rename in this sequence so each layer's dependents are already in place:

1. **Pick one canonical token** and derive its forms up front. You need: `<NEWNAME>` (display, e.g. "Aurora"), `<newname>` (kebab/lower, e.g. `aurora` for package/binary names), `<newname>_lib` (snake, Cargo lib), and a bundle-id triple `app.<org>.<newname>` (reverse-DNS).
2. **Decide the runtime-contract policy first** (see migration note below). If you keep `terax` as an internal codename, steps 3–4 shrink dramatically.
3. **Identity layer (config zone):** `productName`, bundle `identifier` (+ migration), updater endpoint + new minisign keypair, window titles, copyright/publisher, `package.json` name, `Cargo.toml` package name. Then the derived artifact names in `installer-hooks.nsh`, `nix/package.nix`, `.github/workflows/*`.
4. **Backend-emitted contracts (only if you choose to rename them):** env vars + OSC token + `~/.terax` dir + `terax:` events — change Rust **and** the installed hooks **and** the frontend listeners in the same commit; keep old `OWNED_MARKERS` as legacy for hook cleanup.
5. **User-facing strings + assets:** titles, About panel, notification/persona strings, `public/logo.png`, `terax-icon.png`, docs/README.
6. **Cosmetic sweep last:** CSS `--terax-*` namespace + TSX className consumers (one pass), `[terax]` log prefixes, thread names, comments, test fixtures.

### Parameterized find-replace plan

| Form | Use for | Example targets |
|---|---|---|
| `<NEWNAME>` (Display) | titles, UI copy, persona, About, docs | `index.html:7`, `settings.html:7`, `tauri.conf.json:3/16`, `ai/config.ts:730/781`, `AboutSection.tsx` |
| `<newname>` (kebab/lower) | npm name, Cargo package, binary/artifact names, nix pname, AUR | `package.json:2`, `Cargo.toml:2`, `installer-hooks.nsh` (`terax.exe`), `nix/package.nix:32/60` |
| `<newname>_lib` (snake) | Cargo `[lib]` name + call sites | `Cargo.toml:15`, `main.rs:17`, `src-tauri/tests/*.rs` |
| `app.<org>.<newname>` (bundle-id) | Tauri identifier + the hard-coded About string | `tauri.conf.json:5`, `AboutSection.tsx:93` (literal, must be hand-synced) |
| `<newname>` internal token (optional) | env-var prefix, OSC, `~/.<newname>`, `terax:` events, keyring | only if you choose to rename contracts |

> ⚠️ A blind global `s/terax/<newname>/` will silently orphan persisted user data and desync the OSC/hook wire protocol. Do not run it. The contracts below must be handled deliberately.

### Risk table

#### Breaking — rename in lockstep or migrate

| What | Where | Consequence / Action |
|---|---|---|
| Env vars `TERAX_TERMINAL`, `TERAX_SESSION`, `TERAX_BLOCKS`, `TERAX_USER_ZDOTDIR`, `TERAX_USAGE_ENDPOINT` | `src-tauri/src/modules/pty/shell_init.rs:108/455/110/206/485`, `pty/session.rs:137`, `usage/poll.rs:44`; consumed in `pty/scripts/*` (bash/zsh/fish/ps1) | `TERAX_TERMINAL` gates **all** shell integration; rename only with `agent.rs` hook cmds + every script. `TERAX_SESSION` routes per-pane agent status. `TERAX_USAGE_ENDPOINT` is an external test/sandbox override. Rename → spawned shells lose prompt marks/status. |
| OSC 777 token `notify;Terax;<event>` | EMIT `src-tauri/src/modules/agent.rs:17`; MATCH `pty/agent_detect.rs:11` (`TERAX_MARKER`); FE match `src/modules/terminal/lib/commandMarks.ts:326` + test `:84` | Two-ended wire protocol between installed Claude hooks and the PTY parser. Change all three ends together or turn-marking silently dies. |
| `~/.claude` hook owned-markers `["notify;Terax;","terax;notify","director-bus.jsonl"]` | `agent.rs:11` (`OWNED_MARKERS`), `:61`, `:176` | Identify which hooks belong to the app for idempotent re-install/cleanup. Rename **but keep old markers as legacy** or hooks the old build wrote are orphaned. |
| `~/.terax` dir + `director-bus.jsonl` / `agent-bus.jsonl` + `terax.ps1` | `agent.rs:26-27`; FE `src/app/App.tsx:371/375/377/382/1391` | On-disk runtime dir authorized for Tauri fs writes; bus files tailed by `AgentBusBridge`. Rename needs the Tauri fs-scope capability updated **and** one-time migration of existing `~/.terax` data. |
| Bundle identifier `app.crynta.terax` | `src-tauri/tauri.conf.json:5`; hard-coded display copy `AboutSection.tsx:93` | Derives appdata/config/cache path, keyring service, single-instance lock, autostart key, updater install identity. Change = fresh install identity → all LazyStores + keyring secrets appear reset unless you copy the old appdata dir on first launch. |
| 9 Tauri LazyStore files (`terax-settings/spaces/workspace-docs(.bak)/custom-themes/ai-agents/ai-sessions/ai-snippets/ai-todos.json`) | `settings/store.ts:132`, `spaces/lib/store.ts:21`, `workspace-docs/store/docsStore.ts:20/27`, `theme/customThemes.ts:5`, `ai/lib/{agents.ts:82,sessions.ts:11,snippets.ts:12,todos.ts:12}` | Renaming filename **or** bundle id orphans settings/spaces/notes/tasks/agents (incl. `spaces.json` = the fork's grids/notes/tasks crash-safe state). Keep filenames as-is (cheapest) or migrate. |
| ~16 localStorage keys (`terax.sidebar.*`, `terax.layout.mode`, `terax.grid.recentCmds`, `terax-palette-mru`, `terax.agents*`, `terax-ui-*-shadow`, `terax-ui-mini-window-geom`, `terax:updater:last-check`) | 9 files incl. `sidebar/useSidebarPanel.ts`, `theme/ThemeProvider.tsx:51/52`, `command-palette/lib/mru.ts:4`, `orchestration/components/AgentDock.tsx:57-59` | Bundle-id-independent; rename silently resets UI prefs (data loss, not crash). `terax-ui-theme-shadow` is **duplicated** in `index.html:11` + `settings.html:11` boot scripts and must stay in sync with `ThemeProvider` or theme flashes on boot. |
| IndexedDB `terax-bg-images` | `src/modules/theme/bgImageStore.ts:1` | Rename orphans stored background images. |
| Updater endpoint + artifact names | `useUpdater.ts:10`, `UpdaterDialog.tsx:20/22/24`; `tauri.conf.json:92-94`, pubkey `:91` | Endpoint points at `crynta/terax-ai`; pubkey is crynta's minisign key (fork lacks the private key). Must repoint to fork's repo + mint a new keypair, or updater pulls upstream builds / can't verify. |
| Keyring service `terax-ai` | `src/modules/ai/config.ts:1`; Rust keyring access (same string) | Rename orphans API keys in the OS keychain (user re-enters). Coordinate FE + Rust. |
| `terax:agent-signal` / `terax:usage-signal` / `terax:retry-signal` (backend-emitted Tauri events) | Rust `lib.rs:49`, `pty/session.rs:17-18`, `usage/poll.rs:23`, `usage/mod.rs:39`; FE listeners across App.tsx + `agents/components/*Bridge.tsx` | Emitted by Rust, listened in FE. Rename in lockstep or signals stop arriving. (The `terax://` CustomEvent channels are FE-internal and safe.) |
| `.terax-theme` file extension | `theme/themeFiles.ts:8`, `editor/EditorPane.tsx:229`, `ThemesSection.tsx:155/161` | Renaming breaks opening previously-exported theme files; add back-compat on import if changed. |
| CWD sentinel `__TERAX_CWD_…` + `__terax_rc` | `src-tauri/src/modules/shell/session.rs:44/125/140` | Emitter + stdout parser co-located; change together (no persisted migration). |

#### Careful — identity / config

| What | Where | Consequence / Action |
|---|---|---|
| `productName "Terax"` | `tauri.conf.json:3` | Drives binary/artifact names; cascades to `installer-hooks.nsh` (`terax.exe`), `nix/package.nix` (`usr/bin/terax`, `Terax_*` artifacts), `release.yml` globs. |
| npm `name: "terax"` | `package.json:2` | Private, low blast radius; feeds `getName()` fallbacks. Keep `license: Apache-2.0`. |
| Cargo lib `terax_lib` | `Cargo.toml:15`, `main.rs:17`, integration tests | Optional cosmetic rename, but a code-wide refactor if done — lockstep with all `use terax_lib::…`. |
| Window titles `"Terax"` | `tauri.conf.json:16`, `tauri.windows.conf.json:7` | OS taskbar/alt-tab text. Window **labels** (`main`/`settings`) are safe — leave them. |
| OS window-title fallback `APP_NAME = 'Terax'` | `src/modules/tabs/lib/useWindowTitle.ts:6` | Drives `document.title` + Tauri `setTitle` when no tab label. |
| Updater endpoint (functional) + repo URLs | `useUpdater.ts:10`, `AboutSection.tsx:11/12/106/117` | Functional "View on GitHub" / "Report issue" / releases must point at the new repo (the credit text is separate — see attribution). |
| Deep-link / URL scheme | **none found** — confirmed no `tauri://` custom scheme, no `protocols`/`scheme` key, no `setAsDefaultProtocolClient` anywhere | Nothing to rename. The `terax://` strings are internal CustomEvent channel names, not an OS URI scheme. |
| Icons / brand assets | `src-tauri/icons/*` (+ android/ios), `public/logo.png`, root `terax-icon.png` | Regenerate icon set via `tauri icon <new.png>`; replace `logo.png` (path `/logo.png` can stay — no code change). `dist/*` is a build output. |
| macOS plist usage strings | `src-tauri/Info.plist:6/8` (camera/mic), copyright/publisher `tauri.conf.json:39/41` | Shown in macOS permission prompt + Windows installer/code-signing. Update to new brand/owner. |
| Default theme id `terax-default` | `theme/themes/terax-default.ts`, `settings/store.ts:24`, `theme/types.ts:67` | Display name/desc safe to rename; the **id** is persisted — fall back to old id when reading old prefs or saved theme selection resets. |
| Keyring/about/persona display strings | `ai/config.ts:730/781` ("You are Terax…"), `ai/lib/agent.ts:158/159` (OpenRouter `HTTP-Referer`/`X-Title`), notification strings `LocalAgentNotificationsBridge.tsx` | Update persona name + OpenRouter attribution to new brand. |

#### Safe — cosmetic (representative, not exhaustive)

| What | Where (representative) | Note |
|---|---|---|
| CSS namespace `terax-*` / `--terax-*` (~34) + TSX className consumers (~22 across ~14 files) | `src/styles/globals.css`; consumers `App.tsx:2001`, `TabBar.tsx:265/471`, `AgentTopologyView.tsx:466+`, `SurfaceLayer.tsx:85` | Rename CSS + all className strings in one pass, or leave the prefix entirely. |
| `[terax]` / `[terax-webgl]` log prefixes (~22 across ~13 files) | `terminal/lib/useTerminalSession.ts`, `rendererPool.ts`, `dormantRing.ts:7` (faintly user-visible in scrollback) | Bulk rename or leave. |
| Thread names `terax-*` (7) | `fs/watch.rs:126`, `pty/mod.rs`, `pty/session.rs`, `usage/poll.rs:386` | Debug-only. |
| Window globals / data-attrs `__teraxTerm`, `__teraxNewBlockTab`, `data-terax-*` | `tabs/lib/useTabs.ts:421`, `terminal/lib/useTerminalSession.ts:1443`, `rendererPool.ts:161/226` | Self-contained debug hatches. |
| Atomic-write temp suffixes `.json.terax-tmp` / `.__terax_tmp__` | `agent.rs:157`, `usage/poll.rs:358`, `usage/mod.rs:141`, `shell_init.rs:297/607` | Transient, never read back. |
| UI brand strings (notifications, updater dialog, settings copy) | `LocalAgentNotificationsBridge.tsx`, `UpdaterDialog.tsx:95/96/100`, `GeneralSection.tsx:526/587` | Pure copy. |
| Test fixtures | `tabLabel.test.ts:19` (`terax-ai` folder example), `commandMarks.test.ts:84` (update with OSC token) | `tabLabel` is cosmetic; `commandMarks` follows the token rename. |

### Keep as attribution

Do **not** erase the upstream credit — convert it to "forked from":

- **LICENSE** is Apache-2.0 with `Copyright 2026 Crynta` (`LICENSE:189`). Apache-2.0 §4 requires **retaining** the upstream copyright + license text. **Keep the Crynta line, add a second `Copyright 2026 <Kosta/NEWNAME>`** for the fork's contributions. No `NOTICE` file exists — consider adding one crediting `crynta/terax-ai`.
- Keep the existing fork notes: `ROADMAP.md:3` ("fork of crynta/terax-ai"), `WORKSPACE.md:3`, `.memory/INDEX.md:12`, and the About panel's visible `crynta/terax-ai` link text (`AboutSection.tsx:106`). **Split:** functional links (`REPO_URL`, updater endpoint, "Report an issue") → new brand; the credit line → keep "forked from crynta/terax-ai".
- Keep the `claude-auto-retry` port credit in `pty/retry_detect.rs:1`.

### Persisted-data migration

Renaming store filenames, localStorage keys, IndexedDB names, the keyring service, **or** the bundle id all orphan existing user data. There are two honest options:

1. **Cheapest correct:** keep the internal contract strings as `terax` (an internal codename) and only rebrand the **user-facing + identity** layer. No migration, no cross-zone lockstep on the wire protocol, no data loss. The user never sees `terax-settings.json` or `~/.terax` unless they go digging.
2. **Clean rename:** change the contracts too, but then you owe a one-time migration on first launch — copy old appdata → new bundle-id path, rename `~/.terax` → `~/.<newname>`, fall back to old localStorage keys + theme id, keep old `OWNED_MARKERS` as legacy, remove the orphaned `~/.config/fish/conf.d/terax.fish`. That's real engineering for zero user-visible benefit.

**Recommendation: option 1.** Treat `terax` as the internal codename. Rebrand titles, productName, logos, About, docs, repo/updater identity, and persona strings; leave env vars, OSC token, `~/.terax`, store filenames, keyring service, and `terax:` events alone. This is the anti-bloat, terminal-first, reuse-existing-machinery choice — and it sidesteps the entire breaking-contract table except the identity items you *must* change anyway (bundle id, updater endpoint + keypair). If you ever do rename the contracts, do it as a deliberate separate project with the migration above, not as part of the cosmetic rebrand.

## Fork bloat — what to cut

### Ask-Terax — verdict: **CUT the popup, KEEP the plumbing**

**What it is:** the `SelectionAskAi` floating "Ask Terax" button (Mod+L) that appears at the pointer after you select text in the terminal/editor. It does **not** run any LLM itself — it's a thin mouse shortcut that forwards the selection into the shared AI composer (`attachSelection`), the same composer every other AI surface uses.

**Why it's safe to cut:** it's base-Terax (upstream commit `4ac04ff`), not a fork add. It carries **zero** unique deps and zero unique AI plumbing, and it's fully redundant — the identical `askFromSelection` action is already exposed via the **Mod+J** shortcut (`shortcuts.ts` id `ai.askSelection`), the **command palette** ("Ask AI about selection"), and the terminal **block overlay**. Removing the popup loses no capability. Like all AI surfaces it's gated behind a configured key/local model, matching Kosta's "needs an API key and nobody uses it" complaint.

**Cost:** ~132 LOC (`SelectionAskAi.tsx` 67, `useSelectionAskAi.ts` 65) + lazy/barrel wrappers + ~30 lines of App.tsx wiring. The hook installs two always-on document-level `mousedown`/`mouseup` listeners that poll selection on every click/drag — a small constant runtime cost. No bundle savings (everything it touches is shared).

**Blast radius — minimal, self-contained.** To cut the popup only:
- Delete `src/modules/ai/components/SelectionAskAi.tsx` and `src/modules/ai/hooks/useSelectionAskAi.ts`.
- Remove `SelectionAskAi` from `src/modules/ai/index.ts:5` and `src/modules/ai/components/lazy.tsx:18-20/46-52`; remove the `useSelectionAskAi` export (`index.ts:9`).
- In `src/app/App.tsx`: remove the `useSelectionAskAi()` call (~836-840), the `askPopup`/`askPresence` state, and the `<SelectionAskAi>` render block (~2204-2212).

**Keep** `askFromSelection` / `captureActiveSelection` / `attachSelection` (still used by Mod+J, palette, block overlay). Optionally also remove the `ai.askSelection` shortcut + palette entry if you want the whole gesture gone (`command-palette/commands.ts`, `shortcuts/shortcuts.ts`).

> **CRITICAL — do NOT touch the rest of `src/modules/ai`.** `lib/native.ts` (git/source-control/explorer/spaces/orchestration), `chatStore`/`chatRuntime`/`composer.tsx`/`proxyFetch`/`transport`/`agent.ts`, `config.ts`/`keyring.ts`, and `agents/`/`tools/` are load-bearing — imported by ~31 files. They power editor autocomplete, commit-message AI, the coding agents, and status pills. Cutting any of it breaks core fork features.

### Other candidates

| Feature | Verdict | Why | Files |
|---|---|---|---|
| **Voice input (Whisper)** | **cut** | ~117 LOC, needs a *second* cloud key (OpenAI), bolted onto the AI composer, pure base-Terax garnish. Goes with the ask-surface direction. Confirm Kosta never wants dictation first. | `ai/hooks/useWhisperRecording.ts`, `ai/lib/composer.tsx` (mic control) |
| **Auto-updater** | **demote (repoint, don't delete)** | Code is fine (307 LOC) and load-bearing *if* he ships his own releases — but it's pointed at upstream `crynta/terax-ai` + crynta's pubkey. Either gate auto-check off until a signed feed exists, or repoint endpoint + mint new minisign keypair. | `modules/updater/*`, `App.tsx` (mount), `tauri.conf.json` (pubkey/endpoints), `Cargo.toml`/`package.json` |
| **Inline editor autocomplete** | **keep** | Off by default (zero runtime cost), shares `buildLanguageModel` provider plumbing with the agents. Removing it wouldn't shrink the fork and risks shared config. Simplify the *config UI* later, not this. | `editor/lib/autocomplete/*`, `settings/store.ts`, `ModelsSection.tsx` |
| **Web preview** | **keep** | 551 LOC, no deps, no key, self-suspends after 30s. Orthogonal to fork value but cheap; removal threads through tab types + serialize + ~8 App sites + header + palette — more blast radius than bloat. If minimalist: drop it from the header `+` menu, leave it palette-only. | `modules/preview/*`, `tabs/lib/useTabs.ts`, `spaces/lib/serialize.ts`, `App.tsx` |
| **Git history / commit graph** | **keep** | Heaviest IDE panel (1432 LOC) but already lazy-loaded (zero startup cost) and reuses existing git IPC. First IDE panel to demote if he wants to shed surface — but verify he doesn't use the commit graph first. | `modules/git-history/*`, `tabs/lib/useTabs.ts`, `spaces/lib/serialize.ts` |
| **Explorer + Source Control sidebar** | **keep** | Structural, not bloat. The fork's Agents view is implemented *as a third sidebar rail* alongside these; explorer is the fallback view. Removing would gut the navigation chrome. | `modules/explorer/*`, `modules/source-control/*`, `sidebar/*` |
| **Local-LLM provider config sprawl** | **keep (demote UI)** | ~12 base-URL/model-id fields nobody sets — genuine config clutter — but it's the shared `buildLanguageModel` plumbing the agents use, and the fields are inert defaults. Collapse the rarely-used rows behind an "Advanced" disclosure in `ModelsSection`; don't remove capabilities. | `settings/store.ts`, `ModelsSection.tsx`, `ai/config.ts`, `ai/lib/agent.ts` |
| **Provider/key gating (`useAiBootstrap`, keyring)** | **keep** | The shared gate for the *entire* AI subsystem; local models satisfy it without a cloud key. Not specific to the ask-helper. | `ai/hooks/useAiBootstrap.ts`, `ai/lib/keyring.ts`, `ai/store/chatStore.ts` |

## Setup wizard / onboarding

**Should it exist? YES — small, skippable, one-shot.** Kosta's complaint ("you don't have any idea of settings, nothing") is a **discovery** gap, not a config gap. Terax already has a deep settings store and a separate settings webview, but **nothing on first launch tells a new user any of it exists**, and the fork's signature features (conversation search, tasks/notes, grids, agent graph/list, sidebar agent status, auto tab color) have zero discovery surface beyond the per-pane `BlockWatermark` hints. There is **no** existing onboarding machinery — this is greenfield (only to add, nothing to remove).

Build it as an **in-main-window overlay** (NOT a new webview) so it reads live preferences and calls the existing setters directly, previewing themes/colors instantly via the existing `terax://prefs-changed` event loop.

### Ordered steps (each writes through existing `store.ts` setters)

| # | Card | Explains | Sets (existing setter) | Reuses |
|---|---|---|---|---|
| 0 | **Welcome + feature map** | One scannable screen: 6–8 one-liners naming the fork's standout features, each ending with its trigger (e.g. "Command palette: \<key\>"). Pure orientation, no settings. | nothing | keybinding labels from `shortcuts/shortcuts.ts` (stay correct if rebound); shadcn `Card`/`Dialog` |
| 1 | **Theme + appearance** | 10 built-in theme swatches + light/dark/system toggle; one line pointing to Settings > Themes for more. | `themeId` (`setThemeId`), `theme` (`setTheme`) | theme list from `theme/themes/index.ts`; live re-render via `ThemeProvider`/`applyTheme` |
| 2 | **Default folder** (the one new affordance) | "Where should new terminals + the file tree open?" Path display + **native folder picker** (the thing the settings text input lacks). Skippable. | `defaultFolder` (`setDefaultFolder`, normalized, updates synchronous `cachedDefaultFolder`) | Tauri dialog plugin `open({directory:true})` — verify it's already a dep before counting it |
| 3 | **Pane / tab color** (signature look) | Explains the **two real axes**: Mode = Manual or Automatic; when Automatic, Palette = Muted/Vibrant/Pastel. **Corrects the "6 modes" mental model** — there is no "variant"; it's 2 modes × 3 palettes. | `paneColorMode` (`setPaneColorMode`), `paneColorPalette` (`setPaneColorPalette`) | `PalettePreview` + `PALETTES` from `GeneralSection.tsx`; OKLCH engine `terminal/lib/paneAutoColor.ts` |
| 4 | **Optional: connect AI (BYOK)** — last, clearly skippable | "Want AI in the terminal? Add a key — or skip." Provider dropdown + key field, with "Skip — set up later in Settings > Models" as the default action. Honors anti-bloat: AI never gates anyone. | optional API key via keychain + `defaultModelId` (`setDefaultModelId`); skip sets nothing. On Finish: `setHasCompletedSetup(true)` | `secrets_*` Rust keychain path (service `terax-ai`), `terax://ai-keys-changed` broadcast — NOT the prefs store |

**Default layout on Finish/Skip:** a single full-window terminal pane in the chosen default folder — exactly what `App.tsx` boots today via `getDefaultFolder()`. The wizard does **not** auto-split into a grid; grids are a feature to *mention* in step 0, not force. Lowest-surprise, terminal-first.

### First-run gate + reopen

- **One new persisted boolean:** `hasCompletedSetup: boolean` (default `false`) in `src/modules/settings/store.ts` — add to the `Preferences` type (~line 129), a `KEY_HAS_COMPLETED_SETUP` const, `DEFAULT_PREFERENCES` (~262), the `loadPreferences()` read, and a `setHasCompletedSetup()` going through the existing `writePref()` (auto-saves + emits `terax://prefs-changed` like every other key). **No new store, no localStorage, no migration.**
- **Gate in `App.tsx`:** read `hasCompletedSetup` + `hydrated` from `usePreferencesStore` (already imported at line 46). Render `<SetupWizard/>` when `(!hasCompletedSetup || showWizard) && hydrated`. Gate on `hydrated` to avoid a flash before the store loads. Finish and Skip both call `setHasCompletedSetup(true)` — identical w.r.t. the flag, so no half-completed states.
- **Reopen — two existing surfaces, zero new machinery:** (1) a command-palette item ("Open Setup / Welcome") in `command-palette/commands.ts` that flips an ephemeral `showWizard` `useState` (so re-opening doesn't un-set the persisted flag); (2) a "Show welcome again" button in the About tab (`AboutSection.tsx`) that — since settings is a separate webview — emits a `terax://open-wizard` event that `App.tsx` catches via the `listen()` already imported at line 74.

### Anti-bloat notes

- **No new state container** — reads `usePreferencesStore`, calls `store.ts` setters directly. Only one added persisted field.
- **No new heavy dep** — UI from shadcn primitives already in `src/components/ui`; previews reuse `PalettePreview` + the theme list; folder picker uses the Tauri dialog plugin (verify present).
- **Feature mentions are copy, not interactive tours** — each a one-liner with its real shortcut from the registry. No coach-marks to maintain.
- **Captures only 4 settings inline** (theme, appearance, default folder, pane color) + one optional key. Everything else stays in the settings window, which the wizard *names* — so the wizard stays small and settings remains the single source of depth.
- **Corrects rather than invents** — the pane-color card models the 2×3 that exists, not a fabricated 6-value enum with first-class "custom" (that would be new engine work, out of scope).

**Effort: ~1 focused day.** Store flag ~20 min · `SetupWizard` overlay + 4 cards (new `src/modules/onboarding/`) ~4–5h · folder picker ~45 min · gate + `showWizard` + `listen()` wiring ~45 min · palette item + About button + event ~45 min · one vitest for the gate logic ~30 min. Risk: low — purely additive, main untouched.

## Open questions for Kosta

1. **The new name.** Pick `<NEWNAME>` and its derived forms — display, kebab/lower (binary/package), and the reverse-DNS bundle id `app.<org>.<newname>`. Everything downstream keys off this one choice. Also: new website domain (replaces `terax.app`) and the GitHub repo owner/slug for the updater + CI gates.
2. **Rename runtime contracts, or keep `terax` as an internal codename?** My recommendation is **keep them** (option 1: rebrand only user-facing + identity, no data migration). Confirm you're OK with the internal strings (`~/.terax`, `terax-settings.json`, env vars, OSC token, keyring service) staying as `terax` — users won't see them, and renaming risks orphaning existing data for zero benefit. If you want a fully clean codename, that's a separate migration project.
3. **Updater now or later?** Do you have a release pipeline + signing key ready? If not, I'd **disable auto-check** until you stand up your own signed feed (it currently pulls upstream). When ready: repoint the endpoint and mint a new minisign keypair (`tauri signer generate`).
4. **Which bloat actually goes?** Ask-Terax popup → cut (recommended). Voice/Whisper → cut **only if** you never want dictation — confirm. Everything else I marked keep/demote; tell me if you want a more aggressive trim (e.g. drop web preview / git-history from menus rather than keeping them).
5. **Wizard scope.** Are the 4 cards + optional AI step the right set, or do you want fewer (e.g. just theme + folder)? And: is the "single terminal in the default folder" first-run layout what you want, or should the wizard offer a starter grid/split?