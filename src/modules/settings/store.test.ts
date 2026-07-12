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
