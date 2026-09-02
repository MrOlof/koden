import { describe, expect, it } from "vitest";
import { paneEventStep, paneKeyMap, parsePaneEventLine } from "./paneEvents";

describe("parsePaneEventLine", () => {
  it("accepts the hook contract line", () => {
    expect(
      parsePaneEventLine(
        '{"pane":"%51","sessionId":"1a88-64","event":"user-prompt","ts":1788265076}',
      ),
    ).toEqual({
      pane: "%51",
      sessionId: "1a88-64",
      event: "user-prompt",
      ts: 1788265076,
    });
  });

  it("rejects junk, torn lines, bad panes and unknown kinds", () => {
    expect(parsePaneEventLine("")).toBeNull();
    expect(parsePaneEventLine('{"pane":"%1","sessionId":"s","ev')).toBeNull();
    expect(parsePaneEventLine("[1,2]")).toBeNull();
    expect(parsePaneEventLine('"just a string"')).toBeNull();
    expect(
      parsePaneEventLine(
        '{"pane":"51","sessionId":"s","event":"stop","ts":1}',
      ),
    ).toBeNull();
    expect(
      parsePaneEventLine(
        '{"pane":"%x","sessionId":"s","event":"stop","ts":1}',
      ),
    ).toBeNull();
    expect(
      parsePaneEventLine(
        '{"pane":"%1","sessionId":"s","event":"self-destruct","ts":1}',
      ),
    ).toBeNull();
    expect(
      parsePaneEventLine(
        '{"pane":"%1","sessionId":"s","event":"stop","ts":"now"}',
      ),
    ).toBeNull();
    expect(
      parsePaneEventLine(
        `{"pane":"%1","sessionId":"${"s".repeat(200)}","event":"stop","ts":1}`,
      ),
    ).toBeNull();
  });
});

describe("paneEventStep", () => {
  it("prompt starts a working turn, stop finishes it green", () => {
    expect(paneEventStep("user-prompt", false)).toEqual({
      tab: "working",
      midTurn: true,
    });
    expect(paneEventStep("stop", true)).toEqual({ tab: "done", midTurn: false });
  });

  it("notification is orange only mid-turn", () => {
    expect(paneEventStep("notification", true)).toEqual({
      tab: "waiting",
      midTurn: true,
    });
    expect(paneEventStep("notification", false)).toEqual({
      tab: null,
      midTurn: false,
    });
  });

  it("session-start resets the turn without a pill", () => {
    expect(paneEventStep("session-start", true)).toEqual({
      tab: null,
      midTurn: false,
    });
  });
});

describe("paneKeyMap", () => {
  it("maps panes of koden windows and skips foreign or paneless ones", () => {
    const map = paneKeyMap([
      { name: "w-rk-a", command: "claude", path: "/x", pane: "%3" },
      { name: "manual", command: "htop", path: "/y", pane: "%4" },
      { name: "w-rk-b", command: "bash", path: "/z", pane: "" },
    ]);
    expect(map).toEqual(new Map([["%3", "rk-a"]]));
  });
});
