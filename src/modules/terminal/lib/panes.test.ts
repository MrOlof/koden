import { beforeEach, describe, expect, it } from "vitest";
import { usePaneTitleStore } from "./paneTitles";
import {
  findLeaf,
  hasLeaf,
  moveLeafBeside,
  type PaneNode,
  type SplitSide,
  sideToSplit,
  splitLeaf,
} from "./panes";

const leaf = (id: number): PaneNode => ({ kind: "leaf", id });

// Run a split of `side` against a single root leaf (id 1) and return the
// resulting top-level children ids in order, so we can assert which side the
// new leaf (id 3) landed on.
function splitRootIds(side: SplitSide): { dir: string; ids: number[] } {
  const { dir, before } = sideToSplit(side);
  const tree = splitLeaf(leaf(1), 1, 2, 3, dir, undefined, before);
  if (tree.kind !== "split") throw new Error("expected a split");
  return { dir, ids: tree.children.map((c) => c.id) };
}

describe("sideToSplit", () => {
  it("maps the 4 sides to axis + before", () => {
    expect(sideToSplit("right")).toEqual({ dir: "row", before: false });
    expect(sideToSplit("left")).toEqual({ dir: "row", before: true });
    expect(sideToSplit("bottom")).toEqual({ dir: "col", before: false });
    expect(sideToSplit("top")).toEqual({ dir: "col", before: true });
  });
});

describe("splitLeaf 4-direction placement (leaf promotion)", () => {
  it("Right: row, new leaf after the target", () => {
    expect(splitRootIds("right")).toEqual({ dir: "row", ids: [1, 3] });
  });
  it("Left: row, new leaf before the target", () => {
    expect(splitRootIds("left")).toEqual({ dir: "row", ids: [3, 1] });
  });
  it("Bottom: col, new leaf after the target", () => {
    expect(splitRootIds("bottom")).toEqual({ dir: "col", ids: [1, 3] });
  });
  it("Top: col, new leaf before the target", () => {
    expect(splitRootIds("top")).toEqual({ dir: "col", ids: [3, 1] });
  });
});

describe("splitLeaf 4-direction placement (same-direction merge)", () => {
  // A row split [10, 11]; splitting 11 along the row axis must merge into the
  // same split rather than nest, inserting on the correct side of 11.
  const rowSplit: PaneNode = {
    kind: "split",
    id: 5,
    dir: "row",
    children: [leaf(10), leaf(11)],
  };

  it("Right of 11 inserts the new leaf directly after 11", () => {
    const { dir, before } = sideToSplit("right");
    const out = splitLeaf(rowSplit, 11, 99, 12, dir, undefined, before);
    expect(out.kind).toBe("split");
    if (out.kind !== "split") return;
    expect(out.children.map((c) => c.id)).toEqual([10, 11, 12]);
  });

  it("Left of 11 inserts the new leaf directly before 11", () => {
    const { dir, before } = sideToSplit("left");
    const out = splitLeaf(rowSplit, 11, 99, 12, dir, undefined, before);
    expect(out.kind).toBe("split");
    if (out.kind !== "split") return;
    expect(out.children.map((c) => c.id)).toEqual([10, 12, 11]);
  });

  it("Bottom of 11 (col axis) promotes 11 into a nested col split below", () => {
    const { dir, before } = sideToSplit("bottom");
    const out = splitLeaf(rowSplit, 11, 99, 12, dir, undefined, before);
    expect(out.kind).toBe("split");
    if (out.kind !== "split") return;
    // The outer split stays a row; 11 becomes a col split [11, 12].
    expect(out.dir).toBe("row");
    const nested = out.children[1];
    expect(nested.kind).toBe("split");
    if (nested.kind !== "split") return;
    expect(nested.dir).toBe("col");
    expect(nested.children.map((c) => c.id)).toEqual([11, 12]);
  });

  it("Top of 11 (col axis) promotes 11 into a nested col split above", () => {
    const { dir, before } = sideToSplit("top");
    const out = splitLeaf(rowSplit, 11, 99, 12, dir, undefined, before);
    expect(out.kind).toBe("split");
    if (out.kind !== "split") return;
    const nested = out.children[1];
    expect(nested.kind).toBe("split");
    if (nested.kind !== "split") return;
    expect(nested.dir).toBe("col");
    expect(nested.children.map((c) => c.id)).toEqual([12, 11]);
  });
});

