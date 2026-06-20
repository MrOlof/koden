---
title: Koden Overhaul Plan — Terax → Koden rebrand + own update channel
created: 2026-06-20
status: PLANNED (not executed) — execution doc, build on it; READ-ONLY verification, no source touched
supersedes: parameterized <NEWNAME> sections of fork-rebrand-and-onboarding-2026-06-19.md (now concretized for "Koden")
verified_against: actual working tree 2026-06-20 (uncommitted overnight + this-session edits); line numbers below are CURRENT unless marked GUI-VERIFY
upstream: forked from crynta/terax-ai (Apache-2.0) — attribution KEPT, not erased
---

# Koden Overhaul Plan (2026-06-20)

Concrete execution plan to rebrand **Terax → Koden** (*KOsta waDENfalk*; *koden* = "the code" in Swedish) and stand up Koden's own signed, opt-in update channel. Builds on `.memory/fork-rebrand-and-onboarding-2026-06-19.md` + `.memory/feature-research-2026-06-19.md`, corrected by this session's verification. Strategy: **rename the identity/user-facing layer; keep `terax` as the internal runtime codename.** No blind `s/terax/koden/`.

---

## ┌─ FINAL STATE (2026-06-20, latest) — FULL RENAME EXECUTED, D4 REVERSED ─┐

The "keep `terax` as internal codename" strategy below is **OBSOLETE**. At the user's
explicit request the codename was fully renamed too: a complete `terax`→`koden` sweep
(3 case passes across 118 source files, run via a review→apply→verify agent workflow) is
DONE and pushed. So now:
- **No `terax` anywhere in source** (verified `git grep -i terax` outside `.memory/` = 0).
  Env vars are `KODEN_*`, OSC token `notify;Koden;`, dir `~/.koden`, stores `koden-*.json`,
  keyring `koden-ai`, theme id `koden-default`, crate `koden`/`koden_lib`, Tauri events `koden:*`.
- Files renamed: `TERAX.md`→`KODEN.md`, `terax-default.ts`→`koden-default.ts`, `terax-icon.png`→`koden-icon.png`.
- **All upstream attribution REMOVED** too (private personal use, never distributed): LICENSE is
  `Copyright 2026 Kosta Wadenfalk` only; no crynta in tauri.conf copyright / Cargo authors /
  About panel / README / ROADMAP / CONTRIBUTING. (Apache-2.0 attribution only binds on
  distribution; if ever made public/shipped, restore the crynta credit.)
- **Repo is private, fresh single-commit history**, sole contributor MrOlof, at `github.com/MrOlof/koden`.
- Consequence accepted: renaming storage keys resets local app data (settings, themes, API keys) once.
- `.memory/` files were intentionally EXCLUDED from the rename — they still say "terax" because
  they document the rebrand history; renaming them would make them nonsense. They are stale-by-design.
- `tsc` passed; `cargo check` run for the Rust side; `ci.yml` runs cross-platform on push.

Everything below is historical record of the earlier (superseded) "keep codename" plan.

## ┌─ EXECUTION LOG (2026-06-20, later session) ─┐

Reconciled actual working tree vs this plan (two Explore passes). **The §2A user-facing
identity rename + the CI/nix/installer slug repoint + the updater endpoint repoint + the
`autoUpdateCheck` pref + the Ask-Terax popup cut were ALREADY DONE** in the uncommitted tree
(the overnight "identity rename"). This session resolved the remaining decisions and shipped
the bundle-id + website pieces:

- **D1/D5 — DONE:** bundle id `app.crynta.terax` → **`app.mrolof.koden`** (`tauri.conf.json:5`
  + hardcoded `AboutSection.tsx` Bundle ID row). Accepting one-time appdata/keyring reset.
- **D3 — DONE (drop):** Website rows removed — `AboutSection.tsx` Website `<dt>/<dd>` + `WEBSITE`
  const + `Globe02Icon` import gone; `README.md` Website/Docs/Website-source links collapsed to a
  single GitHub link. OpenRouter referer left as harmless placeholder.
- **D6 — already DONE:** productName = "Koden" (was applied in the overnight rename).
- **D7 — KEEP (not cut):** Whisper stays. It's a composer-scoped record→OpenAI-`whisper-1`→text
  hook (`useWhisperRecording.ts`), no per-terminal concept. Kosta is building a proprietary
  per-terminal voice; reuse the ~60% capture skeleton (MIME nego, idle/recording/transcribing
  state machine, stream teardown) by refactoring into a backend-agnostic
  `useVoiceCapture({ onResult, transcribe })`; throw away the OpenAI backend + composer wiring.
- **D8 — KEEP:** Mod+J "Ask AI about selection" gesture + palette command stay (only the popup
  was bloat, already cut).
- Cargo `authors` → `["Kosta Wadenfalk", "crynta"]` (crate name stays `terax` codename, D4).
- `tsc --noEmit` clean after edits.

