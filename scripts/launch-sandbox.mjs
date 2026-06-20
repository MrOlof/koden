#!/usr/bin/env node
// =============================================================================
// Koden sandbox launcher — boot `pnpm tauri dev` against a DISPOSABLE profile so
// the autonomous test harness never touches the real workspace, settings, keys,
// or background images. Zero dependencies (Node ESM).
//
//   node scripts/launch-sandbox.mjs            # boot the sandboxed dev app
//   node scripts/launch-sandbox.mjs --usage    # + fake usage endpoint wired in
//   node scripts/launch-sandbox.mjs --teardown # wipe the scratch profile + exit
//   node scripts/launch-sandbox.mjs --keep     # don't wipe scratch on exit
//
// ISOLATION STATUS (be honest — see tests/e2e/README.md):
//   * WebView2 user-data (localStorage `koden-palette-mru`/`koden.*`, IndexedDB
//     `koden-bg-images`)  -> ISOLATED via WEBVIEW2_USER_DATA_FOLDER (honored).
//   * Provider API keys (OS keychain)                              -> ISOLATED
//     via VITE_KEYRING_SERVICE=koden-sandbox (see src/modules/ai/config.ts).
//   * HOME/USERPROFILE (shell cwd + OS-home fallbacks)             -> redirected.
//   * Tauri plugin-store files (`koden-*.json` in %APPDATA%\<bundle-id>) are NOT
//     redirected by the APPDATA env var on Windows — the `dirs` crate reads the
//     known-folder path, ignoring env. TRUE appdata isolation needs a sandbox
//     bundle identifier; see the TODO at the bottom. Until then these LazyStores
//     write to the real per-OS-user appdata; teardown clears the scratch dirs
//     but cannot reach those without the identifier override.
// =============================================================================

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const argv = new Set(process.argv.slice(2));
const ROOT = path.resolve(import.meta.dirname, "..");
// Stable dir so WebdriverIO and --teardown target the same profile each run.
const SANDBOX = path.join(os.tmpdir(), "koden-sandbox");
const DIRS = {
  home: path.join(SANDBOX, "home"),
  appdata: path.join(SANDBOX, "appdata"),
  localappdata: path.join(SANDBOX, "localappdata"),
  webview2: path.join(SANDBOX, "webview2"),
  workspace: path.join(SANDBOX, "workspace"),
};

function wipe() {
  fs.rmSync(SANDBOX, { recursive: true, force: true });
}

function seed() {
  wipe();
  for (const dir of Object.values(DIRS)) fs.mkdirSync(dir, { recursive: true });
  // A pre-seeded workspace so the folder picker is never invoked.
  fs.writeFileSync(
    path.join(DIRS.workspace, "README.md"),
    "# koden sandbox workspace\nDisposable. Wiped by launch-sandbox.mjs.\n",
  );
}

if (argv.has("--teardown")) {
  wipe();
  console.log(`[sandbox] wiped ${SANDBOX}`);
  console.log(
    "[sandbox] NOTE: OS keychain entries under service 'koden-sandbox' and the\n" +
      "          real-appdata koden-*.json are NOT removed here — delete keychain\n" +
      "          creds with `cmdkey /list` + `cmdkey /delete`, or via the app's\n" +
      "          secrets delete, and see the bundle-identifier TODO for appdata.",
  );
  process.exit(0);
}

seed();

const env = {
  ...process.env,
  HOME: DIRS.home,
  USERPROFILE: DIRS.home,
  APPDATA: DIRS.appdata,
  LOCALAPPDATA: DIRS.localappdata,
  // Honored by WebView2 — redirects localStorage + IndexedDB to scratch.
  WEBVIEW2_USER_DATA_FOLDER: DIRS.webview2,
  // Read by src/modules/ai/config.ts -> disposable keychain service.
  VITE_KEYRING_SERVICE: "koden-sandbox",
};
if (argv.has("--usage")) {
  env.KODEN_USAGE_ENDPOINT =
    process.env.KODEN_USAGE_ENDPOINT ?? "http://127.0.0.1:8473";
  console.log(
    `[sandbox] usage guard pointed at ${env.KODEN_USAGE_ENDPOINT} ` +
      "(start scripts/fake-usage-endpoint.mjs separately)",
  );
}

console.log(`[sandbox] profile: ${SANDBOX}`);
console.log(`[sandbox] workspace cwd: ${DIRS.workspace}`);
console.log("[sandbox] launching: pnpm tauri dev");

const child = spawn("pnpm", ["tauri", "dev"], {
  cwd: ROOT,
  env,
  stdio: "inherit",
  shell: process.platform === "win32", // resolve pnpm.cmd on Windows
});

function shutdown() {
  if (!child.killed) child.kill();
  if (!argv.has("--keep")) {
    wipe();
    console.log(`\n[sandbox] wiped ${SANDBOX}`);
  }
}

process.on("SIGINT", () => {
  shutdown();
  process.exit(0);
});
child.on("exit", (code) => {
  shutdown();
  process.exit(code ?? 0);
});

// TODO(full-isolation): redirect the Tauri appDataDir too. The cleanest path is
// a sandbox bundle identifier so `koden-*.json` land in a scratch appdata folder.
// Pass `--` args through to tauri to override the identifier, e.g.:
//   spawn("pnpm", ["tauri", "dev", "--", "--config", sandboxConfPath], ...)
// where sandboxConfPath sets `identifier` to `app.koden.sandbox`. Verify the dev
// build honors the override before relying on it.