describe("moveLeafBeside", () => {
  const row = (...ids: number[]): PaneNode => ({
    kind: "split",
    id: 5,
    dir: "row",
    children: ids.map(leaf),
  });

  it("moves leaf 11 to the left of 10 in a [10,11] row -> [11,10]", () => {
    const out = moveLeafBeside(row(10, 11), 11, 10, 99, "left");
    expect(out.kind).toBe("split");
    if (out.kind !== "split") return;
    expect(out.children.map((c) => c.id)).toEqual([11, 10]);
  });

  it("moving to a perpendicular side promotes the target into a new split", () => {
    // Move 11 to the bottom of 10: 10 becomes a col split [10, 11].
    const out = moveLeafBeside(row(10, 11), 11, 10, 99, "bottom");
    expect(out.kind).toBe("split");
    if (out.kind !== "split") return;
    // Source's parent collapsed (only 10 remained), so the col split is the root.
    expect(out.id).toBe(99);
    expect(out.dir).toBe("col");
    expect(out.children.map((c) => c.id)).toEqual([10, 11]);
  });

  it("move-onto-self is a no-op (returns the same tree object)", () => {
    const tree = row(10, 11);
    expect(moveLeafBeside(tree, 11, 11, 99, "left")).toBe(tree);
  });

  it("moving the only leaf is a no-op", () => {
    const tree = leaf(10);
    // Target id is bogus, but the source-is-last-leaf guard fires first.
    expect(moveLeafBeside(tree, 10, 999, 99, "right")).toBe(tree);
  });

  it("finds the target after the source's parent collapses", () => {
    // [ [10,11] , 12 ]: move 11 next to 12. Removing 11 collapses the inner
    // split to bare 10, but 12 must still be found in the pruned tree.
    const tree: PaneNode = {
      kind: "split",
      id: 5,
      dir: "row",
      children: [
        { kind: "split", id: 6, dir: "col", children: [leaf(10), leaf(11)] },
        leaf(12),
      ],
    };
    // Sanity: removing 11 leaves 12 reachable.
    const out = moveLeafBeside(tree, 11, 12, 99, "right");
    expect(hasLeaf(out, 12)).toBe(true);
    expect(hasLeaf(out, 11)).toBe(true);
    expect(hasLeaf(out, 10)).toBe(true);
    if (out.kind !== "split") return;
    // Outer row, inner collapsed to leaf 10, then [10, 12, 11] merged on the row.
    expect(out.children.map((c) => c.id)).toEqual([10, 12, 11]);
  });

  it("preserves the moving leaf's id (PTY/title travel with it)", () => {
    const out = moveLeafBeside(row(10, 11), 11, 10, 99, "left");
    expect(hasLeaf(out, 11)).toBe(true);
    expect(findLeaf(out, 11)).not.toBeNull();
  });

  it("repeated same-direction moves do not nest wrappers (merge-aware)", () => {
    // [10, 11, 12]: move 12 left of 10, then move 11 left of 10 again. Should
    // stay a single flat row, never accumulating nested splits.
    let tree = row(10, 11, 12);
    tree = moveLeafBeside(tree, 12, 10, 99, "left");
    if (tree.kind !== "split") throw new Error("expected split");
    expect(tree.children.every((c) => c.kind === "leaf")).toBe(true);
    expect(tree.children.map((c) => c.id)).toEqual([12, 10, 11]);
    tree = moveLeafBeside(tree, 11, 10, 100, "left");
    if (tree.kind !== "split") throw new Error("expected split");
    // Still a single flat row of leaves, no nested wrappers.
    expect(tree.children.every((c) => c.kind === "leaf")).toBe(true);
    expect(tree.children.map((c) => c.id)).toEqual([12, 11, 10]);
  });
});

describe("usePaneTitleStore", () => {
  beforeEach(() => {
    usePaneTitleStore.setState({ titles: {} });
  });

  it("preserves the color when renaming", () => {
    const s = usePaneTitleStore.getState();
    s.setPaneTitle(1, "Notes", false, "#d8a657");
    s.renamePane(1, "My Notes");
    const entry = usePaneTitleStore.getState().titles[1];
    expect(entry).toMatchObject({ label: "My Notes", color: "#d8a657" });
  });

  it("clears the entry when renamed to empty", () => {
    const s = usePaneTitleStore.getState();
    s.setPaneTitle(1, "Notes", false, "#d8a657");
    s.renamePane(1, "   ");
    expect(usePaneTitleStore.getState().titles[1]).toBeUndefined();
  });

  it("setPaneColor sets a color on a previously untitled pane", () => {
    usePaneTitleStore.getState().setPaneColor(2, "#5fb8a8");
    expect(usePaneTitleStore.getState().titles[2]).toMatchObject({
      color: "#5fb8a8",
      locked: false,
    });
  });

  it("renamePane and setPaneColor are no-ops on locked panes", () => {
    const s = usePaneTitleStore.getState();
    s.setPaneTitle(3, "Director", true, "#abc123");
    s.renamePane(3, "Hacked");
    s.setPaneColor(3, "#000000");
    expect(usePaneTitleStore.getState().titles[3]).toMatchObject({
      label: "Director",
      locked: true,
      color: "#abc123",
    });
  });
});
