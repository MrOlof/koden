import { describe, expect, it } from "vitest";
import { isLauncherNavKey, stepIndex } from "./useLauncherKeys";

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
