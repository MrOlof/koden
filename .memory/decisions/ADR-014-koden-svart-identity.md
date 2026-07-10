# ADR-014 — Koden Svart: the design identity

- **Status:** Accepted (Kosta 2026-07-10: direction=Koden Svart, accent=muted spruce, font=Commit Mono, mono chrome=full; scope=full sweep; remaining calls delegated to Claude overnight)
- **Branch:** `feat/koden-svart` (worktree `Products/koden-svart-wt`, based on `feat/koden-brain@f6dc786`)
- **Grounding:** 6-agent GUI audit + design critique, 2026-07-10 (workflow wf_68ea9ced-8a9). File:line pointers below come from that audit.
- **References (owner-picked):** Ghostty (quiet, typography-led, zero chrome) × Warp (block structure). Anti-goals: gradient logos, sparkles, SaaS-violet, flag-kitsch Swedishness.

## Concept

Lean all the way into "koden" = *the code*. A near-black monochrome canvas; ONE muted
spruce-green wire carrying every focus/active/cursor/primary affordance; mono type
promoted into the structural chrome so the whole app reads as one continuous terminal,
not a GUI wrapping a terminal. Sharp corners, 1px hairlines, zero decoration.
Everything else in the room is monochrome — if a second color appears anywhere outside
the terminal ANSI palette and destructive/error states, it's a bug.

## 1. Tokens — dark (primary; light is a secondary port)

globals.css `.dark` block (converted to oklch in-file; hex given as source of truth):

| Token | Value | Note |
|---|---|---|
| `--background` | `#0a0b0b` | near-black, faint warmth |
| `--foreground` | `#ededec` | |
| `--card` | `#121313` | |
| `--card-foreground` | `#ededec` | |
| `--popover` | `#101111` | |
| `--primary` | `#5b8a6f` | THE spruce wire |
| `--primary-foreground` | `#0a0b0b` | |
| `--secondary` | `#1a1b1b` | |
| `--muted` | `#171818` | |
| `--muted-foreground` | `#7d817f` | |
| `--accent` | `#1c1e1d` | hover fills stay monochrome |
| `--accent-foreground` | `#ededec` | |
| `--destructive` | `#e5706b` | matches FAIL ruler family |
| `--border` | `rgba(237,237,236,0.12)` | hairline, slightly raised |
| `--input` | `rgba(237,237,236,0.16)` | |
| `--ring` | `#5b8a6f` | |
| `--sidebar` | `#0d0e0e` | |
| `--sidebar-primary` | `#5b8a6f` | **kills the violet at globals.css:115** |
| `--chart-1..5` | `#5b8a6f #7fae8f #8f938a #4a4e4b #c9c7bf` | spruce→stone ramp |
| `--radius` | `0.25rem` | near-square |

Light variant (`:root`): keep the stock light neutrals but warm them one step
(bg `#f2f1ec`, fg `#17191a`), primary/ring/sidebar-primary → spruce `#4f7d68`,
radius 0.25rem. Do not spend design effort here; dark is the product.

**Do NOT touch:** the hand-pinned 12px borderless-window corner radius + self-painted
window border/shadow in globals.css (load-bearing for Win/Linux chrome). `--radius`
change must not leak into it.

## 2. Terminal palette — bespoke "Svart" ANSI 16 (the signature)

Goes into `koden-default.ts` `variants.dark.terminal` (mirror `themes/nord.ts` shape —
currently BOTH variants are empty `{}`, which is the core defect). Theme **id stays
`koden-default`** (persisted contract); display name → **"Koden Svart"**, description
updated. High contrast on true black, restrained saturation, green family = spruce.

```
background  #0a0b0b   foreground #ededec
cursor      #5b8a6f   cursorAccent #0a0b0b
selection   rgba(91,138,111,0.28)   (leave selectionForeground unset)

black   #1a1c1b   brightBlack   #4a4e4b
red     #d97a72   brightRed     #e8938c
green   #7fae8f   brightGreen   #9ac4a8
yellow  #cfa964   brightYellow  #e0bd7e
blue    #7d9fc7   brightBlue    #99b5d6
magenta #ab8fb5   brightMagenta #c0a8c9
cyan    #82b8b4   brightCyan    #9cccc8
white   #d6d4cc   brightWhite   #f0efe9
```

Also give `variants.dark.colors` the §1 values so the theme is self-describing
(a `.koden-theme` export of the default must reproduce the look), and a light
`colors`+`terminal` mirroring §1's light port (ANSI: same hues, darkened for
light bg legibility — implementer tunes L until WCAG-ish contrast ≥ 4.5 on `#f2f1ec`).

## 3. Typography

- Add `@fontsource/commit-mono` (v5.2.5 confirmed on npm). Import in **BOTH** entry
  points: `src/main.tsx` AND `src/settings/main.tsx` (two webviews — miss one and it
  FOUTs; audit constraint).
- `src/lib/fonts.ts`: put `"Commit Mono"` at the head of `FALLBACK_CHAIN` (before
  JetBrains Mono). Keep the Nerd-Font auto-detect machinery, but the DEFAULT resolved
  family becomes a comma chain so glyph fallback still works:
  `"Commit Mono", <detected nerd font if any>, "JetBrains Mono", monospace`.
  **Constraint:** `terminalFontFamily` default stays `""` (empty string is the
  auto-detect signal — store.ts:199-205 area; do not change the default value, change
  what auto-detect resolves to). `ensureMonoFontsLoaded()` must preload Commit Mono.
- globals.css `@theme` block: define `--font-mono: "Commit Mono", "JetBrains Mono",
  ui-monospace, monospace;` — this alone fixes the 29 `font-mono` chrome files
  currently falling back to OS monospace.
