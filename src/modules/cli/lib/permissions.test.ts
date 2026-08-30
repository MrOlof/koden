import { describe, expect, it } from "vitest";
import {
  CLI_COMMANDS,
  type CliPrefs,
  checkPermission,
  gateFor,
} from "./permissions";

const ALL_ON: CliPrefs = {
  cliEnabled: true,
  cliTerminalRead: true,
  cliTerminalInput: true,
  cliPanelControl: true,
  cliNotify: true,
};

describe("checkPermission", () => {
  it("allows every known command when everything is on", () => {
    for (const cmd of CLI_COMMANDS) {
      expect(checkPermission(cmd, ALL_ON)).toBeNull();
    }
  });

  it("master switch denies everything with the Settings hint", () => {
    const off = { ...ALL_ON, cliEnabled: false };
    for (const cmd of CLI_COMMANDS) {
      expect(checkPermission(cmd, off)).toBe(
        "the koden CLI is disabled in Settings > CLI",
      );
    }
  });

  it("maps each surface x access cell to its pref", () => {
    const noRead = { ...ALL_ON, cliTerminalRead: false };
    expect(checkPermission("terminal.list", noRead)).toBe(
      "Terminal read is disabled in Settings > CLI",
    );
    expect(checkPermission("terminal.read", noRead)).toContain("Terminal read");
    expect(checkPermission("terminal.type", noRead)).toBeNull();

    const noInput = { ...ALL_ON, cliTerminalInput: false };
    for (const cmd of ["terminal.type", "terminal.press", "terminal.run"]) {
      expect(checkPermission(cmd, noInput)).toBe(
        "Terminal control is disabled in Settings > CLI",
      );
    }
    expect(checkPermission("terminal.read", noInput)).toBeNull();

    const noPanel = { ...ALL_ON, cliPanelControl: false };
    for (const cmd of ["tab.open", "pane.split", "space.new"]) {
      expect(checkPermission(cmd, noPanel)).toBe(
        "Panel control is disabled in Settings > CLI",
      );
    }
    // List-only reads have no pref: always allowed while the CLI is on.
    expect(checkPermission("space.list", noPanel)).toBeNull();
    expect(checkPermission("ping", noPanel)).toBeNull();

    expect(checkPermission("notify", { ...ALL_ON, cliNotify: false })).toBe(
      "Notify control is disabled in Settings > CLI",
    );
  });

  it("refuses unknown commands even with everything on", () => {
    expect(checkPermission("agent.create", ALL_ON)).toBe(
      "unknown command 'agent.create'",
    );
    expect(gateFor("agent.create")).toBeNull();
  });
});
