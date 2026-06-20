import { describe, expect, it } from "vitest";
import { defaultConfigForRole } from "./roles";
import {
  countActive,
  deriveEdges,
  isActiveStatus,
  sortAgentsForDock,
  totalTokens,
} from "./topology";
import type { Agent, AgentRole, AgentStatus, FlowEvent } from "./types";

let seq = 0;
function agent(
  id: string,
  role: AgentRole,
  status: AgentStatus,
  parentId: string | null = null,
  tokens = { input: 0, output: 0 },
): Agent {
  seq += 1;
  return {
    id,
    name: id,
    role,
    status,
    task: null,
    config: defaultConfigForRole(role),
    tokens,
    cost: 0,
    parentId,
    leafId: null,
    tabId: null,
    createdAt: seq,
    lastActivityAt: seq,
  };
}

function flow(fromId: string, toId: string | null): FlowEvent {
  seq += 1;
  return {
    id: `fl-${seq}`,
    ts: seq,
    kind: "message",
    fromId,
    toId,
    summary: "",
  };
}

describe("isActiveStatus", () => {
  it("treats working/reviewing/waiting/spawning as active", () => {
    for (const s of ["spawning", "working", "reviewing", "waiting"] as const) {
      expect(isActiveStatus(s)).toBe(true);
    }
  });
  it("treats idle/done/error/blocked as inactive", () => {
    for (const s of ["idle", "done", "error", "blocked"] as const) {
      expect(isActiveStatus(s)).toBe(false);
    }
  });
});

describe("countActive / totalTokens", () => {
  it("counts active agents and sums tokens", () => {
    const agents = [
      agent("a", "director", "working", null, { input: 10, output: 5 }),
      agent("b", "coder", "idle", "a", { input: 3, output: 2 }),
      agent("c", "auditor", "reviewing", "a", { input: 1, output: 1 }),
    ];
    expect(countActive(agents)).toBe(2);
    expect(totalTokens(agents)).toEqual({ input: 14, output: 8 });
  });
});

describe("deriveEdges", () => {
  it("creates ownership edges from parent links", () => {
    const agents = [
      agent("a", "director", "working"),
      agent("b", "coder", "working", "a"),
    ];
    const edges = deriveEdges(agents, []);
    expect(edges).toContainEqual({
      fromId: "a",
      toId: "b",
      kind: "owns",
      weight: 1,
    });
  });

  it("aggregates flow edges by direction and weight", () => {
    const agents = [
      agent("a", "director", "working"),
      agent("b", "coder", "working", "a"),
    ];
    const edges = deriveEdges(agents, [
      flow("a", "b"),
      flow("a", "b"),
      flow("b", "a"),
    ]);
    const ab = edges.find((e) => e.kind === "flow" && e.fromId === "a");
    const ba = edges.find((e) => e.kind === "flow" && e.fromId === "b");
    expect(ab?.weight).toBe(2);
    expect(ba?.weight).toBe(1);
  });

  it("drops flow edges referencing unknown or self agents", () => {
    const agents = [agent("a", "director", "working")];
    const edges = deriveEdges(agents, [flow("a", "ghost"), flow("a", "a")]);
    expect(edges.filter((e) => e.kind === "flow")).toHaveLength(0);
  });
});

describe("sortAgentsForDock", () => {
  it("puts the director first, then active, then most recent", () => {
    const a = agent("a", "coder", "idle");
    const b = agent("b", "director", "idle");
    const c = agent("c", "coder", "working");
    const sorted = sortAgentsForDock([a, b, c]);
    expect(sorted[0].id).toBe("b");
    expect(sorted[1].id).toBe("c");
  });
});