- Fix `BrainMapPane.tsx:481,598,902`: unbundled `Manrope` / `IBM Plex Mono` → bundled
  Inter / the new mono chain.
- Keep JetBrains Mono bundled as fallback. UI prose stays Inter.

## 4. Mono chrome (the load-bearing identity move)

Structural chrome renders in `font-mono`; prose (chat messages, markdown, docs
editor) stays Inter. Surfaces: TabBar tab titles, StatusBar (all segments), settings
section headers/labels, `kbd.tsx`, command palette items + input, GridDialog command
tiles, pane titles. Sizing: mono chrome at 12–13px, `letter-spacing: 0.01em`.

`kbd`: mono 11px, bg `#0f1010`, hairline border, fg `#b9bdb9`, radius 2px. No glow
(ponytail: flat first; a spruce text-shadow is a one-line addition later if it reads
too dead).

Active tab: 1px spruce underline + full-fg text (inactive = muted-fg, no underline).
Focused terminal pane (incl. the single unsplit pane — closes a real gap found in the
audit): 1px spruce top hairline on the pane container. Border/hairline audit: remove
inner soft shadows on surfaces; keep the outer window drop shadow.

## 5. Wordmark

Mono lowercase `koden` followed by a spruce block cursor `▊` with a slow CSS blink
(~1.2s steps(1)). Used in: About panel header, main empty state. Nowhere else. No
logo redraw this sweep — the cyan K icon stays; re-tint to spruce is a follow-up
(flagged, not done).

## 6. Full-sweep decisions (delegated, decided here)

- **Naming rule:** "Koden Brain" = the context/memory engine ONLY (brain module,
  Librarian, Brain Map). Assistant/chat/model surfaces = "AI" (settings tab renamed
  "Koden AI" → "AI"; internal id `agents` stays — SettingsTab union is a contract).
  AiMiniWindow header says "Koden". Fix the stale StatusBar.tsx:64 comment.
- **Settings restructure** (audit list, all in scope):
  1. Extract Terminal (9 settings) from GeneralSection into its own top-level tab.
  2. Move Appearance mode control (GeneralSection.tsx:183-207) into ThemesSection.
  3. Promote the Brain/Librarian block (AgentsSection.tsx:589-956) to its own "Brain" tab.
  4. Custom-instructions field → save-on-blur like every other field.
  5. Derive the key→PrefKey map + legacy tab redirects from single sources
     (store.ts:787-848, SettingsApp.tsx:76-96).
  6. Curated font dropdown (detected Nerd Fonts + bundled) alongside the free-text
     input, + expose terminal line-height (xterm supports it; nothing sets it).
- **Quick wins** (all in scope): FOUC fix — boot scripts in index.html:9-23 +
  settings.html:9-23 paint the ACTIVE theme's real bg (tiny embedded id→bg map keyed
  off `koden-ui-theme-id-shadow`, which ThemeProvider already writes) instead of
  hardcoded `#0a0a0a`/`#ffffff`; new dark fallback literal = `#0a0b0b`.
  Consolidate the two `DEFAULT_THEME_ID` declarations (theme/types.ts:67,
  settings/store.ts:24) into one export. Fix components.json css path →
  `src/styles/globals.css`. Delete dead repo-root `koden-icon.png` (old Terax
  Mobius-'A' art, unreferenced). Theme-ify the four hardcoded highlight colors
  (blockDecorations.ts:17-18 OK/FAIL rulers, block.css:118-123 + 
  TerminalHistoryPopover.tsx:64-70 ambers — unify the two, globals.css:274-277 copy
  badge) via the resolve-token-to-hex pattern from useTerminalSession.ts:717 (xterm
  decorations structurally need literal hex — resolve at apply-time, don't inline
  CSS vars). Make GitHistoryPane.tsx:128-135 avatar palette + BrainMapPane colors
  mode/theme-aware. Adopt `ui/empty.tsx` for the hand-rolled empty states (AgentDock
  ~410, NotificationBell ~219, GitHistoryPane).
- **Explicitly out of scope:** Whisper refactor (own project, D7 already settled),
  bundle-id/store-key changes (rename fully done June 20; only deliberate legacy
  markers in agent.rs remain — NEVER touch `OWNED_MARKERS`), logo redraw, Rust-side
  anything (the brain session owns src-tauri), pushing anywhere.

## 7. Hard constraints (from audit — violating any is a defect)

1. `AiComposerProvider` stays unconditionally mounted at App.tsx root (conditional
   mount re-spawns every PTY).
2. Theme id `koden-default`, store filenames `koden-*.json`, localStorage shadow
   keys: **frozen contracts.**
3. `koden-ui-theme-shadow` literal is 3-way hand-synced (index.html, settings.html,
   ThemeProvider.tsx:51) — change all three together or none.
4. xterm `overviewRulerOptions`/Search decoration colors must be literal hex —
   resolve tokens at apply-time.
5. Font imports go in BOTH webview entry points.
6. `terminalFontFamily` `""` default semantic stays.
7. applyTheme's `ALL_VARS` allow-list is the reachable theme surface — new themable
   vars must be added there AND in types + validateTheme COLOR_KEYS together.
8. Legacy `terax` strings in `agent.rs` (OWNED_MARKERS migration) are load-bearing.

## Verification gates (per phase, CLI only — GUI run is Kosta's morning step)

`pnpm exec tsc --noEmit` clean; `pnpm vitest run` (known pre-existing failures:
eager-budget.test.ts env; Rust symlink test — not ours); `pnpm build` green at the
end. Commit per phase on `feat/koden-svart`. NEVER push.
