import type { Tab } from "@/modules/tabs";
import { describe, expect, it } from "vitest";
import { groupRootsByTab, statusCounts } from "./grouping";
import { defaultConfigForRole } from "./roles";
import type { Agent, AgentRole, AgentStatus } from "./types";

let seq = 0;
function agent(
  id: string,
  tabId: number | null,
  status: AgentStatus = "working",
  role: AgentRole = "director",
): Agent {
  seq += 1;
  return {
    id,
    name: id,
    role,
    status,
    task: null,
    config: defaultConfigForRole(role),
    tokens: { input: 0, output: 0 },
    cost: 0,
    parentId: null,
    leafId: null,
    tabId,
    createdAt: seq,
    lastActivityAt: seq,
  };
}

// labelFor only reads kind/title/customTitle/cwd; a minimal terminal tab is
// enough to exercise grouping titles without standing up a full pane tree.
function tab(id: number, title: string): Tab {
  return {
    id,
    kind: "terminal",
    title,
    customTitle: title,
    spaceId: "s",
    paneTree: { kind: "leaf", id: 0 },
    activeLeafId: 0,
  } as unknown as Tab;
}

describe("groupRootsByTab", () => {
  it("groups roots across two tabs in tabs-array order", () => {
    const tabs = [tab(1, "Alpha"), tab(2, "Beta")];
    const roots = [agent("a1", 1), agent("b1", 2), agent("a2", 1)];

    const groups = groupRootsByTab(roots, tabs);

    expect(groups.map((g) => g.title)).toEqual(["Alpha", "Beta"]);
    expect(groups[0].tabId).toBe(1);
    expect(groups[0].agents.map((a) => a.id)).toEqual(["a1", "a2"]);
    expect(groups[1].agents.map((a) => a.id)).toEqual(["b1"]);
  });

  it("follows tabs-array order even when roots arrive in another order", () => {
    // Tabs declare Beta before Alpha; group emission must honor that.
    const tabs = [tab(2, "Beta"), tab(1, "Alpha")];
    const roots = [agent("a1", 1), agent("b1", 2)];

    const groups = groupRootsByTab(roots, tabs);

    expect(groups.map((g) => g.title)).toEqual(["Beta", "Alpha"]);
  });

  it("buckets null and unknown tabId roots into a trailing Other group", () => {
    const tabs = [tab(1, "Alpha")];
    const roots = [
      agent("a1", 1),
      agent("orphan", null),
      agent("ghost", 99), // tabId not present in tabs
    ];

    const groups = groupRootsByTab(roots, tabs);

    expect(groups.map((g) => g.title)).toEqual(["Alpha", "Other"]);
    const other = groups[groups.length - 1];
    expect(other.tabId).toBeNull();
    expect(other.agents.map((a) => a.id)).toEqual(["orphan", "ghost"]);
  });

  it("preserves the incoming root order within a group (no re-sort)", () => {
    const tabs = [tab(1, "Alpha")];
    // Intentionally not status- or activity-sorted: order must survive verbatim.
    const roots = [
      agent("z", 1, "done"),
      agent("m", 1, "idle"),
      agent("a", 1, "working"),
    ];

    const groups = groupRootsByTab(roots, tabs);

    expect(groups[0].agents.map((a) => a.id)).toEqual(["z", "m", "a"]);
  });

  it("omits tabs that have no roots", () => {
    const tabs = [tab(1, "Alpha"), tab(2, "Beta"), tab(3, "Gamma")];
    const roots = [agent("a1", 1), agent("g1", 3)];

    const groups = groupRootsByTab(roots, tabs);

    expect(groups.map((g) => g.title)).toEqual(["Alpha", "Gamma"]);
  });
});

describe("statusCounts", () => {
  it("tallies each status correctly", () => {
    const agents = [
      agent("a", 1, "working"),
      agent("b", 1, "working"),
      agent("c", 1, "done"),
      agent("d", 1, "waiting"),
    ];

    expect(statusCounts(agents)).toEqual({ working: 2, done: 1, waiting: 1 });
  });

  it("returns an empty object for no agents", () => {
    expect(statusCounts([])).toEqual({});
  });
});
