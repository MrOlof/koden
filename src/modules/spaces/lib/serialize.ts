import {
  isLeaf,
  type PaneNode,
  type SplitDir,
} from "@/modules/terminal/lib/panes";
import type {
  BoardTab,
  EditorTab,
  LibraryTab,
  MarkdownTab,
  NotesTab,
  OrchestrationTab,
  OrchestrationView,
  PreviewTab,
  Tab,
  TasksTab,
  TerminalTab,
} from "@/modules/tabs/lib/useTabs";

export type SerializedNode =
  | {
      kind: "leaf";
      cwd?: string;
      active?: boolean;
      content?: "note" | "tasks";
      docId?: string;
      /** Custom per-pane label (from usePaneTitleStore). */
      title?: string;
      /** Custom per-pane accent color (from usePaneTitleStore). */
      color?: string;
      /** Restart-stable identity for the leaf's scrollback snapshot
       * (koden-scrollback.json). Absent in files written before restore. */
      key?: string;
    }
  | { kind: "split"; dir: SplitDir; children: SerializedNode[] };

/** Reads a leaf's persisted title/color (typically usePaneTitleStore). Only
 * unlocked entries should be returned: locked panes (Director/agents) are
 * recreated programmatically on boot, not restored from disk. */
export type PaneTitleReader = (
  leafId: number,
) => { label?: string; color?: string } | undefined;

/** Re-seeds a leaf's title/color after hydration (typically setPaneTitle). */
export type PaneTitleSeeder = (
  leafId: number,
  title: string | undefined,
  color: string | undefined,
) => void;

/** Reads (minting if needed) a leaf's restart-stable restore key. */
export type LeafKeyReader = (leafId: number) => string | undefined;

/** Re-binds a persisted restore key to the freshly allocated leaf id. */
export type LeafKeySeeder = (leafId: number, key: string) => void;

export type SerializedTab =
  | {
      kind: "terminal";
      tree: SerializedNode;
      blocks?: boolean;
      customTitle?: string;
    }
  | { kind: "editor"; path: string }
  | { kind: "preview"; url: string }
  | { kind: "markdown"; path: string }
  | { kind: "notes"; docId: string; title: string }
  | { kind: "board"; boardId: string; title: string }
  | { kind: "tasks"; listId: string; title: string }
  | { kind: "library" }
  | { kind: OrchestrationView; title: string };

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : path;
}

function titleFromUrl(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url || "preview";
  }
}

function serializeNode(
  node: PaneNode,
  activeLeafId: number,
  getTitle?: PaneTitleReader,
  getKey?: LeafKeyReader,
): SerializedNode {
  if (isLeaf(node)) {
    const t = getTitle?.(node.id);
    const k = getKey?.(node.id);
    return {
      kind: "leaf",
      ...(node.cwd !== undefined && { cwd: node.cwd }),
      ...(node.id === activeLeafId && { active: true }),
      ...(node.content !== undefined && { content: node.content }),
      ...(node.docId !== undefined && { docId: node.docId }),
      ...(t?.label && { title: t.label }),
      ...(t?.color && { color: t.color }),
      ...(k && { key: k }),
    };
  }
  return {
    kind: "split",
    dir: node.dir,
    children: node.children.map((c) =>
      serializeNode(c, activeLeafId, getTitle, getKey),
    ),
  };
}

export function isSerializableTab(tab: Tab): boolean {
  switch (tab.kind) {
    case "terminal":
      return !tab.private;
    case "editor":
    case "preview":
    case "markdown":
    case "notes":
    case "board":
    case "tasks":
    case "library":
    case "agent-topology":
    case "message-flow":
    case "director":
    case "brain":
      return true;
    default:
      return false;
  }
}

