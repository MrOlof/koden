import {
  type PaneNode,
  sideToSplit,
  splitLeafNote,
  splitLeafTasks,
} from "@/modules/terminal/lib/panes";
import { describe, expect, it } from "vitest";
import type { ToolContext } from "./context";
import {
  buildLayoutTools,
  normalizeSplitKind,
  serializePaneTree,
  sideForDirection,
} from "./layout";

// Why this seam: useTabs is a React hook (no renderHook/testing-library dep in
// the repo), so its split fns aren't store-level testable. But they delegate
// 1:1 to the pure pane primitives (splitLeaf/splitLeafNote/splitLeafTasks via
// sideToSplit) and then move focus to the new leaf — which is exactly the
// contract the layout tools rely on for sequential composition. The scripted
// test below replays the owner's ask at that seam, chaining activeLeaf the
// same way useTabs does.

const CALL_OPTS = { toolCallId: "t", messages: [] };

describe("sideForDirection", () => {
  it("maps model-facing directions onto SplitSide", () => {
    expect(sideForDirection("left")).toBe("left");
    expect(sideForDirection("right")).toBe("right");
    expect(sideForDirection("up")).toBe("top");
    expect(sideForDirection("down")).toBe("bottom");
  });
});

describe("normalizeSplitKind", () => {
  it("accepts the split-capable kinds, case-insensitive", () => {
    expect(normalizeSplitKind("terminal")).toBe("terminal");
    expect(normalizeSplitKind("Note")).toBe("note");
    expect(normalizeSplitKind("TASKS")).toBe("tasks");
  });
  it("aliases the plural 'notes' to 'note'", () => {
    expect(normalizeSplitKind("notes")).toBe("note");
  });
  it("rejects tab-only kinds instead of substituting", () => {
    expect(normalizeSplitKind("board")).toBeNull();
    expect(normalizeSplitKind("library")).toBeNull();
    expect(normalizeSplitKind("editor")).toBeNull();
  });
});

describe("scripted layout build (the owner's ask)", () => {
  it("open terminal → split tasks right → split note down = terminal left, tasks top-right, notes bottom-right", () => {
    // workspace_open_tab { kind: 'terminal' } → one terminal leaf, focused.
    let tree: PaneNode = { kind: "leaf", id: 1 };
    let activeLeaf = 1;
    let nextId = 2;

    // workspace_split_pane { kind: 'tasks', direction: 'right' }
    {
      const { dir, before } = sideToSplit(sideForDirection("right"));
      const splitId = nextId++;
      const leafId = nextId++;
      tree = splitLeafTasks(
        tree,
        activeLeaf,
        splitId,
        leafId,
        dir,
        "t1",
        before,
      );
      activeLeaf = leafId; // focus follows the new pane (addTasksPane contract)
    }

    // workspace_split_pane { kind: 'note', direction: 'down' } — targets the
    // tasks pane because focus moved there.
    {
      const { dir, before } = sideToSplit(sideForDirection("down"));
      const splitId = nextId++;
      const leafId = nextId++;
      tree = splitLeafNote(
        tree,
        activeLeaf,
        splitId,
        leafId,
        dir,
        "n1",
        before,
      );
      activeLeaf = leafId;
    }

    expect(
      serializePaneTree(tree, activeLeaf, { 3: "Tasks", 5: "Notes" }),
    ).toEqual({
      type: "split",
      direction: "row",
      children: [
        { type: "pane", paneId: 1, kind: "terminal", focused: false },
        {
          type: "split",
          direction: "col",
          children: [
            {
              type: "pane",
              paneId: 3,
              kind: "tasks",
              title: "Tasks",
              focused: false,
            },
            {
              type: "pane",
              paneId: 5,
              kind: "note",
              title: "Notes",
              focused: true,
            },
          ],
        },
      ],
    });
  });
});

describe("workspace_split_pane tool", () => {
  const calls: unknown[][] = [];
  const ctx = {
    getCwd: () => null,
    splitWorkspacePane: (...args: unknown[]) => {
      calls.push(args);
      return { tabId: 1, paneId: 9 };
    },
  } as unknown as ToolContext;
  const tools = buildLayoutTools(ctx);

  it("errors on non-split kinds, naming the supported set", async () => {
    const res = await tools.workspace_split_pane.execute?.(
      { kind: "board", direction: "right" },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      error: expect.stringContaining("terminal, note, tasks"),
    });
    expect(calls).toHaveLength(0); // never silently substitutes
  });

  it("normalizes 'notes' and maps 'down' to the bottom side", async () => {
    const res = await tools.workspace_split_pane.execute?.(
      { kind: "notes", direction: "down" },
      CALL_OPTS,
    );
    expect(res).toEqual({ tabId: 1, paneId: 9 });
    expect(calls).toEqual([["note", "bottom", undefined]]);
  });
});

describe("workspace_open_tab tool", () => {
  it("requires a path for kind 'editor'", async () => {
    const ctx = {
      getCwd: () => null,
      openWorkspaceTab: () => ({ tabId: 1, action: "opened", title: "x" }),
    } as unknown as ToolContext;
    const tools = buildLayoutTools(ctx);
    const res = await tools.workspace_open_tab.execute?.(
      { kind: "editor" },
      CALL_OPTS,
    );
    expect(res).toMatchObject({ error: expect.stringContaining("path") });
  });
});
