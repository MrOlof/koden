import { describe, expect, it } from "vitest";
import { proposalKey, reconcileProposals } from "./proposalPoll";

const p = (project: string, signature: string) => ({ project, signature });

describe("reconcileProposals", () => {
  it("passes fetched through untouched when nothing is pending", () => {
    const fetched = [p("a", "s1"), p("a", "s2")];
    const pending = new Set<string>();
    expect(reconcileProposals(fetched, pending, null)).toEqual(fetched);
  });

  it("hides an in-flight resolution so the poll cannot clobber the optimistic removal", () => {
    const fetched = [p("a", "s1"), p("a", "s2")];
    const pending = new Set([proposalKey("a", "s1")]);
    // Worker hasn't applied yet: the backend still returns s1 — it must stay hidden.
    expect(reconcileProposals(fetched, pending, null)).toEqual([p("a", "s2")]);
    expect(pending.has(proposalKey("a", "s1"))).toBe(true);
  });

  it("forgets a pending key once the worker applied it (gone from the backend)", () => {
    const pending = new Set([proposalKey("a", "s1")]);
    expect(reconcileProposals([p("a", "s2")], pending, null)).toEqual([
      p("a", "s2"),
    ]);
    expect(pending.size).toBe(0);
    // A later re-appearance of the same signature (e.g. a fresh doctor run) shows again.
    expect(reconcileProposals([p("a", "s1")], pending, null)).toEqual([
      p("a", "s1"),
    ]);
  });

  it("keys per project — the same signature in another project is unaffected", () => {
    const fetched = [p("a", "s1"), p("b", "s1")];
    const pending = new Set([proposalKey("a", "s1")]);
    expect(reconcileProposals(fetched, pending, null)).toEqual([p("b", "s1")]);
  });

  it("does not forget another project's pending key on a project-scoped fetch", () => {
    // Resolve P(a, s1), then the selector switches to project b: the b-scoped
    // fetch never returns a's proposals — absence is filtering, not "applied".
    const pending = new Set([proposalKey("a", "s1")]);
    expect(reconcileProposals([p("b", "s2")], pending, "b")).toEqual([
      p("b", "s2"),
    ]);
    expect(pending.has(proposalKey("a", "s1"))).toBe(true);
    // A still-outstanding a-scoped tick keeps the resolved card hidden…
    expect(reconcileProposals([p("a", "s1")], pending, "a")).toEqual([]);
    // …until an a-scoped fetch no longer returns it (worker applied it).
    expect(reconcileProposals([], pending, "a")).toEqual([]);
    expect(pending.size).toBe(0);
  });

  it("forgets a scope-matching pending key that the scoped fetch no longer returns", () => {
    const pending = new Set([proposalKey("a", "s1"), proposalKey("b", "s1")]);
    expect(reconcileProposals([], pending, "a")).toEqual([]);
    expect(pending.has(proposalKey("a", "s1"))).toBe(false);
    expect(pending.has(proposalKey("b", "s1"))).toBe(true);
  });
});