function serializeTab(
  tab: Tab,
  getTitle?: PaneTitleReader,
  getKey?: LeafKeyReader,
): SerializedTab | null {
  if (!isSerializableTab(tab)) return null;
  switch (tab.kind) {
    case "terminal":
      return {
        kind: "terminal",
        tree: serializeNode(tab.paneTree, tab.activeLeafId, getTitle, getKey),
        ...(tab.blocks && { blocks: true }),
        ...(tab.customTitle !== undefined && { customTitle: tab.customTitle }),
      };
    case "editor":
      return { kind: "editor", path: tab.path };
    case "preview":
      return { kind: "preview", url: tab.url };
    case "markdown":
      return { kind: "markdown", path: tab.path };
    case "notes":
      return { kind: "notes", docId: tab.docId, title: tab.title };
    case "board":
      return { kind: "board", boardId: tab.boardId, title: tab.title };
    case "tasks":
      return { kind: "tasks", listId: tab.listId, title: tab.title };
    case "library":
      return { kind: "library" };
    case "agent-topology":
    case "message-flow":
    case "director":
    case "brain":
      return { kind: tab.kind, title: tab.title };
    default:
      return null;
  }
}

export function serializeTabs(
  tabs: Tab[],
  getTitle?: PaneTitleReader,
  getKey?: LeafKeyReader,
): SerializedTab[] {
  const out: SerializedTab[] = [];
  for (const tab of tabs) {
    const s = serializeTab(tab, getTitle, getKey);
    if (s) out.push(s);
  }
  return out;
}

type HydratedTree = {
  tree: PaneNode;
  activeLeafId: number;
  firstLeafCwd?: string;
};

function hydrateNode(
  node: SerializedNode,
  allocId: () => number,
  acc: { activeLeafId: number | null },
  seedTitle?: PaneTitleSeeder,
  seedKey?: LeafKeySeeder,
): PaneNode {
  if (node.kind === "leaf") {
    const id = allocId();
    if (node.active && acc.activeLeafId === null) acc.activeLeafId = id;
    if (seedTitle && (node.title || node.color))
      seedTitle(id, node.title, node.color);
    if (seedKey && typeof node.key === "string" && node.key)
      seedKey(id, node.key);
    return {
      kind: "leaf",
      id,
      ...(node.cwd !== undefined && { cwd: node.cwd }),
      ...(node.content !== undefined && { content: node.content }),
      ...(node.docId !== undefined && { docId: node.docId }),
    };
  }
  const children = node.children.map((c) =>
    hydrateNode(c, allocId, acc, seedTitle, seedKey),
  );
  if (children.length === 0) return { kind: "leaf", id: allocId() };
  if (children.length === 1) return children[0];
  return { kind: "split", id: allocId(), dir: node.dir, children };
}

function hydrateTree(
  tree: SerializedNode,
  allocId: () => number,
  seedTitle?: PaneTitleSeeder,
  seedKey?: LeafKeySeeder,
): HydratedTree {
  const acc: { activeLeafId: number | null } = { activeLeafId: null };
  const paneTree = hydrateNode(tree, allocId, acc, seedTitle, seedKey);
  const leaves = collectLeaves(paneTree);
  const activeLeafId = acc.activeLeafId ?? leaves[0]?.id ?? allocId();
  const firstLeafCwd =
    leaves.find((l) => l.id === activeLeafId)?.cwd ?? leaves[0]?.cwd;
  return { tree: paneTree, activeLeafId, firstLeafCwd };
}

/**
 * Live structural adoption (ADR-025): hydrate a peer's pane tree AROUND the
 * leaves this device already runs. A leaf whose restore key matches an
 * existing leaf is reused verbatim (its PTY stays attached, its cwd stays);
 * new leaves are allocated and seeded exactly as at boot.
 */
export function hydrateTreeReusing(
  tree: SerializedNode,
  existing: ReadonlyMap<string, Extract<PaneNode, { kind: "leaf" }>>,
  allocId: () => number,
  seedTitle?: PaneTitleSeeder,
  seedKey?: LeafKeySeeder,
): HydratedTree {
  const acc: { activeLeafId: number | null } = { activeLeafId: null };
  const build = (node: SerializedNode): PaneNode => {
    if (node.kind === "leaf") {
      const reused = node.key ? existing.get(node.key) : undefined;
      if (reused) {
        if (node.active && acc.activeLeafId === null)
          acc.activeLeafId = reused.id;
        return reused;
      }
      return hydrateNode(node, allocId, acc, seedTitle, seedKey);
    }
    const children = node.children.map(build);
    if (children.length === 0) return { kind: "leaf", id: allocId() };
    if (children.length === 1) return children[0];
    return { kind: "split", id: allocId(), dir: node.dir, children };
  };
  const paneTree = build(tree);
  const leaves = collectLeaves(paneTree);
  const activeLeafId = acc.activeLeafId ?? leaves[0]?.id ?? allocId();
  return { tree: paneTree, activeLeafId, firstLeafCwd: leaves[0]?.cwd };
}