**MANUAL STEPS — STATUS:**
1. ~~Mint Koden minisign keypair~~ **DONE 2026-06-20** — generated **passwordless** via
   `npx @tauri-apps/cli signer generate -w ~/.koden-updater.key --password "" --force`
   (the `pnpm tauri` wrapper failed on pnpm-11 verify-deps-before-run; npx bypasses it).
   Public key wired into `tauri.conf.json:91`; private key in CI secret `TAURI_SIGNING_PRIVATE_KEY`
   on `MrOlof/koden`; `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` set empty (passwordless). Private key
   file at `~/.koden-updater.key` (+ `.pub`) — **back this up; losing it breaks all future updates.**
2. ~~Create the real `MrOlof/koden` GitHub repo~~ **DONE 2026-06-20** — public, empty, created via
   `gh` (account MrOlof). URL: https://github.com/MrOlof/koden
3. SignPath `project-slug: 'terax-ai'` (`signpath-test.yml:75-76`) → real Koden SignPath project.
   **STILL OPEN** — optional (Windows Authenticode only); needs a signpath.io OSS account.

Auto-update stays safe meanwhile: `autoUpdateCheck` defaults OFF; manual check still works but
won't verify until the pubkey is replaced (step 1).

## ┌─ DECISIONS NEEDED FROM KOSTA (blockers — set before Phase 1) ─┐

| # | Decision | Recommendation | Why it blocks |
|---|---|---|---|
| D1 | **Bundle-id triple** `app.<org>.koden` | **`app.mrolof.koden`** (reverse-DNS convention; lowercase). The `crynta` segment MUST change regardless. | Derives appdata/config/cache dir, keyring service identity, single-instance lock, autostart key, updater install identity. Changing the org segment ALONE already orphans existing installs → decide migrate-vs-reset (D5). Goes in `tauri.conf.json:5` + hand-synced literal `AboutSection.tsx:93`. |
| D2 | **GitHub repo owner/slug** | **`MrOlof/koden`** | Every updater endpoint, CI slug guard, nix glob, and "View on GitHub"/"Report issue" link keys off this. CODEOWNERS/issue-templates still say `crynta`. |
| D3 | **Website domain** (replaces `terax.app` / `terax.ai`) | If none yet: **drop the Website row + leave OpenRouter referer as a harmless placeholder**, fill later. Else name it. | Used by `AboutSection` WEBSITE, OpenRouter `HTTP-Referer`, nix `homepage`, `security@` contact. Cosmetic, non-blocking — but pick a policy. |
| D4 | **Keep `terax` as runtime codename?** (env vars, OSC token, `~/.terax`, store filenames, keyring, localStorage) | **YES** (strongly) | Renaming orphans user data / desyncs two-ended wire protocols for zero user-visible gain. This is the whole "rename identity, keep contracts" thesis. |
| D5 | **Bundle-id change → migrate appdata or accept one-time reset?** | **Accept reset** (fresh fork, ~no prior installs) — simplest. Optional: appdata-copy migration on first launch. | The moment `crynta`→`mrolof` lands, old appdata/keyring/stores look "reset" unless you copy the old dir. For a personal fork this is fine. |
| D6 | **`productName` "Terax" → "Koden"?** | **YES, change it** | Cleaner Task Manager / installer / file names. BUT triggers the lockstep artifact-name chain (see §3 coupling). If kept "Terax", old brand leaks in process name + installers. |
| D7 | **Cut Whisper/voice dictation?** | Confirm — if never wanted, **cut** (also drop "voice input" from `tauri.conf.json:87`). | `ai/hooks/useWhisperRecording.ts` + mic wiring in `ai/lib/composer.tsx`, `AiChat.tsx`. Independent of rebrand; bundles into the bloat pass. |
| D8 | **Cut Mod+J "Ask AI about selection" gesture too, or just the popup?** | **Cut popup only** (default); cut gesture only if Kosta wants Mod+J gone. | Popup cut is safe (§4). Gesture lives in `shortcuts.ts:248-253` + `commands.ts:429-437`. |

**Already settled (CLAUDE.md / memory):** name = **Koden**; npm reserved **`@mrolof/koden`** (placeholder `v0.0.1` published 2026-06-20, name-reservation only — Koden ships as installers); keep upstream Apache-2.0 attribution to `crynta/terax-ai`.

**└─ Nothing below should be executed until D1, D2, D6 are confirmed. ─┘**

---

## 1. Top-level corrections to the baseline docs (READ FIRST)

The 2026-06-19 docs predate this session. These override them:

1. **`AgentBusBridge.tsx` is LIVE and load-bearing — DO NOT CUT.** It reads `~/.terax/agent-bus.jsonl` (wired `App.tsx:375`, mounted `App.tsx:2184`) and recovers `subagent-start` by `tool_use_id` via the new `src/modules/orchestration/lib/subagentBus.ts` `extractSubagentStarts` (tolerant of corrupt JSONL). feature-research **Open Question #1 is effectively resolved** (reader half). The debloat "cut AgentBusBridge" verdict is now **WRONG**. Treat `AgentBusBridge.tsx` + `subagentBus.ts` as protected.
   - *Nuance (not a rename concern):* the **writer** half is still old — nothing appends `agent-status` lines to `agent-bus.jsonl`; `App.tsx:502` only truncates it on boot, and `agent.rs:23-32` writes subagent lines untagged to `director-bus.jsonl`. So per-pane `agent-status` may still be starved at runtime. Out of scope for rebrand; **GUI-VERIFY** before relying on the live pane status.
