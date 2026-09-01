import { describe, expect, it } from "vitest";
import { layoutContentChanged, type SpaceState } from "./store";

const state = (title: string, activeTabIndex = 0): SpaceState => ({
  tabs: [{ kind: "notes", docId: "d", title }],
  activeTabIndex,
});

describe("layoutContentChanged", () => {
  it("is true for a first write and for a real layout change", () => {
    expect(layoutContentChanged(undefined, state("a"))).toBe(true);
    expect(layoutContentChanged(state("a"), state("b"))).toBe(true);
  });

  it("ignores activeTabIndex so a tab switch or boot seed never re-stamps", () => {
    expect(layoutContentChanged(state("a", 0), state("a", 3))).toBe(false);
    expect(layoutContentChanged(state("a"), state("a"))).toBe(false);
  });
});
