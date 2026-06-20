import type { PaneNode } from "@/modules/terminal/lib/panes";

/** Clamp a grid dimension to the supported 1..8 range. */
export function clampGridDim(n: number): number {
  if (!Number.isFinite(n)) return 1;
  return Math.min(8, Math.max(1, Math.floor(n)));
}

/**
 * Build an R rows x C cols grid of terminal leaves as a nested split tree:
 * an outer "col" split of `rows` rows, each row a "row" split of `cols`
 * terminal leaves. `allocId` must hand out a unique id per call (the tab
 * hook's id counter) — every split AND leaf id is allocated from it. Leaf ids
 * are returned in row-major order so callers can drive each pane.
 *
 * A 1x1 grid is a single terminal leaf (no enclosing split), matching how a
 * plain tab is shaped.
 */
export function buildGridTree(
  rows: number,
  cols: number,
  allocId: () => number,
  cwd?: string,
): { tree: PaneNode; leafIds: number[] } {
  const r = clampGridDim(rows);
  const c = clampGridDim(cols);
  const leafIds: number[] = [];

  const makeRow = (): PaneNode => {
    const children: PaneNode[] = [];
    for (let i = 0; i < c; i++) {
      const id = allocId();
      leafIds.push(id);
      children.push({ kind: "leaf", id, cwd });
    }
    // ponytail: a single-column row is just its leaf — no 1-child split wrapper.
    if (children.length === 1) return children[0];
    return { kind: "split", id: allocId(), dir: "row", children };
  };

  if (r === 1) return { tree: makeRow(), leafIds };

  const rowsNodes: PaneNode[] = [];
  for (let i = 0; i < r; i++) rowsNodes.push(makeRow());
  return {
    tree: { kind: "split", id: allocId(), dir: "col", children: rowsNodes },
    leafIds,
  };
}
