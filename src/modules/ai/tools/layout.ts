import { isLeaf, type PaneNode } from "@/modules/terminal/lib/panes";
import { tool } from "ai";
import { z } from "zod";
import type {
  LayoutSnapshot,
  LayoutSplitKind,
  LayoutSplitSide,
  ToolContext,
} from "./context";
import { resolvePath } from "./context";

// Layout tools: the Librarian builds workspace layouts on request — open tabs,
// split the active pane, re-focus, read the tree. Deliberately NO approval
// gate: every action is immediately visible in the UI, reversible with one
// click, and non-destructive (nothing is closed, deleted, or overwritten).
// For the same reason there are NO close/delete tools in v1 — create/arrange
// only (ADR-017 addendum). Sequential split calls compose layouts because
// focus follows each new pane.

/** Pane kinds that can live in a split (recon truth: PaneTreeView's
 * SplitPaneType / PaneNode leaf `content`). Everything else is tab-only. */
export const SPLIT_KINDS = ["terminal", "note", "tasks"] as const;

/** Model-facing 'up'/'down' → the pane model's top/bottom SplitSide. */
export function sideForDirection(
  direction: "left" | "right" | "up" | "down",
): LayoutSplitSide {
  if (direction === "up") return "top";
  if (direction === "down") return "bottom";
  return direction;
}

/** Case-insensitive; accepts the plural alias 'notes'. Unknown kinds → null —
 * the caller must error listing SPLIT_KINDS, never silently substitute. */
export function normalizeSplitKind(kind: string): LayoutSplitKind | null {
  const k = kind.trim().toLowerCase();
  if (k === "notes") return "note";
  return (SPLIT_KINDS as readonly string[]).includes(k)
    ? (k as LayoutSplitKind)
    : null;
}

export type SerializedPane =
  | {
      type: "pane";
      paneId: number;
      kind: LayoutSplitKind;
      title?: string;
      cwd?: string;
      focused: boolean;
    }
  | { type: "split"; direction: "row" | "col"; children: SerializedPane[] };

/** Pane tree → model-readable shape: kinds, ids, titles, focus. */
export function serializePaneTree(
  node: PaneNode,
  activeLeafId: number,
  paneTitles: Record<number, string>,
): SerializedPane {
  if (isLeaf(node)) {
    const title = paneTitles[node.id]?.trim();
    return {
      type: "pane",
      paneId: node.id,
      kind: node.content ?? "terminal",
      ...(title ? { title } : {}),
      ...(node.cwd ? { cwd: node.cwd } : {}),
      focused: node.id === activeLeafId,
    };
  }
  return {
    type: "split",
    direction: node.dir,
    children: node.children.map((c) =>
      serializePaneTree(c, activeLeafId, paneTitles),
    ),
  };
}

function shapeLayout(snap: LayoutSnapshot) {
  const active = snap.tabs.find((t) => t.tabId === snap.activeTabId) ?? null;
  return {
    space: snap.space,
    activeTab: active
      ? {
          tabId: active.tabId,
          kind: active.kind,
          title: active.title,
          panes:
            active.paneTree && active.activeLeafId !== undefined
              ? serializePaneTree(
                  active.paneTree,
                  active.activeLeafId,
                  snap.paneTitles,
                )
              : null,
        }
      : null,
    tabs: snap.tabs.map((t) => ({
      tabId: t.tabId,
      kind: t.kind,
      title: t.title,
      active: t.active,
    })),
  };
}

export function buildLayoutTools(ctx: ToolContext) {
  return {
    workspace_open_tab: tool({
      description:
        "Open a workspace tab. kind: 'terminal' (new shell), 'notes' | 'board' | 'tasks' (new docs-backed tab), 'editor' (requires path), 'library' | 'brain' (singletons — focuses the existing tab when already open). The new tab becomes active, so a workspace_split_pane right after targets it. Returns the tab id and whether it was opened or focused. Auto-executes (visible, reversible, nothing destroyed).",
      inputSchema: z.object({
        kind: z.enum([
          "terminal",
          "notes",
          "board",
          "tasks",
          "editor",
          "library",
          "brain",
        ]),
        title: z
          .string()
          .optional()
          .describe("Tab title (terminal/notes/board/tasks only)."),
        path: z
          .string()
          .optional()
          .describe("File to open — required for kind 'editor', else ignored."),
      }),
      execute: async ({ kind, title, path }) => {
        if (kind === "editor") {
          if (!path?.trim())
            return { error: "kind 'editor' needs a path to open" };
          try {
            return ctx.openWorkspaceTab(kind, {
              title,
              path: resolvePath(path.trim(), ctx.getCwd()),
            });
          } catch (e) {
            return { error: e instanceof Error ? e.message : String(e) };
          }
        }
        return ctx.openWorkspaceTab(kind, { title });
      },
    }),

    workspace_split_pane: tool({
      description:
        "Split the focused pane of the active tab, placing a new pane beside it. kind: 'terminal' | 'note' | 'tasks' — the only kinds that can live in a split (board/editor/library/brain are tab-only; use workspace_open_tab). direction = where the NEW pane lands relative to the focused one. The new pane takes focus, so sequential calls compose layouts: split 'tasks' right then 'note' down → tasks top-right, notes bottom-right. Only terminal-kind tabs hold splits. Auto-executes (visible, reversible).",
      inputSchema: z.object({
        kind: z.string().describe("'terminal' | 'note' | 'tasks'"),
        direction: z.enum(["left", "right", "up", "down"]),
        title: z.string().optional().describe("Pane title."),
      }),
      execute: async ({ kind, direction, title }) => {
        const k = normalizeSplitKind(kind);
        if (!k)
          return {
            error: `pane kind '${kind}' can't live in a split. Splittable kinds: ${SPLIT_KINDS.join(", ")}. Board/editor/library/brain exist only as tabs — open those with workspace_open_tab.`,
          };
        return ctx.splitWorkspacePane(k, sideForDirection(direction), title);
      },
    }),

    workspace_focus_pane: tool({
      description:
        "Focus a pane by id (from workspace_layout_state or a previous split), activating its tab. The next workspace_split_pane targets it — use this to re-target multi-step layout builds. Auto-executes.",
      inputSchema: z.object({
        paneId: z.number().int().describe("Pane (leaf) id."),
      }),
      execute: async ({ paneId }) => ctx.focusWorkspacePane(paneId),
    }),

    workspace_layout_state: tool({
      description:
        "Read the current workspace layout: the active space (name + id), the active tab's pane tree (pane ids, kinds, titles, focus), plus the open tabs of that space. Call before building or extending a layout. Auto-executes.",
      inputSchema: z.object({}),
      execute: async () => shapeLayout(ctx.getWorkspaceLayout()),
    }),
  } as const;
}
