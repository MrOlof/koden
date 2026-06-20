export type PaneId = number;

export type SplitDir = "row" | "col";

/** A 4-way split direction relative to a pane. */
export type SplitSide = "left" | "right" | "top" | "bottom";

/** Map a 4-way side to the split axis + whether the new pane leads the target.
 * Right = row/after, Left = row/before, Bottom = col/after, Top = col/before. */
export function sideToSplit(side: SplitSide): {
  dir: SplitDir;
  before: boolean;
} {
  switch (side) {
    case "left":
      return { dir: "row", before: true };
    case "right":
      return { dir: "row", before: false };
    case "top":
      return { dir: "col", before: true };
    case "bottom":
      return { dir: "col", before: false };
  }
}

export type PaneNode =
  | {
      kind: "leaf";
      id: PaneId;
      cwd?: string;
      /** "note"/"tasks" = a docs-backed pane; absent = a terminal. */
      content?: "note" | "tasks";
      /** Key into the workspace-docs store (note = notes, tasks = task list). */
      docId?: string;
    }
  | {
      kind: "split";
      id: PaneId;
      dir: SplitDir;
      children: PaneNode[];
    };

export function isLeaf(
  n: PaneNode,
): n is Extract<PaneNode, { kind: "leaf" }> {
  return n.kind === "leaf";
}

export function leafIds(n: PaneNode): PaneId[] {
  if (isLeaf(n)) return [n.id];
  return n.children.flatMap(leafIds);
}

/**
 * Count leaves that consume a terminal renderer slot (note panes are plain
 * textareas and don't). The per-tab pane cap is renderer-bound, so it should
 * count these, not every leaf.
 */
export function countTerminalLeaves(n: PaneNode): number {
  if (isLeaf(n)) return n.content ? 0 : 1;
  return n.children.reduce((sum, c) => sum + countTerminalLeaves(c), 0);
}

/**
 * Leaf nodes that host a terminal (excludes docs-backed note/tasks panes), with
 * their cwd. Used to surface every running terminal as an agent in the Agents
 * panel.
 */
export function terminalLeaves(
  n: PaneNode,
): Array<{ id: PaneId; cwd?: string }> {
  if (isLeaf(n)) return n.content ? [] : [{ id: n.id, cwd: n.cwd }];
  return n.children.flatMap(terminalLeaves);
}

export function findLeaf(
  n: PaneNode,
  id: PaneId,
): Extract<PaneNode, { kind: "leaf" }> | null {
  if (isLeaf(n)) return n.id === id ? n : null;
  for (const c of n.children) {
    const found = findLeaf(c, id);
    if (found) return found;
  }
  return null;
}

export function findLeafCwd(n: PaneNode, id: PaneId): string | undefined {
  if (isLeaf(n)) return n.id === id ? n.cwd : undefined;
  for (const c of n.children) {
    const found = findLeafCwd(c, id);
    if (found !== undefined) return found;
  }
  return undefined;
}

export function setLeafCwd(
  n: PaneNode,
  id: PaneId,
  cwd: string,
): PaneNode {
  if (isLeaf(n)) {
    if (n.id !== id || n.cwd === cwd) return n;
    return { ...n, cwd };
  }
  let changed = false;
  const next = n.children.map((c) => {
    const u = setLeafCwd(c, id, cwd);
    if (u !== c) changed = true;
    return u;
  });
  return changed ? { ...n, children: next } : n;
}

/**
 * Insert a pre-built leaf next to `targetId` in direction `dir`. When `before`
 * is true the new leaf lands on the leading side (Left for row, Top for col);
 * otherwise on the trailing side (Right for row, Bottom for col).
 *
 * If the target's enclosing split already runs in `dir`, the new leaf is
 * appended as a sibling there (avoids nested same-direction splits — keeps
 * the tree shallow and the resize handles aligned).
 */
export function insertBeside(
  tree: PaneNode,
  targetId: PaneId,
  newSplitId: PaneId,
  dir: SplitDir,
  newLeaf: PaneNode,
  before: boolean,
): PaneNode {
  if (tree.kind === "split" && tree.dir === dir) {
    const idx = tree.children.findIndex(
      (c) => c.kind === "leaf" && c.id === targetId,
    );
    if (idx >= 0) {
      const at = before ? idx : idx + 1;
      return {
        ...tree,
        children: [
          ...tree.children.slice(0, at),
          newLeaf,
          ...tree.children.slice(at),
        ],
      };
    }
  }
  if (isLeaf(tree)) {
    if (tree.id !== targetId) return tree;
    return {
      kind: "split",
      id: newSplitId,
      dir,
      children: before ? [newLeaf, tree] : [tree, newLeaf],
    };
  }
  return {
    ...tree,
    children: tree.children.map((c) =>
      insertBeside(c, targetId, newSplitId, dir, newLeaf, before),
    ),
  };
}

