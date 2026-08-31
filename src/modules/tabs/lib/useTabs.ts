import { isMarkdownPath } from "@/lib/utils";
import {
  countTerminalLeaves,
  findLeafCwd,
  hasLeaf,
  leafIds,
  moveLeafBeside,
  nextLeafId,
  type PaneNode,
  removeLeaf,
  type SplitDir,
  type SplitSide,
  setLeafCwd as setLeafCwdInTree,
  siblingLeafOf,
  splitLeaf,
  splitLeafNote,
  splitLeafTasks,
} from "@/modules/terminal/lib/panes";
import { disposeSession } from "@/modules/terminal/lib/useTerminalSession";
import { useCallback, useEffect, useRef, useState } from "react";
import { buildGridTree } from "./grid";

// Terminal panes per tab — matches the renderer slot pool size (POOL_MAX_SIZE);
// over this we'd evict an active leaf. Note panes are textareas and don't count.
// 64 keeps interactive splitting consistent with the grid launcher's 8x8 ceiling.
export const MAX_PANES_PER_TAB = 64;
// Overall pane cap (terminals + notes) so layouts stay usable given the 10% min
// panel size.
export const MAX_TOTAL_PANES_PER_TAB = 64;

type TabBase = {
  spaceId: string;
  /** Restored from disk, not yet activated: rendered as a placeholder, not mounted. */
  cold?: boolean;
};

export type TerminalTab = TabBase & {
  id: number;
  kind: "terminal";
  title: string;
  cwd?: string;
  paneTree: PaneNode;
  activeLeafId: number;
  blocks?: boolean;
  /** AI agent cannot read buffer / context of this terminal. */
  private?: boolean;
  /** User-set label that overrides the cwd-derived name. Survives cd. */
  customTitle?: string;
};

export type EditorTab = TabBase & {
  id: number;
  kind: "editor";
  title: string;
  path: string;
  dirty: boolean;
  /**
   * True while the tab is in the transient "preview" state — opened by a
   * single-click in the explorer and not yet pinned by the user. A preview tab
   * is replaced by the next single-click rather than accumulating.
   */
  preview: boolean;
};

export type PreviewTab = TabBase & {
  id: number;
  kind: "preview";
  title: string;
  url: string;
};

export type MarkdownTab = TabBase & {
  id: number;
  kind: "markdown";
  title: string;
  path: string;
};

export type AiDiffStatus = "pending" | "approved" | "rejected";

export type AiDiffTab = TabBase & {
  id: number;
  kind: "ai-diff";
  title: string;
  path: string;
  /** "" for newly created files. */
  originalContent: string;
  proposedContent: string;
  /** Tool-call approval id used to resolve the AI SDK approval. */
  approvalId: string;
  status: AiDiffStatus;
  isNewFile: boolean;
};

export type GitDiffTab = TabBase & {
  id: number;
  kind: "git-diff";
  title: string;
  path: string;
  repoRoot: string;
  mode: "-" | "+";
  originalPath: string | null;
};

export type GitHistoryTab = TabBase & {
  id: number;
  kind: "git-history";
  title: string;
  repoRoot: string;
};

export type GitCommitFileDiffTab = TabBase & {
  id: number;
  kind: "git-commit-file";
  title: string;
  repoRoot: string;
  sha: string;
  shortSha: string;
  subject: string;
  path: string;
  originalPath: string | null;
};

export type NotesTab = TabBase & {
  id: number;
  kind: "notes";
  title: string;
  /** Key into the workspace-docs notes store; content persists separately. */
  docId: string;
};

export type BoardTab = TabBase & {
  id: number;
  kind: "board";
  title: string;
  /** Key into the workspace-docs board store. */
  boardId: string;
};

export type TasksTab = TabBase & {
  id: number;
  kind: "tasks";
  title: string;
  /** Key into the workspace-docs tasks store. */
  listId: string;
};

/** The Library: read-only wiki of the Librarian's memory. One per space. */
export type LibraryTab = TabBase & {
  id: number;
  kind: "library";
  title: string;
};

/** The "What do you want to do?" page. One per space, never persisted. */
export type LauncherTab = TabBase & {
  id: number;
  kind: "launcher";
  title: string;
};

/** Singleton workspace views (one per space). "brain" is the Koden Brain pane;
 *  "brain-map" is its interactive knowledge-graph view. */
export type OrchestrationView =
  | "agent-topology"
  | "message-flow"
  | "director"
  | "brain"
  | "brain-map";

export type OrchestrationTab = TabBase & {
  id: number;
  kind: OrchestrationView;
  title: string;
};

export type Tab =
  | TerminalTab
  | EditorTab
  | PreviewTab
  | MarkdownTab
  | AiDiffTab
  | GitDiffTab
  | GitHistoryTab
  | GitCommitFileDiffTab
  | NotesTab
  | BoardTab
  | TasksTab
  | LibraryTab
  | LauncherTab
  | OrchestrationTab;

const ORCHESTRATION_TITLES: Record<OrchestrationView, string> = {
  "agent-topology": "Agent Topology",
  "message-flow": "Message Flow",
  director: "Director",
  brain: "Brain",
  "brain-map": "Brain Map",
};

export function isRenamableKind(kind: Tab["kind"]): boolean {
  return (
    kind === "terminal" ||
    kind === "notes" ||
    kind === "board" ||
    kind === "tasks"
  );
}

