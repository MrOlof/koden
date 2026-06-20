// Shared helpers for the Koden harness specs. Everything goes through the
// in-app bus (window.__KODEN_TEST__); we never scrape the WebGL terminal canvas.
// Reads are always polled (waitUntil) — NEVER fixed sleeps.

/** Resolve once the dev test bus is installed in the page. */
export async function waitForBus(): Promise<void> {
  await browser.waitUntil(
    async () => (await browser.execute(() => !!window.__KODEN_TEST__?.ready())) === true,
    { timeout: 30_000, timeoutMsg: "__KODEN_TEST__ bus never installed (DEV build + harness mount required)" },
  );
}

/** Run a registered command by id, opening the palette so its items exist. */
export async function runCommand(id: string): Promise<void> {
  await browser.execute(() => window.__KODEN_TEST__.openPalette(true));
  await browser.waitUntil(
    async () => (await browser.execute(() => window.__KODEN_TEST__.commandCount() > 0)) === true,
    { timeoutMsg: "palette items never populated after openPalette(true)" },
  );
  await browser.execute((cmd) => window.__KODEN_TEST__.runCommandById(cmd as string), id);
  await browser.execute(() => window.__KODEN_TEST__.openPalette(false));
}

/** The current active terminal leaf id, or null. */
export async function activeLeafId(): Promise<number | null> {
  return browser.execute(() => window.__KODEN_TEST__.tabsSnapshot().activeLeafId);
}

/** Submit a command into a leaf once its session is ready, then wait for `expect` in the buffer. */
export async function submitAndExpect(leafId: number, command: string, expectText: string): Promise<void> {
  // Buffer is non-null once the renderer slot exists (=== "" is also ready).
  await browser.waitUntil(
    async () =>
      (await browser.execute((id) => window.__KODEN_TEST__.getBuffer(id as number) !== null, leafId)) === true,
    { timeoutMsg: `leaf ${leafId} renderer never came up` },
  );
  await browser.execute(
    (id, cmd) => window.__KODEN_TEST__.submitToLeaf(id as number, cmd as string),
    leafId,
    command,
  );
  await browser.waitUntil(
    async () =>
      (await browser.execute(
        (id, txt) => (window.__KODEN_TEST__.getBuffer(id as number) ?? "").includes(txt as string),
        leafId,
        expectText,
      )) === true,
    { timeout: 30_000, timeoutMsg: `"${expectText}" never appeared in leaf ${leafId}` },
  );
}