2. **Auto-updater is already half-demoted.** `useUpdater.ts:18 AUTO_UPDATE_DISABLED = true` (gated `:157`). Manual "Check for updates" in About still works. Endpoint + keypair repoint still outstanding before flipping back.
3. **New localStorage contract this session:** `terax.terminalSearch.mode` (`TerminalHistoryPopover.tsx:47`) from shipped scrollback search. Plus the doc omitted `terax.agentCommand` (`agentCommand.ts:1`) and `terax-ui-bg-kind-shadow` / `terax-ui-bg-image-shadow` (`preferences.ts:17/18`). True localStorage `terax.*`/`terax-*` count is **19**, not ~16.
4. **`getAgentCommand()` default is still `cm`** (`agentCommand.ts:5`) — pre-existing Tier-0 bug, unrelated to rebrand, noted so it isn't mistaken for rename fallout.

---

## 2. Concrete rename mapping — Koden

Casings: Display **`Koden`** · kebab **`koden`** · snake **`koden`** · bundle-id **`app.mrolof.koden`** (pending D1) · npm **`@mrolof/koden`**.

### 2A. RENAME → Koden (identity / user-facing) — verified, line-corrected

| Item | File:line (current) | Action |
|---|---|---|
| `productName` | `src-tauri/tauri.conf.json:3` | `"Terax"` → `"Koden"` (triggers §3 coupling chain) |
| Main-window title | `tauri.conf.json:16`, `tauri.windows.conf.json:7` | → `"Koden"` (window LABELS `main`/`settings` stay — internal) |
| short/longDescription | `tauri.conf.json:86/87` | rewrite; drop "voice input" if D7=cut |
| copyright / publisher | `tauri.conf.json:39/41` ("Crynta") | ADD Koden/Kosta line — **keep** Crynta attribution, don't replace |
| **Bundle id** | `tauri.conf.json:5` + hand-synced literal `AboutSection.tsx:93` | `app.crynta.terax` → `app.mrolof.koden` (D1) — orphans appdata/keyring/stores (D5) |
| Updater pubkey | `tauri.conf.json:91` (crynta minisign `3BABFD8AB60E3469`) | → new Koden minisign pubkey (§5 step 1) |
| Updater endpoint | `tauri.conf.json:92-93` | → `https://github.com/MrOlof/koden/releases/latest/download/latest.json` |
| Updater Linux API URL | `useUpdater.ts:9-10` | → `https://api.github.com/repos/MrOlof/koden/releases/latest` (last-check key `:7` is a contract — KEEP) |
| About `REPO_URL` | `AboutSection.tsx:11` | → `https://github.com/MrOlof/koden` (powers View-on-GitHub `:134` + Report-issue `:143`) |
| About `WEBSITE` | `AboutSection.tsx:12` (+ text `:117`) | → Koden domain or drop row (D3) |
| About name fallback | `AboutSection.tsx:25` `useState("Terax")` | → `"Koden"` |
| About bundle-id literal | `AboutSection.tsx:93` | hand-sync with `tauri.conf.json:5` |
| About repo link TEXT | `AboutSection.tsx:106` | link target → new repo; **keep** "forked from crynta/terax-ai" credit |
| npm name | `package.json:2` | `"terax"` → `"@mrolof/koden"` (license `Apache-2.0` stays) |
| Cargo name/desc/authors | `Cargo.toml:2/4/5` | → koden; authors KEEP crynta as contributor (Apache §4) |
| Cargo lib `terax_lib` | `Cargo.toml:15` + call site `main.rs:17` | optional → `koden_lib` (code-wide refactor if done; low value) |
| AI persona | `ai/config.ts:730` (SYSTEM_PROMPT) + `:781` (LITE) | "You are Terax…" → "You are Koden…" |
| OpenRouter attribution | `ai/lib/agent.ts:158` (`HTTP-Referer: https://terax.ai`) / `:159` (`X-Title: "Terax"`) | → Koden domain + `"Koden"` |
| Notification persona | `LocalAgentNotificationsBridge.tsx:8` (`AGENT="Terax"`), `:66/68/70` | → "Koden" ("Koden needs your approval" / "Koden run failed" / "Koden finished") |
| Updater dialog copy | `UpdaterDialog.tsx:95/96` (`Terax v…`), `:100` ("Restart Terax…") | → Koden |
| Updater Linux install cmds | `UpdaterDialog.tsx:22/24` (`Terax_${v}_amd64.deb`, `Terax-${v}-1.x86_64.rpm`) | → `Koden_…` (tracks productName artifacts, §3) |
| Composer placeholder | `AiComposerInput.tsx:280` ("Ask Terax anything…") | → "Ask Koden anything…" |
| Chat header | `AiChat.tsx:212` (`title="Ask Terax anything"`) | → "Ask Koden anything" |
| Mini-window | `AiMiniWindow.tsx:543` (`alt="Terax"`), `:546`, `:549` | → Koden (alt + empty-state copy) |
| Theme display name | `theme/themes/terax-default.ts:5/6` (name "Terax Default", desc) | display → Koden; **KEEP id `terax-default`** (persisted) |
| Settings copy | `ModelsSection.tsx:340`, `GeneralSection.tsx:526/587`, `AgentsSection.tsx:398`, `NotificationBell.tsx:223` | Terax → Koden in user-visible strings |
| HTML titles | `index.html:7` (`Terax`), `settings.html:7` (`Terax — Settings`) | → Koden |
| Window-title fallback | `useWindowTitle.ts:6` `APP_NAME="Terax"` | → `"Koden"` |
| Shell verbs (Windows) | `installer-hooks.nsh:6-25` (regkey `OpenInTerax`, "Open in Terax", `terax.exe`) | → `OpenInKoden` / "Open in Koden" / `koden.exe` (§3) |
| Info.plist cam/mic | `Info.plist:6/8` | "…within Koden…" |
| nix artifact/url/pname/bin/homepage | `nix/package.nix:12/16/20/29/32/60/62/72` | repoint repo + `Koden_*` artifacts + bin `koden` + homepage (§3) |
| Assets | `src-tauri/icons/*`, `public/logo.png`, root `terax-icon.png` | regen via `tauri icon`; `/logo.png` path itself can stay |
| Docs | `README.md`, `WORKSPACE.md`, `ROADMAP.md`, `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `TERAX.md`, `.github/*`, `.coderabbit.yaml`, `CODEOWNERS`, ISSUE_TEMPLATE/* | rebrand prose; KEEP "forked from crynta/terax-ai" |

### 2B. KEEP as internal codename `terax` — runtime contracts (reaffirm D4)

Renaming any of these orphans user data or desyncs a two-ended wire protocol for zero user-visible benefit.

**Wire / backend contracts:**
- Env vars: `TERAX_TERMINAL` (`shell_init.rs:108,455`), `TERAX_BLOCKS` (`:110`), `TERAX_USER_ZDOTDIR` (`:206,485`), `TERAX_SESSION` (`session.rs:137`), `TERAX_USAGE_ENDPOINT` (`usage/poll.rs:44`) + all `pty/scripts/*.{zsh,bash,fish,ps1}` consumers.
- OSC token `notify;Terax;` — emit `agent.rs:17`; match `agent_detect.rs:11` (`TERAX_MARKER`); FE turn-mark match `commandMarks.ts:326` (+ `scanTurns`, this session) + test `:84`. Legacy `terax;notify` `agent.rs:240`. **This session's scanTurns/turn-capture depends on the `Terax` token** — confirmed.
- `OWNED_MARKERS = ["notify;Terax;","terax;notify","director-bus.jsonl"]` `agent.rs:11` (keep legacy markers for hook cleanup).
- `~/.terax` dir + `director-bus.jsonl` / `agent-bus.jsonl` / `terax.ps1` — `agent.rs:24-27`; FE `App.tsx:371,375,382,502,1391`.
- Tauri events `terax:agent-signal` / `terax:usage-signal` / `terax:retry-signal`.
- CWD sentinel `__TERAX_CWD_…` + `__terax_rc` — `session.rs:44,125,140`.
- Keyring service `terax-ai` — `ai/config.ts:1` (`KEYRING_SERVICE`) + Rust keyring access.

**Persisted store/key contracts (renaming = silent data loss):**
- LazyStore files: `terax-settings.json` (`settings/store.ts:132`), `terax-spaces.json` (`spaces/lib/store.ts:21`), `terax-workspace-docs.json` + `.bak.json` (`docsStore.ts:20/27`), `terax-custom-themes.json` (`customThemes.ts:5`), `terax-ai-agents.json` (`ai/lib/agents.ts:82`), `terax-ai-sessions.json` (`sessions.ts:11`), `terax-ai-snippets.json` (`snippets.ts:12`), `terax-ai-todos.json` (`todos.ts:12`).
- IndexedDB `terax-bg-images` (`bgImageStore.ts:1`).
- `.terax-theme` extension (`themeFiles.ts:8`).
- Theme **id** `terax-default` — `settings/store.ts:24`, `theme/types.ts:67`, `themes/terax-default.ts:4` (+ filename). Keep id; rename display name only.
- **localStorage keys (19 — full current set):** `terax.sidebar.width/.view/.collapsed` (`useSidebarPanel.ts:19/20/25`), `terax.layout.mode` (`useLayoutMode.ts:5`), `terax.grid.recentCmds` (`GridDialog.tsx:20`), `terax-palette-mru` (`mru.ts:4`), `terax.agentsView/.agentsCollapsedTabs/.agentsStatusFilter` (`AgentDock.tsx:57/58/59`), `terax-ui-theme-shadow` + `terax-ui-theme-id-shadow` (`ThemeProvider.tsx:51/52`), `terax-ui-bg-kind-shadow` + `terax-ui-bg-image-shadow` (`preferences.ts:17/18`), `terax-ui-mini-window-geom` (`useMiniWindowGeometry.ts:12`), `terax.agentCommand` (`agentCommand.ts:1`), `terax:updater:last-check` (`useUpdater.ts:7`), `terax.terminalSearch.mode` (`TerminalHistoryPopover.tsx:47` — NEW this session).
- **Boot-script duplicated key (3-way hand-synced):** `terax-ui-theme-shadow` is hardcoded in `index.html:11` AND `settings.html:11`, mirroring `ThemeProvider.tsx:51`. If ever renamed, all three change together or theme flashes on boot. (This is the hand-synced literal the prompt asked to confirm — confirmed.)

### 2C. SAFE cosmetic (rename or leave wholesale, not load-bearing)

CSS `--terax-*`/`terax-*` (`src/styles/globals.css`, ~34) + ~22 className consumers across ~14 files; `[terax]`/`[terax-webgl]` log prefixes (~22); Rust thread names `terax-*`; window globals `__teraxTerm`/`__teraxNewBlockTab`/`data-terax-*`; atomic-write temp suffixes `.json.terax-tmp`/`.__terax_tmp__`; test fixtures (`tabLabel.test.ts`, `commandMarks.test.ts:84`, `subagentBus.test.ts:78`). Do these LAST (Phase 5) or never.

### 2D. Data-orphaning warning (the forced data-touch)

Four items orphan data if renamed → KEEP as `terax` (D4): **bundle id** (but `crynta` segment MUST change, D1/D5), **keyring `terax-ai`**, **LazyStore `terax-*.json`**, **localStorage `terax.*`/`terax-*`**. The ONLY forced data-touch is the bundle-id org segment `crynta`→`mrolof` (+ updater endpoint + key), unavoidable the moment you stop pointing at crynta. Decide migrate-vs-reset (D5); recommend reset.

---

## 3. External-endpoint inventory + soft update channel

### 3A. Endpoint inventory

**REPOINT — app-identity (defines "this app is Koden"):**

| # | File:line | Current | Target |
|---|---|---|---|
| 1 | `tauri.conf.json:92-93` updater endpoint | `github.com/crynta/terax-ai/releases/latest/download/latest.json` | `github.com/MrOlof/koden/...` |
| 2 | `tauri.conf.json:91` pubkey | crynta minisign `3BABFD8AB60E3469` | new Koden pubkey |
| 3 | `tauri.conf.json:5` identifier | `app.crynta.terax` | `app.mrolof.koden` (D1) |
| 4 | `useUpdater.ts:9-10` Linux API | `api.github.com/repos/crynta/terax-ai/releases/latest` | `.../MrOlof/koden/...` |
| 5 | `AboutSection.tsx:11` REPO_URL | `github.com/crynta/terax-ai` | `github.com/MrOlof/koden` |
| 6 | `AboutSection.tsx:93` bundle-id literal | `app.crynta.terax` | sync with #3 |
| 7 | `AboutSection.tsx:106` link text | `crynta/terax-ai` | target→new; keep "forked from" credit |
| 8 | `AboutSection.tsx:12,117` WEBSITE | `https://terax.app` | Koden domain or drop (D3) |
| 9 | `agent.ts:158-159` OpenRouter attr | `HTTP-Referer: terax.ai`, `X-Title: Terax` | Koden domain + "Koden" |

**REPOINT / VERIFY — CI + packaging (release-flow identity):**

| # | File:line | Current | Note |
|---|---|---|---|
| 10 | `.github/workflows/update-nix-sources.yml:45` | `gh release download --repo crynta/terax-ai` | hardcoded slug → `MrOlof/koden` |
| 11 | `.github/workflows/signpath-test.yml:12` | guard `github.repository == 'crynta/terax-ai'` | → `MrOlof/koden` else job never runs on fork |
| 12 | `release.yml:91` | `releaseName: "Terax ${ref_name}"` | → "Koden …" |
| 13 | `release.yml:152-153` | asserts `usr/bin/terax` in AppImage | follows productName → `usr/bin/koden` (D6) |
| 14 | `nix/package.nix:12,16,20` | `crynta/terax-ai/...Terax_*` | slug + `Koden_*` artifacts |
| 15 | `nix/package.nix:60` | `install -Dm755 usr/bin/terax` | → `koden` (D6) |
| 16 | `nix/package.nix:72` | `homepage = terax.app` | Koden domain |
| 17 | `installer-hooks.nsh` (all) | `terax.exe` + `OpenInTerax` + "Open in Terax" | `koden.exe` / `OpenInKoden` (D6) |
| 18 | `Cargo.toml:5` | `authors = ["crynta"]` | add Kosta/MrOlof (keep crynta) |
| 19 | `CODEOWNERS:1`, ISSUE_TEMPLATE `config.yml:4,7`, `bug_report.yml:10` | `@crynta`, `crynta/Terax-AI` | → `@MrOlof` / `MrOlof/koden` |

**productName coupling chain (CRITICAL, D6):** `tauri.conf.json:3 productName` drives bundle artifact names (`Koden_<ver>_amd64.deb`, `Koden_x64.app.tar.gz`, `koden.exe`, `usr/bin/koden`) → hardcoded in `nix/package.nix`, `installer-hooks.nsh`, `release.yml:152` AppImage assertion, `UpdaterDialog.tsx:22-24` install commands. Change productName → **all change in lockstep** or release/updater/nix break.

**LEAVE — user-config / upstream-protocol / dev-only (NOT rebrand targets):**
- `usage/poll.rs:25` `api.anthropic.com`, `:26` `console.anthropic.com/v1/oauth/token`, `usage/mod.rs:27` public Claude Code `OAUTH_CLIENT_ID`, `poll.rs:54` UA `claude-code/<ver>` + `mod.rs:23` `FALLBACK_CLI_VERSION "2.1.168"` — the user's OWN Claude subscription/usage + Claude Code protocol. (Version pin is a maintenance bump, not rebrand.)
- `agent.ts` provider base URLs `api.mistral.ai`, `openrouter.ai`, z.ai/Anthropic/OpenAI/Ollama — user BYOK config. (Only the OpenRouter *attribution headers* :158-159 are identity = #9.)
- `preview/PreviewAddressBar.tsx` / `PreviewPane.tsx` `localhost:*` — user types it; not a phone-home.
- `scripts/fake-usage-endpoint.mjs`, `scripts/README-sandbox.md` `127.0.0.1:8473` — local test stub.

**ANALYTICS / TELEMETRY / CRASH REPORTING — NONE.** Confirmed: no Sentry/PostHog/analytics wired in `src/` or `src-tauri/`. `@sentry/*` / `@opentelemetry/*` appear only as transitive dev deps of msw/vitest in `pnpm-lock.yaml` (zero `import …@sentry` in `src/**`). ROADMAP/SECURITY state "No telemetry / no account." Nothing to repoint or remove.

### 3B. Soft update channel — end-to-end ("soft" = opt-in, non-forced, signed)

**Step 1 — Mint the Koden minisign keypair**
```bash
# from repo root; prompts for a password (store it in your password manager)
pnpm tauri signer generate -w ~/.koden-updater.key
```
- PUBLIC key (base64) → replace `tauri.conf.json:91 pubkey`.
- PRIVATE key + password → two GitHub Actions secrets: `TAURI_SIGNING_PRIVATE_KEY` (file contents) + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. `release.yml:80-81,112-113` already reference both — just replace values. **Never commit the private key.**

**Step 2 — Repoint endpoints**
- `tauri.conf.json:93` → `https://github.com/MrOlof/koden/releases/latest/download/latest.json`
- `useUpdater.ts:10` → `https://api.github.com/repos/MrOlof/koden/releases/latest`
- Verify `tauri.conf.json:28` CSP `connect-src` allows `https:` (it does).

**Step 3 — `latest.json` schema (Tauri updater plugin `~2.10.1`, confirmed `package.json:77`)**
`tauri-action` generates this automatically; reference shape:
```json
{
  "version": "0.9.0",
  "notes": "Release notes shown in the Install dialog.",
  "pub_date": "2026-06-20T12:00:00Z",
  "platforms": {
    "windows-x86_64": { "signature": "<.sig of NSIS .exe>", "url": "https://github.com/MrOlof/koden/releases/download/v0.9.0/Koden_0.9.0_x64-setup.exe" },
    "darwin-aarch64": { "signature": "<.sig>", "url": ".../Koden_aarch64.app.tar.gz" },
    "darwin-x86_64":  { "signature": "<.sig>", "url": ".../Koden_x64.app.tar.gz" },
    "linux-x86_64":   { "signature": "<.sig>", "url": ".../Koden_0.9.0_amd64.AppImage" }
  }
}
```
Platform keys `{os}-{arch}`. `version` = clean semver (compared to `getVersion()`; `useUpdater.ts:37-55 isNewer` mirrors this for the Linux path).

**Step 4 — GitHub Actions release flow (mostly already correct)**
`release.yml` is already repo-agnostic for uploads (`${{ github.repository }}`, `GITHUB_TOKEN`); `tauri-action@v0` builds+signs all installers and generates `latest.json` when `createUpdaterArtifacts: true` (set `tauri.conf.json:38`); the `patch-appimage-updater` job (`:170-200`) re-signs the wayland-stripped AppImage and patches its signature into `latest.json`. **Must change:** the two CI secrets (Step 1); `release.yml:91` releaseName → "Koden"; `release.yml:152` `usr/bin/terax` → `koden` (D6); `update-nix-sources.yml:45` + `signpath-test.yml:12` → `MrOlof/koden`; nix + installer-hooks + `UpdaterDialog.tsx:22-24` artifact names → `Koden_*`. Apple signing secrets (`release.yml:83-88`) optional (notarized macOS only). Trigger `on: push tags: v*` + `releaseDraft: true` stay (publish draft manually = fits "soft").

**Step 5 — Make it soft (opt-in toggle + re-enable)**
1. Add persisted pref `autoUpdateCheck: boolean` to `settings/store.ts` via existing `writePref()` (same mechanism the setup-wizard plan uses for `hasCompletedSetup`). Default: `true` once feed is live (or `false` for max non-intrusive).
2. Wire into hook: `useUpdater.ts:156-160 useEffect` currently gates on `AUTO_UPDATE_DISABLED` then `autoCheck`. Replace the constant gate with the pref; **delete/flip `AUTO_UPDATE_DISABLED`** (`:18` + early return `:157`). About's manual check (`autoCheck:false`) is unaffected.
3. Add Settings toggle in `GeneralSection` (or new "Updates" row): "Automatically check for updates" → `setAutoUpdateCheck`. The Install dialog already keeps **Later/Install** (`UpdaterDialog.tsx:151-159`) — that's the non-forced half; no change.
4. Keep the 30-min throttle (`CHECK_INTERVAL_MS` `:8`) + `LAST_CHECK_KEY` (`:7`) — anti-nag.

**Step 6 (optional, defer) — stable/beta channels**
Two manifests: `latest.json` (stable) + `beta.json` (prereleases); a `channel` pref swaps the endpoint string before `check()`. Prefer two static files over `?channel=` query (GitHub Releases serves static files). Defer until stable feed works.

---

## 4. Ask-Terax popup cut + branded-string map + bloat table

### 4A. Ask-Terax cut (VERIFIED — "cut the popup, keep the plumbing" still correct)

**Delete files:**
- `src/modules/ai/components/SelectionAskAi.tsx` (67 lines; renders literal `<span>Ask Terax</span>` `:58`)
- `src/modules/ai/hooks/useSelectionAskAi.ts` (65 lines; installs always-on document mousedown/mouseup selection listeners)

**Remove exports:**
- `ai/index.ts:5` — drop `SelectionAskAi` from `./components/lazy` re-export
- `ai/index.ts:9` — `export { useSelectionAskAi } …`
- `ai/components/lazy.tsx` — remove `SelectionAskAiProps` import (`:4`), `SelectionAskAiInner` lazy def (`:18-20`), `SelectionAskAi` wrapper (`:46-52`)

**Remove App.tsx wiring (lines MOVED this session, re-found):**
- `App.tsx:20` `SelectionAskAi,` import · `:24` `useSelectionAskAi,` import
- `App.tsx:836-840` the `useSelectionAskAi({ captureActiveSelection, askFromSelection })` call + `askPopup`/`setAskPopup`/`onAskFromSelection` destructure + `const askPresence = usePresence(Boolean(askPopup), 120)`
- `App.tsx:2204-2212` the `{askPresence.mounted ? (<SelectionAskAi …/>) : null}` render block

**KEEP (load-bearing — all 3 alt surfaces verified):** `askFromSelection` (`App.tsx:815-834`), `captureActiveSelection` (`:770`), `attachSelection` (`:796`); **Mod+J** (`shortcuts.ts:249-252` → handler `App.tsx:1214`, disabled-guard `:1255-1263`); **palette** "Ask AI about selection" (`commands.ts:430-436` → ctx `App.tsx:1853 askAiSelection: askFromSelection`). No separate terminal block-overlay "Ask AI" path. **Boundary:** rest of `src/modules/ai` untouched. Optional full-gesture removal (D8): also delete `shortcuts.ts:248-253` + `commands.ts:429-437`.

### 4B. Branded-string → Koden copy map

All 38 user-facing strings consolidated in **§2A** (rows: persona `ai/config.ts:730/781`, titles, notifications `LocalAgentNotificationsBridge`, updater dialog, composer/chat/mini-window placeholders, theme display name, settings copy `ModelsSection:340`/`GeneralSection:526,587`/`AgentsSection:398`/`NotificationBell:223`, HTML titles). **DO-NOT-touch internal codename** strings (OSC token, `OWNED_MARKERS`, `TERAX_SESSION`, `~/.terax`, `terax-ui-theme-shadow` boot literal, store filenames, keyring) per **§2B**. **Keep-as-attribution** (Apache §4): `tauri.conf.json` copyright/publisher (ADD line, don't replace), `AboutSection.tsx:106` "crynta/terax-ai" text, `LICENSE`, `retry_detect.rs:1` credit.

### 4C. Re-verified bloat table (all files confirmed present)

| Item | Verdict | Status | Files |
|---|---|---|---|
| **Ask-Terax popup** | **CUT** | confirmed safe, ~132 LOC, plumbing kept (§4A) | `SelectionAskAi.tsx`, `useSelectionAskAi.ts` |
| **Whisper / voice** | **CUT if D7** | present + cuttable; also drop "voice input" `tauri.conf.json:87` | `ai/hooks/useWhisperRecording.ts`, mic in `ai/lib/composer.tsx`, `AiChat.tsx` |
| **Auto-updater** | **DEMOTE** (repoint, don't delete) | half-done (`AUTO_UPDATE_DISABLED=true`); repoint+keypair outstanding (§3) | `modules/updater/*`, `tauri.conf.json` |
| **`AgentBusBridge`** | **DO NOT CUT** (verdict reversed) | now live + load-bearing (§1.1) | `orchestration/components/AgentBusBridge.tsx`, `lib/subagentBus.ts` |
| Web preview | KEEP | self-suspends; removal blast radius > savings | `src/modules/preview/*` |
| Git history / commit graph | KEEP | lazy-loaded `GitHistoryStackLazy.tsx`; zero startup cost | `src/modules/git-history/*` |
| Explorer + Source Control | KEEP | structural nav chrome | `modules/explorer/*`, `source-control/*` |
| Local-LLM config sprawl | KEEP (demote UI) | shared `buildLanguageModel`; inert defaults | `settings/store.ts`, `ModelsSection.tsx`, `ai/config.ts` |
| Provider/key gating | KEEP | shared AI gate | `ai/hooks/useAiBootstrap.ts`, `ai/lib/keyring.ts` |

**Setup wizard: still greenfield** — no `src/modules/onboarding/`, no `hasCompletedSetup` flag, no `SetupWizard`. The ~1-day additive plan in the rebrand doc stands unchanged (separate from rebrand; schedule after Phase 4 if wanted).

---

## 5. Phased execution order + effort sizing

Keep upstream attribution (Apache-2.0 Crynta copyright + "forked from crynta/terax-ai") in every phase.

| Phase | Scope | Files | Effort | Gate |
|---|---|---|---|---|
| **0. Decisions** | Confirm D1 (`app.mrolof.koden`), D2 (`MrOlof/koden`), D6 (productName), D5 (reset vs migrate), D3/D7/D8 | — | 15 min (Kosta) | **BLOCKS all** |
| **1. Identity + updater** | Bundle id + productName + updater pubkey/endpoint + CI slugs + mint keypair + add `autoUpdateCheck` pref + flip `AUTO_UPDATE_DISABLED`. Lockstep artifact-name chain (§3 coupling). | `tauri.conf.json`, `tauri.windows.conf.json`, `useUpdater.ts`, `UpdaterDialog.tsx`, `AboutSection.tsx` (11/12/93), `Cargo.toml`, `nix/package.nix`, `installer-hooks.nsh`, `Info.plist`, `.github/workflows/{release,update-nix-sources,signpath-test}.yml`, `CODEOWNERS`, ISSUE_TEMPLATE/*, `package.json`, 2 CI secrets | **~1 day** | Tag a `v0.9.0` test release; confirm signed `latest.json` + in-app update install end-to-end. **GUI-VERIFY updater.** |
| **2. User-facing strings + assets** | All §2A copy (persona, titles, notifications, placeholders, theme display name, settings copy, HTML titles) + icon/logo regen (`tauri icon`). Ask-Terax cut (§4A). Whisper cut if D7. | §2A string rows, `index.html`, `settings.html`, `src-tauri/icons/*`, `public/logo.png`, `terax-icon.png`; §4A files | **~0.5 day** | Build + run; **GUI-VERIFY** title bar, About panel, notifications, composer placeholder, theme name. |
| **3. Docs** | README/WORKSPACE/ROADMAP/CONTRIBUTING/SECURITY/CODE_OF_CONDUCT/.github prose → Koden; keep "forked from crynta/terax-ai". | docs + `.github/*`, `.coderabbit.yaml` | **~2 hr** | Skim render. |
| **4. (Optional, SEPARATE PROJECT) Contract migration** | ONLY if Kosta wants `terax`→`koden` runtime codename too. Two-ended protocol + appdata/keyring/store migration shims. NOT recommended — orphans data for zero user gain. | §2B contracts (all) | **Multi-day + migration code** | Out of scope; own ADR. |
| **5. Cosmetic sweep (last / never)** | §2C: CSS vars, log prefixes, thread names, window globals, temp suffixes, test fixtures. | §2C list | **~2 hr (optional)** | Lint + tests green. |

**Recommended cut line:** ship Phases 1-3 (= a fully branded "Koden" with its own signed soft-update channel and clean docs). Phase 4 = explicitly a separate, deferred project (likely never). Phase 5 = cosmetic polish, do opportunistically.

---

## Open GUI-verification items (static-only so far)
- AgentBusBridge per-pane `agent-status` may be runtime-starved (writer half old; §1.1) — verify before relying on live pane status. **Not a rebrand blocker.**
- Updater end-to-end (Phase-1 gate): signed `latest.json` fetch + install + relaunch on a real `v0.9.0` test tag.
- Phase-2 visual sweep (title bar, About, notifications, theme name, composer placeholder).
- Line numbers are current as of 2026-06-20 but the tree has heavy uncommitted churn — re-grep the literal before each edit if drift is suspected.

## Files of record
- `.memory/fork-rebrand-and-onboarding-2026-06-19.md` · `.memory/feature-research-2026-06-19.md` · `CLAUDE.md` (rebrand summary)
- Hot files: `src-tauri/tauri.conf.json`, `src/modules/updater/{useUpdater.ts,UpdaterDialog.tsx}`, `src/settings/sections/AboutSection.tsx`, `src/modules/ai/config.ts`, `src/app/App.tsx`, `src/modules/orchestration/{components/AgentBusBridge.tsx,lib/subagentBus.ts}`, `src-tauri/src/modules/{agent.rs,pty/session.rs}`, `.github/workflows/release.yml`, `nix/package.nix`, `src-tauri/installer-hooks.nsh`
