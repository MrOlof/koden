import {
  countTerminalLeaves,
  isLeaf,
  leafIds as treeLeafIds,
  type PaneNode,
} from "@/modules/terminal/lib/panes";
import { describe, expect, it } from "vitest";
import { buildGridTree, clampGridDim } from "./grid";

function makeAllocId() {
  let n = 100;
  return () => n++;
}

describe("buildGridTree", () => {
  it("builds a 2x3 grid: col split of 2 row splits of 3 leaves each", () => {
    const { tree, leafIds } = buildGridTree(2, 3, makeAllocId());

    expect(tree.kind).toBe("split");
    if (tree.kind !== "split") throw new Error("expected outer split");
    expect(tree.dir).toBe("col");
    expect(tree.children).toHaveLength(2);

    for (const row of tree.children) {
      expect(row.kind).toBe("split");
      if (row.kind !== "split") throw new Error("expected row split");
      expect(row.dir).toBe("row");
      expect(row.children).toHaveLength(3);
      expect(row.children.every(isLeaf)).toBe(true);
    }

    expect(leafIds).toHaveLength(6);
    expect(new Set(leafIds).size).toBe(6);
    expect(treeLeafIds(tree)).toHaveLength(6);
    expect(treeLeafIds(tree)).toEqual(leafIds);
    expect(countTerminalLeaves(tree)).toBe(6);
  });

  it("allocates every split and leaf id uniquely from the counter", () => {
    const alloc = makeAllocId();
    const { tree } = buildGridTree(3, 3, alloc);
    const ids: number[] = [];
    const walk = (n: PaneNode) => {
      ids.push(n.id);
      if (n.kind === "split") n.children.forEach(walk);
    };
    walk(tree);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("returns a single leaf for a 1x1 grid", () => {
    const { tree, leafIds } = buildGridTree(1, 1, makeAllocId());
    expect(tree.kind).toBe("leaf");
    expect(leafIds).toHaveLength(1);
    expect(countTerminalLeaves(tree)).toBe(1);
  });

  it("carries cwd onto every leaf", () => {
    const { tree } = buildGridTree(2, 2, makeAllocId(), "/work/repo");
    const cwds: Array<string | undefined> = [];
    const walk = (n: PaneNode) => {
      if (n.kind === "leaf") cwds.push(n.cwd);
      else n.children.forEach(walk);
    };
    walk(tree);
    expect(cwds).toEqual(["/work/repo", "/work/repo", "/work/repo", "/work/repo"]);
  });

  it("clamps dimensions to 1..8", () => {
    expect(clampGridDim(0)).toBe(1);
    expect(clampGridDim(-4)).toBe(1);
    expect(clampGridDim(99)).toBe(8);
    expect(clampGridDim(3.7)).toBe(3);
    const { leafIds } = buildGridTree(20, 20, makeAllocId());
    expect(leafIds).toHaveLength(64);
  });
});
