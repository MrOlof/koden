import { describe, expect, it } from "vitest";
import { DEFAULT_PREFERENCES } from "./store";

// ADR-020: memory-activity notifications ship default-ON (the Librarian is
// autonomous by default, so its activity must be visible by default — the
// toggle in the Librarian settings tab is the opt-out).
describe("memoryNotifications preference", () => {
  it("defaults ON", () => {
    expect(DEFAULT_PREFERENCES.memoryNotifications).toBe(true);
  });

  it("rides the standard key-per-field persistence contract", () => {
    // The cross-window change fan-out (onPreferencesChange) maps store keys to
    // Preferences fields via the identity over DEFAULT_PREFERENCES — a field
    // present here is guaranteed to propagate to other windows.
    expect(Object.keys(DEFAULT_PREFERENCES)).toContain("memoryNotifications");
  });
});

// modules/cli: the koden CLI ships enabled with every gate open; Settings >
// CLI is the opt-out. Each key must ride the identity persistence contract so
// a flip in the settings window reaches the main window's bridge.
describe("cli preferences", () => {
  it("default ON across the whole matrix", () => {
    expect(DEFAULT_PREFERENCES.cliEnabled).toBe(true);
    expect(DEFAULT_PREFERENCES.cliTerminalRead).toBe(true);
    expect(DEFAULT_PREFERENCES.cliTerminalInput).toBe(true);
    expect(DEFAULT_PREFERENCES.cliPanelControl).toBe(true);
    expect(DEFAULT_PREFERENCES.cliNotify).toBe(true);
  });

  it("ride the standard key-per-field persistence contract", () => {
    for (const k of [
      "cliEnabled",
      "cliTerminalRead",
      "cliTerminalInput",
      "cliPanelControl",
      "cliNotify",
    ]) {
      expect(Object.keys(DEFAULT_PREFERENCES)).toContain(k);
    }
  });
});
