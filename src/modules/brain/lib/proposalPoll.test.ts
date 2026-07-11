import { describe, expect, it } from "vitest";
import {
  proposalKey,
  reconcileChanges,
  reconcileProposals,
} from "./proposalPoll";

const p = (project: string, signature: string) => ({ project, signature });
const c = (project: string, signature: string, status: string) => ({
  project,
  signature,
  status,
});

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

describe("reconcileChanges (ADR-018 revert guard)", () => {
  it("passes fetched through untouched when nothing is reverting", () => {
    const fetched = [c("a", "s1", "applied")];
    expect(reconcileChanges(fetched, new Set(), null)).toEqual(fetched);
  });

  it("holds an in-flight revert at 'reverted' so a stale poll can't flash the button back", () => {
    const reverting = new Set([proposalKey("a", "s1")]);
    // Worker hasn't landed yet: the backend still says applied.
    const out = reconcileChanges(
      [c("a", "s1", "applied"), c("a", "s2", "applied")],
      reverting,
      null,
    );
    expect(out).toEqual([c("a", "s1", "reverted"), c("a", "s2", "applied")]);
    expect(reverting.has(proposalKey("a", "s1"))).toBe(true);
  });

  it("forgets the key once the backend reports the row reverted", () => {
    const reverting = new Set([proposalKey("a", "s1")]);
    const out = reconcileChanges([c("a", "s1", "reverted")], reverting, null);
    expect(out).toEqual([c("a", "s1", "reverted")]);
    expect(reverting.size).toBe(0);
  });

  it("forgets a vanished key only when the fetch scope could have returned it", () => {
    const reverting = new Set([proposalKey("a", "s1")]);
    // b-scoped fetch says nothing about project a — keep waiting.
    reconcileChanges([c("b", "s9", "applied")], reverting, "b");
    expect(reverting.has(proposalKey("a", "s1"))).toBe(true);
    // An a-scoped fetch without the row = it vanished (e.g. project removed) — forget.
    reconcileChanges([], reverting, "a");
    expect(reverting.size).toBe(0);
  });
});
