import type { SpaceState } from "@/modules/spaces/lib/store";
import type { Tab } from "@/modules/tabs/lib/useTabs";
import { describe, expect, it } from "vitest";
import {
  docsInRemoteState,
  liveTabIdentity,
  planLiveDocAdoption,
  planLiveRenames,
} from "./liveAdopt";

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

function liveTerminal(
  id: number,
  spaceId: string,
  leafId: number,
  customTitle?: string,
): Tab {
  return {
    id,
    spaceId,
    kind: "terminal",
    title: "shell",
    activeLeafId: leafId,
    paneTree: { kind: "leaf", id: leafId },
    ...(customTitle !== undefined && { customTitle }),
  } as Tab;
}

const keys: Record<number, string> = { 1: "k1", 2: "k2" };
const leafKey = (leafId: number) => keys[leafId];

describe("liveTabIdentity", () => {
  it("matches tabClocks.tabIdentity for terminals and docs", () => {
    expect(liveTabIdentity(liveTerminal(1, "s", 1), leafKey)).toBe("t:k1");
    expect(liveTabIdentity(liveTerminal(1, "s", 9), leafKey)).toBeNull();
    expect(liveTabIdentity(noteTab(2, "s", "d", "N"), leafKey)).toBe("n:d");
  });
});

describe("planLiveRenames (ADR-025)", () => {
  const onDisk: SpaceState = {
    activeTabIndex: 0,
    tabs: [
      {
        kind: "terminal",
        tree: { kind: "leaf", key: "k1" },
        customTitle: "old",
      },
      { kind: "notes", docId: "d", title: "Notes" },
    ],
  };
  const diskMeta = { at: 10, tabs: { "t:k1": 10, "n:d": 10 } };
  const live = [liveTerminal(1, "s", 1, "old"), noteTab(2, "s", "d", "Notes")];

  it("renames tabs whose remote clock beats the local one, labels only", () => {
    const remote: SpaceState = {
      activeTabIndex: 0,
      tabs: [
        {
          kind: "terminal",
          tree: { kind: "leaf", key: "k1" },
          customTitle: "123",
        },
        { kind: "notes", docId: "d", title: "Plan" },
      ],
    };
    const plan = planLiveRenames(
      "s",
      live,
      onDisk,
      diskMeta,
      remote,
      { at: 20, tabs: { "t:k1": 20, "n:d": 20 } },
      leafKey,
    );
    expect(plan).toEqual([
      {
        tabId: 1,
        identity: "t:k1",
        kind: "terminal",
        title: "123",
        clock: 20,
        before: "old",
      },
      {
        tabId: 2,
        identity: "n:d",
        kind: "doc",
        title: "Plan",
        clock: 20,
        before: "Notes",
      },
    ]);
  });

  it("ignores older or equal remote clocks, unknown tabs, and tabs not on disk", () => {
    const remote: SpaceState = {
      activeTabIndex: 0,
      tabs: [
        {
          kind: "terminal",
          tree: { kind: "leaf", key: "k1" },
          customTitle: "stale",
        },
        {
          kind: "terminal",
          tree: { kind: "leaf", key: "k2" },
          customTitle: "fresh",
        },
      ],
    };
    const plan = planLiveRenames(
      "s",
      [...live, liveTerminal(3, "s", 2, "unsaved")],
      onDisk,
      diskMeta,
      remote,
      { at: 10, tabs: { "t:k1": 10, "t:k2": 99 } },
      leafKey,
    );
    expect(plan).toEqual([]);
  });

  it("a cleared remote title clears the local one when newer", () => {
    const remote: SpaceState = {
      activeTabIndex: 0,
      tabs: [{ kind: "terminal", tree: { kind: "leaf", key: "k1" } }],
    };
    const plan = planLiveRenames(
      "s",
      live,
      onDisk,
      diskMeta,
      remote,
      {
        at: 30,
        tabs: { "t:k1": 30 },
      },
      leafKey,
    );
    expect(plan).toEqual([
      {
        tabId: 1,
        identity: "t:k1",
        kind: "terminal",
        title: "",
        clock: 30,
        before: "old",
      },
    ]);
  });
});