/** Split `targetId` along `dir`, adding a new terminal leaf beside it. */
export function splitLeaf(
  tree: PaneNode,
  targetId: PaneId,
  newSplitId: PaneId,
  newLeafId: PaneId,
  dir: SplitDir,
  newCwd?: string,
  before = false,
): PaneNode {
  return insertBeside(
    tree,
    targetId,
    newSplitId,
    dir,
    { kind: "leaf", id: newLeafId, cwd: newCwd },
    before,
  );
}

/** Split `targetId` along `dir`, adding a docs-backed note pane beside it. */
export function splitLeafNote(
  tree: PaneNode,
  targetId: PaneId,
  newSplitId: PaneId,
  newLeafId: PaneId,
  dir: SplitDir,
  docId: string,
  before = false,
): PaneNode {
  return insertBeside(
    tree,
    targetId,
    newSplitId,
    dir,
    { kind: "leaf", id: newLeafId, content: "note", docId },
    before,
  );
}

/** Split `targetId` along `dir`, adding a docs-backed tasks pane beside it. */
export function splitLeafTasks(
  tree: PaneNode,
  targetId: PaneId,
  newSplitId: PaneId,
  newLeafId: PaneId,
  dir: SplitDir,
  listId: string,
  before = false,
): PaneNode {
  return insertBeside(
    tree,
    targetId,
    newSplitId,
    dir,
    { kind: "leaf", id: newLeafId, content: "tasks", docId: listId },
    before,
  );
}

/**
 * Remove a leaf and collapse single-child splits left in its wake. Returns
 * `null` when the entire subtree is gone.
 */
export function removeLeaf(
  tree: PaneNode,
  targetId: PaneId,
): PaneNode | null {
  if (isLeaf(tree)) return tree.id === targetId ? null : tree;
  const newChildren: PaneNode[] = [];
  for (const c of tree.children) {
    const r = removeLeaf(c, targetId);
    if (r !== null) newChildren.push(r);
  }
  if (newChildren.length === 0) return null;
  if (newChildren.length === 1) return newChildren[0];
  return { ...tree, children: newChildren };
}

/**
 * Re-dock an existing leaf beside another within the same tree: a move is
 * `removeLeaf` (prune the source, collapsing single-child wrappers) followed by
 * `insertBeside` (drop it next to the target on `side`). The SAME leaf object is
 * reused — never rebuilt — so its id (and thus the live PTY + the title/color
 * keyed by that id) travels with it. Returns the original `tree` unchanged on
 * any no-op so callers can cheaply detect "nothing moved".
 */
export function moveLeafBeside(
  tree: PaneNode,
  sourceLeafId: PaneId,
  targetLeafId: PaneId,
  newSplitId: PaneId,
  side: SplitSide,
): PaneNode {
  // Drop onto self: nothing to do.
  if (sourceLeafId === targetLeafId) return tree;
  // Reuse the existing leaf OBJECT so its id survives the round-trip.
  const moving = findLeaf(tree, sourceLeafId);
  if (!moving) return tree;
  const pruned = removeLeaf(tree, sourceLeafId);
  // Source was the last leaf in the tree — nothing left to dock beside.
  if (pruned === null) return tree;
  // Target collapsed away while pruning (e.g. it shared a now-removed wrapper):
  // bail rather than insert beside a vanished node.
  if (!hasLeaf(pruned, targetLeafId)) return tree;
  const { dir, before } = sideToSplit(side);
  return insertBeside(pruned, targetLeafId, newSplitId, dir, moving, before);
}

export function nextLeafId(
  tree: PaneNode,
  currentId: PaneId,
  delta: 1 | -1,
): PaneId {
  const ids = leafIds(tree);
  if (ids.length === 0) return currentId;
  const idx = ids.indexOf(currentId);
  if (idx < 0) return ids[0];
  return ids[(idx + delta + ids.length) % ids.length];
}

// Closest neighbor of `leafId` within its enclosing split — prefer the
// next sibling, fall back to the previous. Used to pick the new focus
// when a pane closes (so focus stays in the same neighborhood instead of
// snapping to the first pane in the tree).
export function siblingLeafOf(
  tree: PaneNode,
  leafId: PaneId,
): PaneId | null {
  if (isLeaf(tree)) return null;
  for (let i = 0; i < tree.children.length; i++) {
    const c = tree.children[i];
    if (isLeaf(c) && c.id === leafId) {
      const sibling = tree.children[i + 1] ?? tree.children[i - 1];
      if (!sibling) return null;
      return leafIds(sibling)[0] ?? null;
    }
  }
  for (const c of tree.children) {
    if (!isLeaf(c)) {
      const r = siblingLeafOf(c, leafId);
      if (r !== null) return r;
    }
  }
  return null;
}

export function hasLeaf(tree: PaneNode, id: PaneId): boolean {
  return leafIds(tree).includes(id);
}
