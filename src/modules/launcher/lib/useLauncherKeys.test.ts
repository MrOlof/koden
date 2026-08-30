import { describe, expect, it } from "vitest";
import {
  isLauncherNavKey,
  locateStop,
  stepIndex,
  type StopNode,
} from "./useLauncherKeys";

describe("stepIndex", () => {
  it("walks down and up with wrap-around", () => {
    expect(stepIndex(0, 4, "ArrowDown")).toBe(1);
    expect(stepIndex(3, 4, "ArrowDown")).toBe(0);
    expect(stepIndex(0, 4, "ArrowUp")).toBe(3);
    expect(stepIndex(2, 4, "ArrowUp")).toBe(1);
  });

  it("jumps to the ends", () => {
    expect(stepIndex(2, 4, "Home")).toBe(0);
    expect(stepIndex(2, 4, "End")).toBe(3);
  });

  it("resolves a between-stops position to the neighbouring stop", () => {
    expect(stepIndex(1.5, 4, "ArrowDown")).toBe(2);
    expect(stepIndex(1.5, 4, "ArrowUp")).toBe(1);
    expect(stepIndex(-0.5, 4, "ArrowDown")).toBe(0);
    expect(stepIndex(-0.5, 4, "ArrowUp")).toBe(3);
    expect(stepIndex(3.5, 4, "ArrowDown")).toBe(0);
  });

  it("returns -1 with no stops", () => {
    expect(stepIndex(0, 0, "ArrowDown")).toBe(-1);
  });
});

describe("isLauncherNavKey", () => {
  it("accepts only the four navigation keys", () => {
    expect(isLauncherNavKey("ArrowDown")).toBe(true);
    expect(isLauncherNavKey("End")).toBe(true);
    expect(isLauncherNavKey("Enter")).toBe(false);
    expect(isLauncherNavKey("ArrowLeft")).toBe(false);
  });
});

// A flat document: each node knows its position; `contains` only matches
// itself or a declared child (the input inside the connect form).
type Fake = StopNode & { order: number; children: Fake[] };
function node(order: number, children: Fake[] = []): Fake {
  const self: Fake = {
    order,
    children,
    contains: (other) =>
      other === self || children.some((c) => c.contains(other)),
    compareDocumentPosition: (other) => ((other as Fake).order > order ? 4 : 2),
  };
  return self;
}

describe("locateStop (start page order)", () => {
  // START rows, then the expanded connect form (not a stop), then RECENT rows.
  const hostInput = node(3.1);
  const form = node(3, [hostInput]);
  const start = [node(0), node(1), node(2)];
  const recent = [node(4), node(5)];
  const stops = [...start, ...recent];

  it("finds a focused row by identity", () => {
    expect(locateStop(stops, start[1])).toBe(1);
    expect(locateStop(stops, recent[0])).toBe(3);
  });

  it("places the connect form between the last START and first RECENT row", () => {
    expect(locateStop(stops, form)).toBe(2.5);
    expect(locateStop(stops, hostInput)).toBe(2.5);
    const from = locateStop(stops, hostInput);
    expect(stops[stepIndex(from, stops.length, "ArrowDown")]).toBe(recent[0]);
    expect(stops[stepIndex(from, stops.length, "ArrowUp")]).toBe(start[2]);
  });

  it("treats focus before every row as -0.5 and after the last as n - 0.5", () => {
    expect(locateStop(stops, node(-1))).toBe(-0.5);
    expect(locateStop(stops, node(99))).toBe(4.5);
    expect(stops[stepIndex(4.5, stops.length, "ArrowDown")]).toBe(start[0]);
  });
});
