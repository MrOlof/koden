# Day report — onboarding, icon, installer, cleanup (2026-07-11 afternoon)

**TLDR: all four asks done and verified. New icon (your hero K) across the
whole app, a branded signed installer builds clean
(`Koden_0.8.0_x64-setup.exe` + `.sig`), the onboarding wizard now tells the
truth about how Koden works, the app teaches its own shortcuts, and ~5 MB of
legacy files are gone. Gates green everywhere.**

## Icon (`10d5e70`)

`docs/koden-hero.png` (the beveled cyan K you pointed at) is now THE icon —
full set regenerated (`ico`/`icns`/all PNG sizes/Windows Square/Android) plus
`public/logo.png`. Source kept at `docs/koden-icon-1024.png`. Reads clean at
taskbar size. The old flat K and the ancient Terax Mobius-'A' are both gone.

## Installer (`cba28f6` + build proof)

- NSIS wizard now shows **Svart-branded header + sidebar art** (24bpp BMPs
  generated from the K mark + `koden▊` wordmark) and an Apache-2.0 license
  page. MSI dropped from targets (it never had the "Open in Koden" Explorer
  verbs; one artifact, less confusion).
- `release.yml`: macOS signing steps now gate on the Apple secrets actually
  existing (they don't — legs previously exported empty creds); release body
  no longer claims auto-update is "built in".
- UpdaterDialog: Debian is the default distro tab; Arch marked "AUR (coming
  soon)" (package doesn't exist yet).
- **Proof:** local signed build succeeded end-to-end — installer + updater
  `.sig` via your `~/.koden-updater.key`. Artifact at
  `src-tauri/target/release/bundle/nsis/Koden_0.8.0_x64-setup.exe` — run it
  when you're back if you want to see the branded wizard.
- **Yours alone (I never push):** the repo is 170+ commits ahead of
  origin/main and PRIVATE — publish + tag `v0.8.0` when ready; the updater
  endpoint 404s until then.

## Onboarding wizard (`cb09d96`, smoke-proven)

Audit found the wizard predated the Librarian pivot and dead-ended keyless
users. Now: Welcome/"How Koden works" tell the real triad (**terminals + your
agents feed the Brain via auto-installed hooks → the Brain indexes and
remembers → the Librarian is the chat**); a keyless user on a local provider
advances cleanly instead of hitting a hard error; Skip jumps to the summary
(pointers visible) instead of vanishing forever; re-running the guide resets
state and shows the configured workspace; a first-boot status error now fails
OPEN (retry once, then show the wizard); the Done step teaches `Ctrl+I`
(Librarian), `Ctrl+P` (palette), and "run claude/codex in a terminal".
Screenshots: `.memory/svart-verification/wizard-*.png`.

## In-app guidance (`aadb7cf`)

- The "+" menu no longer teaches a WRONG shortcut for Preview (said Mod+P —
  that's the palette; real binding Mod+Shift+O).
- Icon-button tooltips now carry their bindings (palette, Librarian, sidebar,
  new tab) — the palette is discoverable without luck.
- Palette gains **Setup guide** and **About Koden** entries; typing "help"
  finds things now. `?` mode shows per-mode examples + a Keyboard-shortcuts
  row.
- Usage-guard pauses get a persistent amber status-bar pill (was: one
  dismissable toast, then silent mystery).
- Tab status colors now derive from the agent-dock palette (two drifting
  palettes unified) + a one-line dot legend in the dock's empty state.
- "Ask Librarian about selection" lives in the pane right-click menu with its
  Mod+J label. Board columns get an empty-state line.

## Cleanup (`cba28f6`)

Deleted (all verified zero-reference, knip-confirmed): `AgentSwitcher.tsx`
(the ADR-017 leftover) + 7 orphaned docs assets (~5 MB: old terax-ai README
screenshots + the superseded icon source). Brain build-history docs moved
`docs/` → `.memory/brain-build/` with every cross-reference updated. Kept
deliberately: `social-banner.png` (GitHub social preview), community files +
nix + signpath (publish-ready story), scripts/ (all wired — wdio/knip use
them).

## Verification

Three parallel implementation lanes → max-effort adversarial verifier:
**clean, zero blocking**, tsc 0 / vitest 398/398 / build green; 8 minors — 3
fixed on the spot (over-promising wizard button label, 32bpp→24bpp BMPs,
stale future-paths in the brain BUILD-PROMPT plan), 5 accepted and documented
(tooltip labels don't track rebinds — pre-existing convention; ask-row in
splits degrades gracefully; AUR command visible under "coming soon";
Done/Ready label split between tab and dock — colors unified; Apple gate
keys off the cert secret only). Wizard smoke-tested live via CDP through the
new palette entry. Knip follow-up for later: 4 unused devDeps + 115 unused
barrel exports (dead exports, not dead files).