function collectLeaves(node: PaneNode): Array<{ id: number; cwd?: string }> {
  if (isLeaf(node)) return [{ id: node.id, cwd: node.cwd }];
  return node.children.flatMap(collectLeaves);
}

function hydrateTab(
  s: SerializedTab,
  spaceId: string,
  allocId: () => number,
  seedTitle?: PaneTitleSeeder,
  seedKey?: LeafKeySeeder,
): Tab | null {
  switch (s.kind) {
    case "terminal": {
      const { tree, activeLeafId, firstLeafCwd } = hydrateTree(
        s.tree,
        allocId,
        seedTitle,
        seedKey,
      );
      const title =
        s.customTitle ??
        (firstLeafCwd ? basename(firstLeafCwd) : s.blocks ? "blocks" : "shell");
      return {
        id: allocId(),
        kind: "terminal",
        spaceId,
        cold: true,
        title,
        cwd: firstLeafCwd,
        paneTree: tree,
        activeLeafId,
        ...(s.blocks && { blocks: true }),
        ...(s.customTitle !== undefined && { customTitle: s.customTitle }),
      } satisfies TerminalTab;
    }
    case "editor":
      return {
        id: allocId(),
        kind: "editor",
        spaceId,
        cold: true,
        title: basename(s.path),
        path: s.path,
        dirty: false,
        preview: false,
      } satisfies EditorTab;
    case "preview":
      return {
        id: allocId(),
        kind: "preview",
        spaceId,
        cold: true,
        title: titleFromUrl(s.url),
        url: s.url,
      } satisfies PreviewTab;
    case "markdown":
      return {
        id: allocId(),
        kind: "markdown",
        spaceId,
        cold: true,
        title: basename(s.path),
        path: s.path,
      } satisfies MarkdownTab;
    case "notes":
      return {
        id: allocId(),
        kind: "notes",
        spaceId,
        cold: true,
        title: s.title,
        docId: s.docId,
      } satisfies NotesTab;
    case "board":
      return {
        id: allocId(),
        kind: "board",
        spaceId,
        cold: true,
        title: s.title,
        boardId: s.boardId,
      } satisfies BoardTab;
    case "tasks":
      return {
        id: allocId(),
        kind: "tasks",
        spaceId,
        cold: true,
        title: s.title,
        listId: s.listId,
      } satisfies TasksTab;
    case "library":
      return {
        id: allocId(),
        kind: "library",
        spaceId,
        cold: true,
        title: "Library",
      } satisfies LibraryTab;
    case "agent-topology":
    case "message-flow":
    case "director":
    case "brain":
      return {
        id: allocId(),
        kind: s.kind,
        spaceId,
        cold: true,
        title: s.title,
      } satisfies OrchestrationTab;
    default:
      return null;
  }
}

export function freshTerminalTab(
  spaceId: string,
  cwd: string | null,
  allocId: () => number,
): TerminalTab {
  const leafId = allocId();
  return {
    id: allocId(),
    kind: "terminal",
    spaceId,
    cold: true,
    title: cwd ? basename(cwd) : "shell",
    cwd: cwd ?? undefined,
    paneTree: { kind: "leaf", id: leafId, ...(cwd && { cwd }) },
    activeLeafId: leafId,
  };
}

export function hydrateTabs(
  serialized: SerializedTab[],
  spaceId: string,
  allocId: () => number,
  seedTitle?: PaneTitleSeeder,
  seedKey?: LeafKeySeeder,
): Tab[] {
  if (!Array.isArray(serialized)) return [];
  const out: Tab[] = [];
  for (const s of serialized) {
    try {
      const tab = hydrateTab(s, spaceId, allocId, seedTitle, seedKey);
      if (tab) out.push(tab);
    } catch {
      // Skip corrupted entries rather than failing the whole restore.
    }
  }
  return out;
}
