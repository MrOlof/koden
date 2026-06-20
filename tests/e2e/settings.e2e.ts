import { waitForBus } from "./_helpers";

// MVP scenario (c): the separate settings webview is bypassed by calling the
// real store setters in-process. setThemeId writes koden-settings.json AND emits
// koden://prefs-changed, which the main window's preferences store consumes.
describe("settings: store-setter bypass", () => {
  before(waitForBus);

  it("sets themeId via the bus and the preferences store reflects it", async () => {
    await browser.execute(() => window.__KODEN_TEST__.settings.setThemeId("koden-default"));

    await browser.waitUntil(
      async () =>
        (await browser.execute(
          () =>
            (window.__KODEN_TEST__.getStores().preferences as { themeId?: string }).themeId ===
            "koden-default",
        )) === true,
      { timeoutMsg: "preferences.themeId never reflected the setter" },
    );
  });
});
