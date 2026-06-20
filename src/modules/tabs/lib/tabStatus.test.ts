import { beforeEach, describe, expect, it } from "vitest";
import { useTabStatusStore, worseTabStatus } from "./tabStatus";

const get = () => useTabStatusStore.getState();

beforeEach(() => {
  useTabStatusStore.setState({ statuses: {} });
});

describe("worseTabStatus", () => {
  it("ranks error > waiting > done > working", () => {
    expect(worseTabStatus("working", "done")).toBe("done");
    expect(worseTabStatus("done", "waiting")).toBe("waiting");
    expect(worseTabStatus("waiting", "error")).toBe("error");
    expect(worseTabStatus("error", "working")).toBe("error");
  });

  it("is order-independent", () => {
    expect(worseTabStatus("done", "working")).toBe("done");
    expect(worseTabStatus("working", "done")).toBe("done");
  });
});

describe("tab status escalate (worst-wins roll-up)", () => {
  it("raises to a more urgent status", () => {
    get().escalate(1, "working");
    expect(get().statuses[1]).toBe("working");
    get().escalate(1, "waiting");
    expect(get().statuses[1]).toBe("waiting");
  });

  it("does not downgrade a more urgent unseen status", () => {
    get().escalate(1, "waiting");
    get().escalate(1, "working");
    expect(get().statuses[1]).toBe("waiting");
    get().escalate(1, "done");
    expect(get().statuses[1]).toBe("waiting");
  });

  it("surfaces a finished terminal over one still working", () => {
    get().escalate(2, "working");
    get().escalate(2, "done");
    expect(get().statuses[2]).toBe("done");
  });

  it("clear resets so the next cycle re-accumulates", () => {
    get().escalate(3, "waiting");
    get().clear(3);
    expect(get().statuses[3]).toBeUndefined();
    get().escalate(3, "working");
    expect(get().statuses[3]).toBe("working");
  });
});
