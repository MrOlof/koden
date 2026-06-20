---
title: Koden Update Channel — Setup Checklist
created: 2026-06-20
status: STAGED (code repointed) — keypair + repo + test release still owned by Kosta
design: .memory/koden-overhaul-plan-2026-06-20.md §3 (this is the do-it steps)
---

# Koden Update Channel — Setup Checklist

The signed, opt-in (“soft”) auto-update channel for Koden. The full design +
rationale lives in `.memory/koden-overhaul-plan-2026-06-20.md` §3B. This file is
the actionable checklist + the state staged in the working tree.

---

## 1. Already staged tonight (code side, uncommitted)

- **Updater endpoints repointed** from upstream `crynta/terax-ai` → `MrOlof/koden`:
  - `src-tauri/tauri.conf.json` → `plugins.updater.endpoints[0]` =
    `https://github.com/MrOlof/koden/releases/latest/download/latest.json`
  - `src/modules/updater/useUpdater.ts` → `GITHUB_LATEST_RELEASE` =
    `https://api.github.com/repos/MrOlof/koden/releases/latest` (Linux manual-check path)
  - A `MrOlof/koden` repo with no releases yet just 404s → the updater finds
    nothing → harmless. No more risk of being offered upstream builds.
- **`autoUpdateCheck` preference added (default OFF).** Mirrors the
  `commandMinimapEnabled` pattern in `src/modules/settings/store.ts`
  (Preferences type, `DEFAULT_PREFERENCES`, loader, `onPreferencesChange` map,
  `setAutoUpdateCheck()` setter via the existing `writePref()` path).
- **Hardcoded `AUTO_UPDATE_DISABLED` flag removed** from `useUpdater.ts`. The
  auto-check effect now runs only when the caller opts in (`autoCheck`) **AND**
  `autoUpdateCheck` is true. The manual About-panel check (`autoCheck:false`)
  works regardless of the pref. The 30-min throttle (`CHECK_INTERVAL_MS` +
  `terax:updater:last-check`) is unchanged.
- **Settings toggle added** in `src/settings/sections/AboutSection.tsx`
  (a `SettingRow` + `Switch`): “Automatically check for updates” / helper
  “Off until a Koden release feed is configured.” Wired to
  `autoUpdateCheck` / `setAutoUpdateCheck`.
- **Release CI already Koden-named (Phase 2 done earlier):**
  `.github/workflows/release.yml` already builds installers, signs them, and
  generates `latest.json` (`createUpdaterArtifacts: true`); `releaseName: "Koden …"`,
  AppImage asserts `usr/bin/koden`, the `patch-appimage-updater` job re-signs the
  wayland-stripped AppImage. It already reads the `TAURI_SIGNING_PRIVATE_KEY*`
  secrets and uses `${{ github.repository }}` for uploads (repo-agnostic).

**Still pointing at crynta (deliberately deferred — your call):**
- `tauri.conf.json` `plugins.updater.pubkey` — still crynta’s minisign key
  (`3BABFD8AB60E3469`). JSON is strict, so no comment marks it; this file does.
  Replace it in step 2.4.
- Bundle identifier `app.crynta.terax` — **untouched** (see §5).

---

## 2. Your remaining steps (exact commands)

### 2.1 Mint the Koden minisign keypair
```bash
# from repo root; prompts for a password — save it in your password manager
pnpm tauri signer generate -w ~/.koden-updater.key
# (or: tauri signer generate -w ~/.koden-updater.key)
```
This prints/saves two halves: a PRIVATE key (file contents) and a PUBLIC key (base64).

### 2.2 Add the PRIVATE key + password as GitHub repo secrets
In `MrOlof/koden` → Settings → Secrets and variables → Actions:
- `TAURI_SIGNING_PRIVATE_KEY` = the private key **file contents**
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = the password you set
`release.yml` already references both — you only supply the values.
**Never commit the private key or paste it anywhere.**

### 2.3 Create the `MrOlof/koden` GitHub repo
Push this fork there. (Keep the upstream `crynta/terax-ai` Apache-2.0 attribution.)

### 2.4 Paste the PUBLIC key into `tauri.conf.json`
Replace `plugins.updater.pubkey` (currently crynta’s `dW50cnVzdGVk…3QkFCRkQ4…`)
with the base64 PUBLIC key from step 2.1. This is the only key change needed —
the endpoint is already repointed.

### 2.5 Confirm the release workflow
`release.yml` should already: build all installers, sign them with the new
secrets, generate `latest.json`, and (Linux) patch the re-signed AppImage
signature in. Skim it once after the secrets are in.

### 2.6 Cut a test release
```bash
git tag v0.9.0
git push origin v0.9.0
```
The tag triggers `release.yml` (`on: push tags: v*`, `releaseDraft: true`).
Publish the draft, then confirm the release has the platform installers + a
`latest.json` asset whose `pubkey`-signed `signature` fields are populated.

### 2.7 Flip the toggle on
In Koden → Settings → About → **“Automatically check for updates”** → ON.
With a current version below `v0.9.0`, startup should now offer the update.
(Throttled to one check / 30 min.)

---

## 3. `latest.json` schema the Tauri updater expects
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
- Platform keys are `{os}-{arch}`. `version` is clean semver — compared against
  `getVersion()`; the Linux manual path mirrors that in `useUpdater.ts isNewer()`.
- `signature` = the minisign signature produced by the private key (step 2.1)
  over each artifact. Mismatched/empty signatures = the updater rejects the update.

---

## 4. Optional: stable / beta channels (defer)
Two static manifests rather than a query param (GitHub Releases serves static files):
- `latest.json` = stable; `beta.json` = prereleases.
- Add a `channel` pref that swaps the endpoint string before `check()`.
Keep it simple until the stable feed is proven; one manifest is enough to start.

---

## 5. Bundle-id note (deferred — cross-reference the overhaul plan)
Updates install **cleanly only after the bundle identifier is changed** from
`app.crynta.terax` → `app.mrolof.koden` (D1/D5 in
`.memory/koden-overhaul-plan-2026-06-20.md`). The identifier derives the
appdata/config dir, keyring identity, single-instance lock, and updater install
identity; changing it orphans existing installs (accept a one-time reset for
this fresh fork). It is set in `src-tauri/tauri.conf.json` (`identifier`) and
hand-synced in the literal at `src/settings/sections/AboutSection.tsx`. Left
**unchanged tonight** on purpose — change it as part of the Phase 1 identity pass,
not here.
