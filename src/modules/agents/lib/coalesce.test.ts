import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CALM_WINDOW_MS, createCoalescer } from "./coalesce";

describe("createCoalescer", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("passes a single event through unchanged after the window", () => {
    const flush = vi.fn();
    const c = createCoalescer(flush);
    c.add({
      kind: "finished",
      agent: "claude",
      title: "claude finished",
      body: "Snorlax",
    });
    expect(flush).not.toHaveBeenCalled();
    vi.advanceTimersByTime(CALM_WINDOW_MS);
    expect(flush).toHaveBeenCalledTimes(1);
    expect(flush).toHaveBeenCalledWith("claude finished", "Snorlax");
  });

  it("batches events inside one window into one summary", () => {
    const flush = vi.fn();
    const c = createCoalescer(flush);
    c.add({
      kind: "finished",
      agent: "claude",
      title: "claude finished",
      body: "Snorlax",
    });
    vi.advanceTimersByTime(1000);
    c.add({
      kind: "finished",
      agent: "claude",
      title: "claude finished",
      body: "Lockin",
    });
    c.add({
      kind: "finished",
      agent: "claude",
      title: "claude finished",
      body: "M365++",
    });
    vi.advanceTimersByTime(CALM_WINDOW_MS);
    expect(flush).toHaveBeenCalledTimes(1);
    expect(flush).toHaveBeenCalledWith(
      "3 agents finished",
      "Snorlax, Lockin, M365++",
    );
  });

  it("dedupes labels and uses the generic title on mixed kinds", () => {
    const flush = vi.fn();
    const c = createCoalescer(flush);
    c.add({
      kind: "finished",
      agent: "claude",
      title: "claude finished",
      body: "Snorlax",
    });
    c.add({
      kind: "memory",
      agent: "Librarian",
      title: "Memory updated",
      body: "Snorlax",
    });
    vi.advanceTimersByTime(CALM_WINDOW_MS);
    expect(flush).toHaveBeenCalledTimes(1);
    expect(flush).toHaveBeenCalledWith("2 agent updates", "Snorlax");
  });

  it("starts a fresh window after a flush", () => {
    const flush = vi.fn();
    const c = createCoalescer(flush);
    c.add({ kind: "finished", agent: "claude", title: "a", body: "A" });
    vi.advanceTimersByTime(CALM_WINDOW_MS);
    c.add({ kind: "finished", agent: "claude", title: "b", body: "B" });
    vi.advanceTimersByTime(CALM_WINDOW_MS);
    expect(flush).toHaveBeenCalledTimes(2);
    expect(flush).toHaveBeenLastCalledWith("b", "B");
  });
});