function newDocId(prefix: string): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

export type TabPatch = Partial<{
  title: string;
  cwd: string;
  path: string;
  dirty: boolean;
  url: string;
  /** Empty string resets a terminal tab to its cwd-derived name. */
  customTitle: string;
}>;

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : path;
}

function titleFromUrl(url: string): string {
  try {
    const u = new URL(url);
    return u.host || url;
  } catch {
    return url || "preview";
  }
}

export const DEFAULT_SPACE_ID = "default";

// Next active after close, scoped to the closing tab's space. null = last tab of
// its space, which callers treat as "refuse to close".
export function nextActiveInSpace(
  tabs: Tab[],
  closingId: number,
): number | null {
  const closing = tabs.find((t) => t.id === closingId);
  if (!closing) return null;
  const sameSpace = tabs.filter((t) => t.spaceId === closing.spaceId);
  if (sameSpace.length <= 1) return null;
  const idx = sameSpace.findIndex((t) => t.id === closingId);
  return (sameSpace[idx - 1] ?? sameSpace[idx + 1]).id;
}

export function useTabs(initial?: Partial<TerminalTab>) {
  const [tabs, setTabs] = useState<Tab[]>(() => {
    const tabId = 1;
    const leafId = 2;
    return [
      {
        id: tabId,
        kind: "terminal",
        spaceId: DEFAULT_SPACE_ID,
        cold: true,
        title: initial?.title ?? "shell",
        cwd: initial?.cwd,
        paneTree: { kind: "leaf", id: leafId, cwd: initial?.cwd },
        activeLeafId: leafId,
      },
    ];
  });
  const [activeId, setActiveId] = useState(1);
  // Gates warming until boot resolves the restore, so no shell spawns before it.
  const [booted, setBooted] = useState(false);
  const nextIdRef = useRef(3);
  const activeSpaceIdRef = useRef(DEFAULT_SPACE_ID);
  const tabsRef = useRef(tabs);
  const activeIdRef = useRef(activeId);

  useEffect(() => {
    tabsRef.current = tabs;
  }, [tabs]);

  useEffect(() => {
    activeIdRef.current = activeId;
  }, [activeId]);

  // Activating a cold tab warms it: one choke point for every activation path.
  useEffect(() => {
    if (!booted) return;
    setTabs((curr) => {
      const t = curr.find((x) => x.id === activeId);
      if (!t?.cold) return curr;
      return curr.map((x) => (x.id === activeId ? { ...x, cold: false } : x));
    });
  }, [activeId, booted]);

  const allocId = useCallback(() => nextIdRef.current++, []);

  const markBooted = useCallback(() => setBooted(true), []);

  const setActiveSpaceForNewTabs = useCallback((spaceId: string) => {
    activeSpaceIdRef.current = spaceId;
  }, []);

  const replaceTabs = useCallback((next: Tab[], nextActiveId: number) => {
    if (next.length === 0) return;
    setTabs(next);
    setActiveId(nextActiveId);
  }, []);

  // Appends a cold terminal tab to a space without stealing focus, so the
  // overview can populate a space in place; it spawns when first opened.
  const newTabInSpace = useCallback((spaceId: string, cwd?: string) => {
    const tabId = nextIdRef.current++;
    const leafId = nextIdRef.current++;
    setTabs((curr) => [
      ...curr,
      {
        id: tabId,
        kind: "terminal",
        spaceId,
        cold: true,
        title: cwd ? basename(cwd) : "shell",
        cwd,
        paneTree: { kind: "leaf", id: leafId, cwd },
        activeLeafId: leafId,
      },
    ]);
    return tabId;
  }, []);

  // Reassigns a tab to another space. Returns true when the moved tab was active
  // and emptied its source space, so the caller should follow it into the target.
  const moveTabToSpace = useCallback(
    (tabId: number, targetSpaceId: string): boolean => {
      const curr = tabsRef.current;
      const tab = curr.find((t) => t.id === tabId);
      if (!tab || tab.spaceId === targetSpaceId) return false;
      setTabs((prev) =>
        prev.map((t) =>
          t.id === tabId ? ({ ...t, spaceId: targetSpaceId } as Tab) : t,
        ),
      );
      if (activeIdRef.current !== tabId) return false;
      const fallback = nextActiveInSpace(curr, tabId);
      if (fallback !== null) {
        setActiveId(fallback);
        return false;
      }
      return true;
    },
    [],
  );

  // Positions a tab next to a target tab, inheriting the target's space. Returns
  // true when the active tab crossed into the target space and emptied its
  // source, so the caller should follow it.
  const reorderTab = useCallback(
    (tabId: number, targetTabId: number, edge: "top" | "bottom"): boolean => {
      if (tabId === targetTabId) return false;
      const curr = tabsRef.current;
      const moved = curr.find((t) => t.id === tabId);
      const target = curr.find((t) => t.id === targetTabId);
      if (!moved || !target) return false;
      const crossSpace = moved.spaceId !== target.spaceId;
      setTabs((prev) => {
        const without = prev.filter((t) => t.id !== tabId);
        let idx = without.findIndex((t) => t.id === targetTabId);
        if (idx < 0) return prev;
        if (edge === "bottom") idx += 1;
        const next: Tab = crossSpace
          ? ({ ...moved, spaceId: target.spaceId } as Tab)
          : moved;
        without.splice(idx, 0, next);
        return without;
      });
      if (!crossSpace || activeIdRef.current !== tabId) return false;
      const fallback = nextActiveInSpace(curr, tabId);
      if (fallback !== null) {
        setActiveId(fallback);
        return false;
      }
      return true;
    },
    [],
  );

  const removeTabsForSpace = useCallback((spaceId: string) => {
    let toDispose: number[] = [];
    setTabs((curr) => {
      const next = curr.filter((t) => t.spaceId !== spaceId);
      if (next.length === 0 || next.length === curr.length) return curr;
      toDispose = curr
        .filter((t) => t.spaceId === spaceId && t.kind === "terminal")
        .flatMap((t) => leafIds((t as TerminalTab).paneTree));
      return next;
    });
    for (const lid of toDispose) disposeSession(lid);
  }, []);

  const newTab = useCallback((cwd?: string) => {
    const tabId = nextIdRef.current++;
    const leafId = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id: tabId,
        kind: "terminal",
        spaceId: activeSpaceIdRef.current,
        title: "shell",
        cwd,
        paneTree: { kind: "leaf", id: leafId, cwd },
        activeLeafId: leafId,
      },
    ]);
    setActiveId(tabId);
    return tabId;
  }, []);

  const newBlockTab = useCallback((cwd?: string) => {
    const tabId = nextIdRef.current++;
    const leafId = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id: tabId,
        kind: "terminal",
        spaceId: activeSpaceIdRef.current,
        title: "blocks",
        cwd,
        paneTree: { kind: "leaf", id: leafId, cwd },
        activeLeafId: leafId,
        blocks: true,
      },
    ]);
    setActiveId(tabId);
    return tabId;
  }, []);

  useEffect(() => {
    if (!import.meta.env?.DEV || typeof window === "undefined") return;
    (
      window as unknown as { __kodenNewBlockTab?: (cwd?: string) => number }
    ).__kodenNewBlockTab = newBlockTab;
  }, [newBlockTab]);

  const newAgentTab = useCallback((cwd: string | undefined, title: string) => {
    const tabId = nextIdRef.current++;
    const leafId = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id: tabId,
        kind: "terminal",
        spaceId: activeSpaceIdRef.current,
        title,
        cwd,
        paneTree: { kind: "leaf", id: leafId, cwd },
        activeLeafId: leafId,
      },
    ]);
    setActiveId(tabId);
    return { tabId, leafId };
  }, []);

  // Hand-builds an R rows x C cols grid of terminal leaves in one new tab. Every
  // split + leaf id is drawn from the same counter as newTab, so ids stay unique
  // across the whole workspace. Bypasses the interactive split caps by design
  // (it builds the literal tree, never calling splitActivePane). Returns the leaf
  // ids in row-major order so the caller can auto-type a launch command per pane.
  const newGridTab = useCallback(
    (rows: number, cols: number, cwd?: string) => {
      const tabId = nextIdRef.current++;
      const { tree, leafIds: gridLeafIds } = buildGridTree(
        rows,
        cols,
        () => nextIdRef.current++,
        cwd,
      );
      const r = Math.min(8, Math.max(1, Math.floor(rows)));
      const c = Math.min(8, Math.max(1, Math.floor(cols)));
      setTabs((t) => [
        ...t,
        {
          id: tabId,
          kind: "terminal",
          spaceId: activeSpaceIdRef.current,
          title: `Grid ${r}×${c}`,
          cwd,
          paneTree: tree,
          activeLeafId: gridLeafIds[0],
        },
      ]);
      setActiveId(tabId);
      return { tabId, leafIds: gridLeafIds };
    },
    [],
  );

  const newPrivateTab = useCallback((cwd?: string) => {
    const tabId = nextIdRef.current++;
    const leafId = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id: tabId,
        kind: "terminal",
        spaceId: activeSpaceIdRef.current,
        title: "private",
        cwd,
        paneTree: { kind: "leaf", id: leafId, cwd },
        activeLeafId: leafId,
        private: true,
      },
    ]);
    setActiveId(tabId);
    return tabId;
  }, []);

  /**
   * Opens a file in an editor tab.
   *
   * - `pin = true` (default) — opens or activates a **persistent** tab.
   *   If the path is currently in the preview slot it is promoted in-place.
   *   Use this for programmatic opens (AI diff, New File dialog, etc.).
   * - `pin = false` — VSCode-style **preview** tab. A single shared slot is
   *   reused: if a persistent tab for the path already exists it is activated;
   *   otherwise the current preview slot is replaced with the new path.
   */
  const openFileTab = useCallback((path: string, pin = true) => {
    let targetId: number | null = null;
    setTabs((curr) => {
      if (pin) {
        // Persistent open: find any existing editor tab, pin it if needed.
        const existing = curr.find(
          (t) => t.kind === "editor" && t.path === path,
        );
        if (existing) {
          targetId = existing.id;
          if ((existing as EditorTab).preview) {
            return curr.map((t) =>
              t.id === existing.id ? { ...t, preview: false } : t,
            );
          }
          return curr;
        }
        const id = nextIdRef.current++;
        targetId = id;
        return [
          ...curr,
          {
            id,
            kind: "editor",
            spaceId: activeSpaceIdRef.current,
            title: basename(path),
            path,
            dirty: false,
            preview: false,
          } satisfies EditorTab,
        ];
      } else {
        // Preview open: persistent tab for this path takes priority.
        const persistent = curr.find(
          (t) =>
            t.kind === "editor" && t.path === path && !(t as EditorTab).preview,
        );
        if (persistent) {
          targetId = persistent.id;
          return curr;
        }
        // Reuse the slot if it already shows the same path.
        const existingPreview = curr.find(
          (t) =>
            t.kind === "editor" && t.path === path && (t as EditorTab).preview,
        );
        if (existingPreview) {
          targetId = existingPreview.id;
          return curr;
        }
        // Replace the current preview slot, or append a new one.
        const previewIdx = curr.findIndex(
          (t) => t.kind === "editor" && (t as EditorTab).preview,
        );
        const id = nextIdRef.current++;
        targetId = id;
        const tab: EditorTab = {
          id,
          kind: "editor",
          spaceId: activeSpaceIdRef.current,
          title: basename(path),
          path,
          dirty: false,
          preview: true,
        };
        if (previewIdx === -1) return [...curr, tab];
        const next = [...curr];
        next[previewIdx] = tab;
        return next;
      }
    });
    if (targetId !== null) setActiveId(targetId);
    return targetId as number | null;
  }, []);

  /**
   * Promotes a preview tab to a persistent one. Called on double-click of the
   * tab title in the tab bar. Dirty edits also auto-promote (see `updateTab`).
   */
  const pinTab = useCallback((id: number) => {
    setTabs((curr) =>
      curr.map((t) =>
        t.id === id && t.kind === "editor" ? { ...t, preview: false } : t,
      ),
    );
  }, []);

  const openAiDiffTab = useCallback(
    (input: {
      path: string;
      originalContent: string;
      proposedContent: string;
      approvalId: string;
      isNewFile: boolean;
    }) => {
      let targetId: number | null = null;
      setTabs((curr) => {
        const existing = curr.find(
          (t) => t.kind === "ai-diff" && t.approvalId === input.approvalId,
        );
        if (existing) {
          targetId = existing.id;
          return curr;
        }
        const id = nextIdRef.current++;
        targetId = id;
        const title = `${basename(input.path)} (AI diff)`;
        return [
          ...curr,
          {
            id,
            kind: "ai-diff",
            spaceId: activeSpaceIdRef.current,
            title,
            path: input.path,
            originalContent: input.originalContent,
            proposedContent: input.proposedContent,
            approvalId: input.approvalId,
            status: "pending",
            isNewFile: input.isNewFile,
          },
        ];
      });
      if (targetId !== null) setActiveId(targetId);
      return targetId as number | null;
    },
    [],
  );

  const setAiDiffStatus = useCallback(
    (approvalId: string, status: AiDiffStatus) => {
      setTabs((curr) =>
        curr.map((t) =>
          t.kind === "ai-diff" && t.approvalId === approvalId
            ? { ...t, status }
            : t,
        ),
      );
    },
    [],
  );

  const closeAiDiffTab = useCallback((approvalId: string) => {
    setTabs((curr) => {
      const target = curr.find(
        (t) => t.kind === "ai-diff" && t.approvalId === approvalId,
      );
      if (!target) return curr;
      const fallback = nextActiveInSpace(curr, target.id);
      if (fallback === null) {
        return curr.map((t) =>
          t.kind === "ai-diff" && t.approvalId === approvalId
            ? { ...t, status: "approved" as AiDiffStatus }
            : t,
        );
      }
      const next = curr.filter((t) => t.id !== target.id);
      setActiveId((active) => (target.id === active ? fallback : active));
      return next;
    });
  }, []);

  const newPreviewTab = useCallback((url: string) => {
    const id = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id,
        kind: "preview",
        spaceId: activeSpaceIdRef.current,
        title: titleFromUrl(url),
        url,
      },
    ]);
    setActiveId(id);
    return id;
  }, []);

  const newMarkdownTab = useCallback((path: string) => {
    let targetId: number | null = null;
    setTabs((curr) => {
      const existing = curr.find(
        (t) => t.kind === "markdown" && t.path === path,
      );
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [
        ...curr,
        {
          id,
          kind: "markdown",
          spaceId: activeSpaceIdRef.current,
          title: basename(path),
          path,
        },
      ];
    });
    if (targetId !== null) setActiveId(targetId);
    return targetId;
  }, []);

  const newNotesTab = useCallback((docId?: string, title?: string) => {
    const id = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id,
        kind: "notes",
        spaceId: activeSpaceIdRef.current,
        title: title ?? "Notes",
        docId: docId ?? newDocId("note"),
      } satisfies NotesTab,
    ]);
    setActiveId(id);
    return id;
  }, []);

  const newBoardTab = useCallback((boardId?: string, title?: string) => {
    const id = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id,
        kind: "board",
        spaceId: activeSpaceIdRef.current,
        title: title ?? "Board",
        boardId: boardId ?? newDocId("board"),
      } satisfies BoardTab,
    ]);
    setActiveId(id);
    return id;
  }, []);

  const newTasksTab = useCallback((listId?: string, title?: string) => {
    const id = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id,
        kind: "tasks",
        spaceId: activeSpaceIdRef.current,
        title: title ?? "Tasks",
        listId: listId ?? newDocId("tasks"),
      } satisfies TasksTab,
    ]);
    setActiveId(id);
    return id;
  }, []);

  // The Library is one-per-space, like the orchestration views.
  const openLibraryTab = useCallback(() => {
    let targetId: number | null = null;
    setTabs((curr) => {
      const existing = curr.find(
        (t) =>
          t.kind === "library" && t.spaceId === activeSpaceIdRef.current,
      );
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [
        ...curr,
        {
          id,
          kind: "library",
          spaceId: activeSpaceIdRef.current,
          title: "Library",
        } satisfies LibraryTab,
      ];
    });
    if (targetId !== null) setActiveId(targetId);
    return targetId;
  }, []);

  // One launcher per space; defaults to the space new tabs land in.
  const openLauncherTab = useCallback((spaceId?: string) => {
    const sid = spaceId ?? activeSpaceIdRef.current;
    let targetId: number | null = null;
    setTabs((curr) => {
      const existing = curr.find(
        (t) => t.kind === "launcher" && t.spaceId === sid,
      );
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [
        ...curr,
        {
          id,
          kind: "launcher",
          spaceId: sid,
          title: "Start",
        } satisfies LauncherTab,
      ];
    });
    if (targetId !== null) setActiveId(targetId);
    return targetId;
  }, []);

  // A live remote session discovered on the host gets its own background tab
  // (M2.5 F2 adoption). `seedKey` binds the window's restore key to the fresh
  // leaf BEFORE the pane mounts, so the pty spawn reattaches that tmux window
  // instead of creating a new one. Never steals focus.
  const adoptTerminalTab = useCallback(
    (
      spaceId: string,
      opts: { cwd?: string; title: string; leafKey: string },
      seedKey: (leafId: number, key: string) => void,
    ) => {
      const leafId = nextIdRef.current++;
      const id = nextIdRef.current++;
      seedKey(leafId, opts.leafKey);
      setTabs((curr) => [
        ...curr,
        {
          id,
          kind: "terminal",
          spaceId,
          cold: true,
          title: opts.title,
          ...(opts.cwd !== undefined && { cwd: opts.cwd }),
          paneTree: {
            kind: "leaf",
            id: leafId,
            ...(opts.cwd !== undefined && { cwd: opts.cwd }),
          },
          activeLeafId: leafId,
        } satisfies TerminalTab,
      ]);
      return id;
    },
    [],
  );

  // Orchestration views are one-per-space: focus an existing one if present.
  const openOrchestrationTab = useCallback((view: OrchestrationView) => {
    let targetId: number | null = null;
    setTabs((curr) => {
      const existing = curr.find(
        (t) => t.kind === view && t.spaceId === activeSpaceIdRef.current,
      );
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [
        ...curr,
        {
          id,
          kind: view,
          spaceId: activeSpaceIdRef.current,
          title: ORCHESTRATION_TITLES[view],
        } satisfies OrchestrationTab,
      ];
    });
    if (targetId !== null) setActiveId(targetId);
    return targetId;
  }, []);

  const setMarkdownView = useCallback(
    (id: number, mode: "rendered" | "raw") => {
      setTabs((curr) =>
        curr.map((t) => {
          if (
            t.id !== id ||
            !isMarkdownPath((t as { path?: string }).path ?? "")
          )
            return t;
          if (mode === "raw" && t.kind === "markdown") {
            return {
              ...t,
              kind: "editor" as const,
              dirty: false,
              preview: false,
            };
          }
          if (mode === "rendered" && t.kind === "editor") {
            if (t.dirty) return t;
            return {
              id: t.id,
              kind: "markdown" as const,
              spaceId: t.spaceId,
              cold: t.cold,
              title: t.title,
              path: t.path,
            };
          }
          return t;
        }),
      );
    },
    [],
  );

  const openGitDiffTab = useCallback(
    (input: {
      path: string;
      repoRoot: string;
      mode: "-" | "+";
      originalPath?: string | null;
      title?: string;
    }) => {
      const curr = tabsRef.current;
      const existing = curr.find(
        (t) =>
          t.kind === "git-diff" &&
          t.repoRoot === input.repoRoot &&
          t.path === input.path &&
          t.mode === input.mode,
      );
      const computedTitle =
        input.title ?? `${basename(input.path)} (${input.mode})`;
      const originalPath = input.originalPath ?? null;

      if (existing) {
        const nextTabs = curr.map((t) =>
          t.id === existing.id
            ? { ...t, title: computedTitle, originalPath }
            : t,
        );
        tabsRef.current = nextTabs;
        setTabs(nextTabs);
        setActiveId(existing.id);
        return existing.id;
      }

      const id = nextIdRef.current++;
      const nextTabs = [
        ...curr,
        {
          id,
          kind: "git-diff",
          spaceId: activeSpaceIdRef.current,
          title: computedTitle,
          path: input.path,
          repoRoot: input.repoRoot,
          mode: input.mode,
          originalPath,
        } satisfies GitDiffTab,
      ];
      tabsRef.current = nextTabs;
      setTabs(nextTabs);
      setActiveId(id);
      return id;
    },
    [],
  );

  const openCommitHistoryTab = useCallback(
    (input: { repoRoot: string; branch?: string | null }) => {
      const curr = tabsRef.current;
      const existing = curr.find(
        (t) => t.kind === "git-history" && t.repoRoot === input.repoRoot,
      );
      const title = input.branch ? `History · ${input.branch}` : "Git History";
      if (existing) {
        const nextTabs = curr.map((t) =>
          t.id === existing.id ? { ...t, title } : t,
        );
        tabsRef.current = nextTabs;
        setTabs(nextTabs);
        setActiveId(existing.id);
        return existing.id;
      }
      const id = nextIdRef.current++;
      const nextTabs = [
        ...curr,
        {
          id,
          kind: "git-history",
          spaceId: activeSpaceIdRef.current,
          title,
          repoRoot: input.repoRoot,
        } satisfies GitHistoryTab,
      ];
      tabsRef.current = nextTabs;
      setTabs(nextTabs);
      setActiveId(id);
      return id;
    },
    [],
  );

  const openCommitFileDiffTab = useCallback(
    (input: {
      repoRoot: string;
      sha: string;
      shortSha: string;
      subject: string;
      path: string;
      originalPath: string | null;
    }) => {
      const curr = tabsRef.current;
      const existing = curr.find(
        (t) =>
          t.kind === "git-commit-file" &&
          t.repoRoot === input.repoRoot &&
          t.sha === input.sha &&
          t.path === input.path,
      );
      const title = `${basename(input.path)} @ ${input.shortSha}`;
      if (existing) {
        const nextTabs = curr.map((t) =>
          t.id === existing.id
            ? {
                ...t,
                title,
                subject: input.subject,
                originalPath: input.originalPath,
              }
            : t,
        );
        tabsRef.current = nextTabs;
        setTabs(nextTabs);
        setActiveId(existing.id);
        return existing.id;
      }
      const id = nextIdRef.current++;
      const nextTabs = [
        ...curr,
        {
          id,
          kind: "git-commit-file",
          spaceId: activeSpaceIdRef.current,
          title,
          repoRoot: input.repoRoot,
          sha: input.sha,
          shortSha: input.shortSha,
          subject: input.subject,
          path: input.path,
          originalPath: input.originalPath,
        } satisfies GitCommitFileDiffTab,
      ];
      tabsRef.current = nextTabs;
      setTabs(nextTabs);
      setActiveId(id);
      return id;
    },
    [],
  );

  const closeTab = useCallback((id: number) => {
    let toDispose: number[] = [];
    setTabs((curr) => {
      const fallback = nextActiveInSpace(curr, id);
      if (fallback === null) return curr;
      const target = curr.find((t) => t.id === id);
      if (target?.kind === "terminal") {
        toDispose = leafIds(target.paneTree);
      }
      const next = curr.filter((t) => t.id !== id);
      setActiveId((active) => (id === active ? fallback : active));
      return next;
    });
    for (const lid of toDispose) disposeSession(lid);
  }, []);

  // Opens a fresh terminal tab in the same space as `id`, inheriting its cwd.
  // ponytail: only terminal/editor tabs carry a cwd worth cloning; every other
  // kind (notes/board/preview/diff/…) just gets a plain terminal in its space.
  const duplicateTab = useCallback((id: number) => {
    const tab = tabsRef.current.find((t) => t.id === id);
    if (!tab) return;
    const cwd = tab.kind === "terminal" ? tab.cwd : undefined;
    const newId = newTabInSpace(tab.spaceId, cwd);
    // newTabInSpace appends a cold tab without focus; duplicating is an explicit
    // user action, so follow it like newTab does.
    setActiveId(newId);
  }, [newTabInSpace]);

  // Closes every other tab sharing this tab's space, leaving `id` open. Routes
  // each through closeTab so active-tab bookkeeping (and PTY disposal) stays
  // correct; nextActiveInSpace naturally refuses the final tab of the space.
  const closeOthersInSpace = useCallback(
    (id: number) => {
      const curr = tabsRef.current;
      const keep = curr.find((t) => t.id === id);
      if (!keep) return;
      const others = curr.filter(
        (t) => t.id !== id && t.spaceId === keep.spaceId,
      );
      for (const t of others) closeTab(t.id);
    },
    [closeTab],
  );

  const updateTab = useCallback((id: number, patch: TabPatch) => {
    setTabs((t) =>
      t.map((x) => {
        if (x.id !== id) return x;
        if (x.kind === "terminal") {
          return {
            ...x,
            ...(patch.title !== undefined && { title: patch.title }),
            ...(patch.cwd !== undefined && { cwd: patch.cwd }),
            ...(patch.customTitle !== undefined && {
              customTitle:
                patch.customTitle === "" ? undefined : patch.customTitle,
            }),
          };
        }
        if (x.kind === "preview") {
          return {
            ...x,
            ...(patch.title !== undefined && { title: patch.title }),
            ...(patch.url !== undefined && {
              url: patch.url,
              title: patch.title ?? titleFromUrl(patch.url),
            }),
          };
        }
        if (x.kind === "markdown") {
          return {
            ...x,
            ...(patch.title !== undefined && { title: patch.title }),
          };
        }
        if (x.kind === "notes" || x.kind === "board" || x.kind === "tasks") {
          // These derive their label from `title`; rename routes through the
          // customTitle patch (shared TabBar path), an empty value resets it.
          const fallbackTitle =
            x.kind === "notes" ? "Notes" : x.kind === "board" ? "Board" : "Tasks";
          const next =
            patch.customTitle !== undefined
              ? patch.customTitle.trim() || fallbackTitle
              : patch.title;
          return { ...x, ...(next !== undefined && { title: next }) };
        }
        if (
          x.kind === "agent-topology" ||
          x.kind === "message-flow" ||
          x.kind === "director" ||
          x.kind === "library" ||
          x.kind === "launcher"
        ) {
          return x;
        }
        // editor tab: auto-promote from preview the moment the file becomes dirty.
        const autoPin =
          patch.dirty === true && (x as EditorTab).preview
            ? { preview: false }
            : {};
        return {
          ...x,
          ...autoPin,
          ...(patch.title !== undefined && { title: patch.title }),
          ...(patch.dirty !== undefined && { dirty: patch.dirty }),
          ...(patch.path !== undefined && { path: patch.path }),
        };
      }),
    );
  }, []);

  const selectByIndex = useCallback(
    (idx: number) => {
      const t = tabs[idx];
      if (t) setActiveId(t.id);
    },
    [tabs],
  );

  /** Update a leaf's cwd; mirror to the tab's `cwd` when the leaf is active.
   * Bails out without setTabs when nothing actually changed — shell integration
   * re-emits OSC 7 on every prompt, including empty Enters, so this fires at
   * keystroke rate. Always-setTabs there cascades a paneTree re-render across
   * every open tab. */
  const setLeafCwd = useCallback((leafId: number, cwd: string) => {
    setTabs((curr) => {
      let changed = false;
      const next = curr.map((t) => {
        if (t.kind !== "terminal" || !hasLeaf(t.paneTree, leafId)) return t;
        const paneTree = setLeafCwdInTree(t.paneTree, leafId, cwd);
        const isActive = t.activeLeafId === leafId;
        const cwdChanged = isActive && t.cwd !== cwd;
        if (paneTree === t.paneTree && !cwdChanged) return t;
        changed = true;
        return { ...t, paneTree, ...(cwdChanged && { cwd }) };
      });
      return changed ? next : curr;
    });
  }, []);

  const focusPane = useCallback((tabId: number, leafId: number) => {
    setTabs((curr) =>
      curr.map((t) => {
        if (t.id !== tabId || t.kind !== "terminal") return t;
        if (!hasLeaf(t.paneTree, leafId)) return t;
        if (t.activeLeafId === leafId) return t;
        const cwd = findLeafCwd(t.paneTree, leafId);
        return {
          ...t,
          activeLeafId: leafId,
          ...(cwd !== undefined && { cwd }),
        };
      }),
    );
  }, []);

  const focusNextPaneInTab = useCallback((tabId: number, delta: 1 | -1) => {
    setTabs((curr) =>
      curr.map((t) => {
        if (t.id !== tabId || t.kind !== "terminal") return t;
        const next = nextLeafId(t.paneTree, t.activeLeafId, delta);
        if (next === t.activeLeafId) return t;
        const cwd = findLeafCwd(t.paneTree, next);
        return { ...t, activeLeafId: next, ...(cwd !== undefined && { cwd }) };
      }),
    );
  }, []);

  /** Split the active leaf of `tabId` along `dir`. `before` places the new leaf
   * on the leading side (Left for row, Top for col). Returns the new leaf id. */
  const splitActivePane = useCallback(
    (tabId: number, dir: SplitDir, before = false): number | null => {
      let newLeafId: number | null = null;
      setTabs((curr) =>
        curr.map((t) => {
          if (t.id !== tabId || t.kind !== "terminal" || t.blocks) return t;
          // Cap terminals by the renderer pool; notes don't count toward it.
          if (countTerminalLeaves(t.paneTree) >= MAX_PANES_PER_TAB) return t;
          if (leafIds(t.paneTree).length >= MAX_TOTAL_PANES_PER_TAB) return t;
          const splitId = nextIdRef.current++;
          const leafId = nextIdRef.current++;
          newLeafId = leafId;
          const paneTree = splitLeaf(
            t.paneTree,
            t.activeLeafId,
            splitId,
            leafId,
            dir,
            t.cwd,
            before,
          );
          return { ...t, paneTree, activeLeafId: leafId };
        }),
      );
      return newLeafId;
    },
    [],
  );

  /**
   * Re-dock an existing pane beside another within the SAME tab (drag a pane's
   * header onto another pane's edge). Mirrors `splitActivePane`, but reuses the
   * moving leaf object so its id survives — which is why we must NOT call
   * `disposeSession` here (unlike `closePaneByLeaf`): the PTY has to stay alive.
   * Focus follows the moved pane.
   */
  const movePane = useCallback(
    (
      tabId: number,
      sourceLeafId: number,
      targetLeafId: number,
      side: SplitSide,
    ): void => {
      setTabs((curr) =>
        curr.map((t) => {
          if (t.id !== tabId || t.kind !== "terminal" || t.blocks) return t;
          if (
            !hasLeaf(t.paneTree, sourceLeafId) ||
            !hasLeaf(t.paneTree, targetLeafId)
          )
            return t;
          const splitId = nextIdRef.current++;
          const paneTree = moveLeafBeside(
            t.paneTree,
            sourceLeafId,
            targetLeafId,
            splitId,
            side,
          );
          // Identity-equal => the move was a no-op (self-drop, etc.).
          if (paneTree === t.paneTree) return t;
          return { ...t, paneTree, activeLeafId: sourceLeafId };
        }),
      );
    },
    [],
  );

  /**
   * Adds a docs-backed note pane beside the active leaf of `tabId`, so notes
   * live alongside terminals in the same tab. Returns the new leaf id and the
   * note's docId, or null if the tab can't take another pane.
   */
  const addNotePane = useCallback(
    (
      tabId: number,
      dir: SplitDir = "row",
      before = false,
    ): { leafId: number; docId: string } | null => {
      let result: { leafId: number; docId: string } | null = null;
      setTabs((curr) =>
        curr.map((t) => {
          if (t.id !== tabId || t.kind !== "terminal" || t.blocks) return t;
          if (leafIds(t.paneTree).length >= MAX_TOTAL_PANES_PER_TAB) return t;
          const splitId = nextIdRef.current++;
          const leafId = nextIdRef.current++;
          const docId = newDocId("note");
          result = { leafId, docId };
          const paneTree = splitLeafNote(
            t.paneTree,
            t.activeLeafId,
            splitId,
            leafId,
            dir,
            docId,
            before,
          );
          return { ...t, paneTree, activeLeafId: leafId };
        }),
      );
      return result;
    },
    [],
  );

  /**
   * Adds a docs-backed tasks pane beside the active leaf of `tabId`, so a
   * checklist lives alongside terminals in the same tab (mirror of addNotePane).
   * Returns the new leaf id and the list's id, or null if the tab can't take
   * another pane.
   */
  const addTasksPane = useCallback(
    (
      tabId: number,
      dir: SplitDir = "row",
      before = false,
    ): { leafId: number; listId: string } | null => {
      let result: { leafId: number; listId: string } | null = null;
      setTabs((curr) =>
        curr.map((t) => {
          if (t.id !== tabId || t.kind !== "terminal" || t.blocks) return t;
          if (leafIds(t.paneTree).length >= MAX_TOTAL_PANES_PER_TAB) return t;
          const splitId = nextIdRef.current++;
          const leafId = nextIdRef.current++;
          const listId = newDocId("tasks");
          result = { leafId, listId };
          const paneTree = splitLeafTasks(
            t.paneTree,
            t.activeLeafId,
            splitId,
            leafId,
            dir,
            listId,
            before,
          );
          return { ...t, paneTree, activeLeafId: leafId };
        }),
      );
      return result;
    },
    [],
  );

  const closePaneByLeaf = useCallback((leafId: number): void => {
    let didRemove = false;
    setTabs((curr) => {
      const tab = curr.find(
        (t) => t.kind === "terminal" && hasLeaf(t.paneTree, leafId),
      );
      if (tab?.kind !== "terminal") return curr;
      const newTree = removeLeaf(tab.paneTree, leafId);
      if (newTree === null) {
        const fallback = nextActiveInSpace(curr, tab.id);
        if (fallback === null) return curr;
        const next = curr.filter((x) => x.id !== tab.id);
        setActiveId((active) => (active === tab.id ? fallback : active));
        didRemove = true;
        return next;
      }
      const remaining = leafIds(newTree);
      let newActive = tab.activeLeafId;
      if (tab.activeLeafId === leafId) {
        const sib = siblingLeafOf(tab.paneTree, leafId);
        newActive = sib && remaining.includes(sib) ? sib : remaining[0];
      }
      didRemove = true;
      return curr.map((x) =>
        x.id === tab.id
          ? { ...x, paneTree: newTree, activeLeafId: newActive }
          : x,
      );
    });
    if (didRemove) disposeSession(leafId);
  }, []);

  const closeActivePane = useCallback((tabId: number): boolean => {
    let closedTab = false;
    let removedLeaf: number | null = null;
    setTabs((curr) => {
      const t = curr.find((x) => x.id === tabId);
      if (t?.kind !== "terminal") return curr;
      const target = t.activeLeafId;
      const newTree = removeLeaf(t.paneTree, target);
      if (newTree === null) {
        const fallback = nextActiveInSpace(curr, tabId);
        if (fallback === null) return curr;
        const next = curr.filter((x) => x.id !== tabId);
        setActiveId((active) => (active === tabId ? fallback : active));
        closedTab = true;
        removedLeaf = target;
        return next;
      }
      const remaining = leafIds(newTree);
      const sib = siblingLeafOf(t.paneTree, target);
      const newActive = sib && remaining.includes(sib) ? sib : remaining[0];
      removedLeaf = target;
      return curr.map((x) =>
        x.id === tabId
          ? { ...x, paneTree: newTree, activeLeafId: newActive }
          : x,
      );
    });
    if (removedLeaf !== null) disposeSession(removedLeaf);
    return closedTab;
  }, []);

  return {
    tabs,
    activeId,
    setActiveId,
    allocId,
    replaceTabs,
    moveTabToSpace,
    reorderTab,
    newTabInSpace,
    removeTabsForSpace,
    markBooted,
    setActiveSpaceForNewTabs,
    newTab,
    newBlockTab,
    newAgentTab,
    newGridTab,
    newPrivateTab,
    openFileTab,
    pinTab,
    newPreviewTab,
    newMarkdownTab,
    newNotesTab,
    newBoardTab,
    newTasksTab,
    openLibraryTab,
    openLauncherTab,
    adoptTerminalTab,
    openOrchestrationTab,
    setMarkdownView,
    openAiDiffTab,
    openGitDiffTab,
    openCommitHistoryTab,
    openCommitFileDiffTab,
    setAiDiffStatus,
    closeAiDiffTab,
    closeTab,
    duplicateTab,
    closeOthersInSpace,
    updateTab,
    selectByIndex,
    setLeafCwd,
    focusPane,
    focusNextPaneInTab,
    splitActivePane,
    movePane,
    addNotePane,
    addTasksPane,
    closeActivePane,
    closePaneByLeaf,
  };
}
