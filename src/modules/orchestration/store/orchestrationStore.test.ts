import { beforeEach, describe, expect, it } from "vitest";
import { useOrchestrationStore } from "./orchestrationStore";

function reset() {
  useOrchestrationStore.setState({ agents: {}, flow: [], hydrated: false });
}

describe("orchestrationStore.spawn", () => {
  beforeEach(reset);

  it("creates an agent with role-default config and spawning status", () => {
    const id = useOrchestrationStore.getState().spawn({ role: "coder" });
    const a = useOrchestrationStore.getState().agents[id];
    expect(a.role).toBe("coder");
    expect(a.status).toBe("spawning");
    expect(a.config.model).toBeTruthy();
    expect(a.config.tools.length).toBeGreaterThan(0);
    expect(a.tokens).toEqual({ input: 0, output: 0 });
  });

  it("auto-names sequential agents of the same role", () => {
    const id1 = useOrchestrationStore.getState().spawn({ role: "coder" });
    const id2 = useOrchestrationStore.getState().spawn({ role: "coder" });
    const { agents } = useOrchestrationStore.getState();
    expect(agents[id1].name).toBe("Coder");
    expect(agents[id2].name).toBe("Coder 2");
  });

  it("logs a delegation flow event when spawned under a parent", () => {
    const s = useOrchestrationStore.getState();
    const dir = s.spawn({ role: "director" });
    useOrchestrationStore.getState().spawn({
      role: "coder",
      parentId: dir,
      task: "build the thing",
    });
    const flow = useOrchestrationStore.getState().flow;
    expect(flow.some((e) => e.kind === "delegation" && e.fromId === dir)).toBe(
      true,
    );
  });

  it("does not log a delegation when there is no parent", () => {
    useOrchestrationStore.getState().spawn({ role: "coder" });
    expect(useOrchestrationStore.getState().flow).toHaveLength(0);
  });
});

describe("orchestrationStore mutations", () => {
  beforeEach(reset);

  it("assign sets task, working status, and logs a delegation", () => {
    const s = useOrchestrationStore.getState();
    const dir = s.spawn({ role: "director" });
    const coder = useOrchestrationStore.getState().spawn({ role: "coder" });
    useOrchestrationStore.getState().assign(dir, coder, "fix the bug");
    const a = useOrchestrationStore.getState().agents[coder];
    expect(a.task).toBe("fix the bug");
    expect(a.status).toBe("working");
    const flow = useOrchestrationStore.getState().flow;
    expect(
      flow.some(
        (e) => e.kind === "delegation" && e.toId === coder && e.summary === "fix the bug",
      ),
    ).toBe(true);
  });

  it("addTokens accumulates usage and cost", () => {
    const id = useOrchestrationStore.getState().spawn({ role: "coder" });
    const store = useOrchestrationStore.getState();
    store.addTokens(id, { input: 100, output: 50 }, 0.2);
    store.addTokens(id, { input: 10, output: 5 }, 0.1);
    const a = useOrchestrationStore.getState().agents[id];
    expect(a.tokens).toEqual({ input: 110, output: 55 });
    expect(a.cost).toBeCloseTo(0.3);
  });

  it("linkTerminal attaches leaf/tab ids", () => {
    const id = useOrchestrationStore.getState().spawn({ role: "coder" });
    useOrchestrationStore.getState().linkTerminal(id, { leafId: 7, tabId: 3 });
    const a = useOrchestrationStore.getState().agents[id];
    expect(a.leafId).toBe(7);
    expect(a.tabId).toBe(3);
  });

  it("updateConfig merges limits without dropping other config", () => {
    const id = useOrchestrationStore.getState().spawn({ role: "coder" });
    const before = useOrchestrationStore.getState().agents[id].config.tools;
    useOrchestrationStore.getState().updateConfig(id, {
      limits: { contextLimit: 8000, costLimit: 5 },
    });
    const a = useOrchestrationStore.getState().agents[id];
    expect(a.config.limits).toEqual({ contextLimit: 8000, costLimit: 5 });
    expect(a.config.tools).toEqual(before);
  });

  it("remove deletes the agent", () => {
    const id = useOrchestrationStore.getState().spawn({ role: "coder" });
    useOrchestrationStore.getState().remove(id);
    expect(useOrchestrationStore.getState().agents[id]).toBeUndefined();
  });
});
