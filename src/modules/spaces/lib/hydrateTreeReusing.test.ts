import type { PaneNode } from "@/modules/terminal/lib/panes";
import { describe, expect, it } from "vitest";
import { hydrateTreeReusing } from "./serialize";

describe("hydrateTreeReusing (ADR-025)", () => {
  it("keeps existing leaves by key and allocates + seeds only new ones", () => {
    const mine: Extract<PaneNode, { kind: "leaf" }> = {
      kind: "leaf",
      id: 7,
      cwd: "/keep/me",
    };
    let next = 100;
    const seeded: [number, string][] = [];
    const titled: [number, string | undefined, string | undefined][] = [];
    const out = hydrateTreeReusing(
      {
        kind: "split",
        dir: "row",
        children: [
          {
            kind: "leaf",
            content: "note",
            docId: "n1",
            key: "kn",
            title: "N",
            color: "#abc",
          },
          { kind: "leaf", key: "k1", cwd: "/their/cwd", active: true },
        ],
      },
      new Map([["k1", mine]]),
      () => next++,
      (id, t, c) => titled.push([id, t, c]),
      (id, k) => seeded.push([id, k]),
    );
    expect(out.tree).toEqual({
      kind: "split",
      id: 101,
      dir: "row",
      children: [{ kind: "leaf", id: 100, content: "note", docId: "n1" }, mine],
    });
    expect(out.activeLeafId).toBe(7);
    expect(seeded).toEqual([[100, "kn"]]);
    expect(titled).toEqual([[100, "N", "#abc"]]);
  });
});
