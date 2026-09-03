import type { SerializedTab } from "@/modules/spaces/lib/serialize";
import type { SpaceState } from "@/modules/spaces/lib/store";
import { describe, expect, it } from "vitest";
import { mergeSpaceState } from "./mergeState";

const term = (key: string, customTitle?: string): SerializedTab => ({
  kind: "terminal",
  tree: { kind: "leaf", key, cwd: "/x" },
  ...(customTitle !== undefined && { customTitle }),
});
const split = (
  key: string,
  docId: string,
  customTitle?: string,
): SerializedTab => ({
  kind: "terminal",
  ...(customTitle !== undefined && { customTitle }),
  tree: {
    kind: "split",
    dir: "row",
    children: [
      { kind: "leaf", content: "note", docId, key: `${key}-note` },
      { kind: "leaf", key, cwd: "/x" },
    ],
  },
});
const st = (...tabs: SerializedTab[]): SpaceState => ({
  tabs,
  activeTabIndex: 0,
});
const labels = (s: SpaceState) =>
  s.tabs.map((t) => (t.kind === "terminal" ? (t.customTitle ?? "") : "?"));

describe("mergeSpaceState", () => {
  it("the incident: an observed bare tab never beats the author's rename + split", () => {
    // HQ authored the tab at 54; the laptop only observed it (clock 0) at 57.
    const hq = st(term("k1"));
    const hqEdited = st(split("k1", "n1", "TESTING TAB"));
    const hqMeta = { at: 54, tabs: { "t:k1": 54 } };
    const laptop = st(term("k1"));
    const laptopMeta = { at: 57, tabs: { "t:k1": 0 } };
    // Laptop pushes first (merge-then-write against HQ's edit): HQ wins.
    const onHost = mergeSpaceState(laptop, laptopMeta, hqEdited, hqMeta);
    expect(onHost.state).toEqual(hqEdited);
    expect(onHost.meta.tabs).toEqual({ "t:k1": 54 });
    expect(onHost.localNewer).toBe(false);
    // HQ pulls the laptop's copy: still its own.
    const onHq = mergeSpaceState(hqEdited, hqMeta, laptop, laptopMeta);
    expect(onHq.state).toEqual(hqEdited);
    expect(onHq.changed).toBe(false);
    expect(hq).not.toEqual(hqEdited);
  });

  it("unions tabs, keeps local order, appends remote-only tabs", () => {
    const m = mergeSpaceState(
      st(term("b"), term("a")),
      { at: 1, tabs: { "t:a": 1, "t:b": 1 } },
      st(term("a"), term("c"), term("b")),
      { at: 2, tabs: { "t:a": 1, "t:b": 1, "t:c": 2 } },
    );
    const treeOf = (t: SerializedTab) =>
      t.kind === "terminal" ? t.tree : null;
    expect(m.state.tabs.map(treeOf)).toEqual(
      [term("b"), term("a"), term("c")].map(treeOf),
    );
    expect(m.changes).toEqual([{ id: "t:c", kind: "added", after: term("c") }]);
    expect(m.localNewer).toBe(false);
  });

  it("per-tab clocks decide independently and report what changed", () => {
    const m = mergeSpaceState(
      st(term("a", "mine"), term("b", "old")),
      { at: 10, tabs: { "t:a": 10, "t:b": 3 } },
      st(term("a", "theirs"), term("b", "new")),
      { at: 9, tabs: { "t:a": 5, "t:b": 9 } },
    );
    expect(labels(m.state)).toEqual(["mine", "new"]);
    expect(m.meta.tabs).toEqual({ "t:a": 10, "t:b": 9 });
    expect(m.changes).toEqual([
      {
        id: "t:b",
        kind: "replaced",
        before: term("b", "old"),
        after: term("b", "new"),
      },
    ]);
    expect(m.localNewer).toBe(true);
    expect(m.changed).toBe(true);
  });

  it("a tombstone closes a tab unless it was edited after the close", () => {
    const closed = mergeSpaceState(
      st(term("a"), term("b")),
      { at: 10, tabs: { "t:a": 10, "t:b": 10 } },
      st(term("a")),
      { at: 20, tabs: { "t:a": 10 }, gone: { "t:b": 20 } },
      100,
    );
    expect(closed.state.tabs).toEqual([term("a")]);
    expect(closed.changes).toEqual([
      { id: "t:b", kind: "removed", before: term("b") },
    ]);
    expect(closed.meta.gone).toEqual({ "t:b": 20 });

    const revived = mergeSpaceState(
      st(term("a"), term("b", "renamed after")),
      { at: 30, tabs: { "t:a": 10, "t:b": 30 } },
      st(term("a")),
      { at: 20, tabs: { "t:a": 10 }, gone: { "t:b": 20 } },
      100,
    );
    expect(labels(revived.state)).toEqual(["", "renamed after"]);
    expect(revived.meta.gone).toEqual({});
    expect(revived.localNewer).toBe(true);
  });

  it("equal clocks resolve identically from both sides", () => {
    const a = st(term("k", "alpha"));
    const b = st(term("k", "beta"));
    const meta = { at: 5, tabs: { "t:k": 5 } };
    const fromA = mergeSpaceState(a, meta, b, meta).state;
    const fromB = mergeSpaceState(b, meta, a, meta).state;
    expect(fromA).toEqual(fromB);
  });

  it("adopts wholesale when there is no local layout and clamps the active index", () => {
    const remote: SpaceState = {
      tabs: [term("a"), term("b")],
      activeTabIndex: 7,
    };
    const m = mergeSpaceState(undefined, undefined, remote, { at: 3 });
    expect(m.state.tabs).toEqual(remote.tabs);
    expect(m.state.activeTabIndex).toBe(1);
    expect(m.meta.tabs).toEqual({ "t:a": 3, "t:b": 3 });
    expect(m.changed).toBe(true);
  });
});
