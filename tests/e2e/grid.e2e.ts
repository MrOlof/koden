import { waitForBus } from "./_helpers";

// MVP scenario (b): grids have NO command/keybinding — the bus is the only
// deterministic path. Create a 2x2 grid and assert four panes exist.
describe("grids: multi-pane creation", () => {
  before(waitForBus);

  it("creates a 2x2 grid tab with four terminal leaves", async () => {
    const result = await browser.execute(() => window.__KODEN_TEST__.newGridTab(2, 2));
    expect(result.leafIds.length).toBe(4);

    await browser.waitUntil(
      async () => (await browser.execute(() => window.__KODEN_TEST__.tabsSnapshot().paneCount)) === 4,
      { timeoutMsg: "grid did not settle to 4 panes" },
    );

    const snap = await browser.execute(() => window.__KODEN_TEST__.tabsSnapshot());
    expect(snap.activeKind).toBe("terminal");
    expect(snap.activeTitle).toContain("Grid");
  });
});
