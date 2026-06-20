// =============================================================================
// WebdriverIO config for the Koden autonomous test harness.
//
// REQUIRES the Phase-0 provider spike before it can connect: the design picks
// the *embedded* `tauri-plugin-webdriver` (W3C endpoints served inside the app
// behind a `webdriver` Cargo feature), NOT the pre-alpha official tauri-driver.
// Until that plugin is wired and the dev app is launched via
// `node scripts/launch-sandbox.mjs`, `pnpm test:e2e` will fail to create a
// session — by design (it must never silently pass green against nothing).
//
// Host/port come from the embedded plugin; override via env once known.
// See tests/e2e/README.md.
// =============================================================================

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./tests/e2e/**/*.e2e.ts"],
  exclude: [],
  maxInstances: 1,

  hostname: process.env.KODEN_WD_HOST ?? "127.0.0.1",
  port: Number(process.env.KODEN_WD_PORT ?? 4444),
  path: process.env.KODEN_WD_PATH ?? "/",

  capabilities: [
    // TODO(phase-0): real capability for the embedded tauri-plugin-webdriver.
    // For the official tauri-driver path instead, run `tauri-driver` as a proxy
    // and use a `tauri:options` capability pointing at the built app binary.
    {
      browserName: "wry",
    } as WebdriverIO.Capabilities,
  ],

  logLevel: "info",
  bail: 0,
  waitforTimeout: 30_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 3,

  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    timeout: 120_000,
  },
};
