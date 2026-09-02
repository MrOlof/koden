import type { SpaceState } from "@/modules/spaces/lib/store";
import type { Tab } from "@/modules/tabs/lib/useTabs";
import { describe, expect, it } from "vitest";
import { docsInRemoteState, planLiveDocAdoption } from "./liveAdopt";

const remote: SpaceState = {
  activeTabIndex: 0,
  tabs: [
    { kind: "notes", docId: "n-tab", title: "Meeting notes" },
    { kind: "tasks", listId: "t-1", title: "Sprint" },
    {
      kind: "terminal",
      tree: {
        kind: "split",
        dir: "row",
        children: [
          { kind: "leaf", cwd: "/home/k" },
          { kind: "leaf", content: "note", docId: "n-pane", title: "Scratch" },
          { kind: "leaf", content: "tasks", docId: "t-pane" },
        ],
      },
    },
  ],
};

describe("docsInRemoteState", () => {
  it("collects doc tabs and doc leaves inside terminal trees, with defaults", () => {
    expect(docsInRemoteState(remote)).toEqual([
      { kind: "notes", id: "n-tab", title: "Meeting notes" },
      { kind: "tasks", id: "t-1", title: "Sprint" },
      { kind: "notes", id: "n-pane", title: "Scratch" },
      { kind: "tasks", id: "t-pane", title: "Tasks" },
    ]);
  });
});

function noteTab(
  id: number,
  spaceId: string,
  docId: string,
  title: string,
): Tab {
  return { id, spaceId, kind: "notes", docId, title } as Tab;
}

function tasksTab(
  id: number,
  spaceId: string,
  listId: string,
  title: string,
): Tab {
  return { id, spaceId, kind: "tasks", listId, title } as Tab;
}

function terminalTabWithPaneNote(
  id: number,
  spaceId: string,
  docId: string,
): Tab {
  return {
    id,
    spaceId,
    kind: "terminal",
    title: "shell",
    activeLeafId: 1,
    paneTree: {
      kind: "split",
      id: 9,
      dir: "row",
      children: [
        { kind: "leaf", id: 1 },
        { kind: "leaf", id: 2, content: "note", docId },
      ],
    },
  } as Tab;
}

describe("planLiveDocAdoption", () => {
  it("creates every remote doc missing locally, once", () => {
    const plan = planLiveDocAdoption("sp", [], remote);
    expect(plan.create.map((d) => d.id)).toEqual([
      "n-tab",
      "t-1",
      "n-pane",
      "t-pane",
    ]);
    expect(plan.rename).toEqual([]);
  });

  it("skips docs present as tabs OR as panes, and ignores other spaces", () => {
    const local = [
      noteTab(1, "sp", "n-tab", "Meeting notes"),
      terminalTabWithPaneNote(2, "sp", "n-pane"),
      noteTab(3, "other-space", "t-1", "Sprint"),
    ];
    const plan = planLiveDocAdoption("sp", local, remote);
    expect(plan.create.map((d) => d.id)).toEqual(["t-1", "t-pane"]);
  });

  it("renames doc TABS to the remote title but never pane-backed docs", () => {
    const local = [
      noteTab(1, "sp", "n-tab", "Old name"),
      terminalTabWithPaneNote(2, "sp", "n-pane"), // remote title "Scratch" ≠ pane: untouched
    ];
    const plan = planLiveDocAdoption("sp", local, remote);
    expect(plan.rename).toEqual([{ tabId: 1, title: "Meeting notes" }]);
  });

  it("is idempotent: a fully-adopted space plans nothing", () => {
    const local = [
      noteTab(1, "sp", "n-tab", "Meeting notes"),
      tasksTab(2, "sp", "t-1", "Sprint"),
      noteTab(3, "sp", "n-pane", "Scratch"),
      tasksTab(4, "sp", "t-pane", "Tasks"),
    ];
    const plan = planLiveDocAdoption("sp", local, remote);
    expect(plan.create).toEqual([]);
    expect(plan.rename).toEqual([]);
  });
});
