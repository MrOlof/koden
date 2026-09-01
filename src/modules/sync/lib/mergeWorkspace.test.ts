import { describe, expect, it } from "vitest";
import type { SpaceMeta, SpaceState } from "@/modules/spaces/lib/store";
import {
  mergeTombstoneMaps,
  mergeWorkspace,
  type WorkspaceLocal,
} from "./mergeWorkspace";
import type { WorkspaceEnvelope } from "./types";

function space(id: string, over: Partial<SpaceMeta> = {}): SpaceMeta {
  return {
    id,
    name: id,
    root: null,
    env: { kind: "local" },
    createdAt: 10,
    updatedAt: 10,
    contentUpdatedAt: 10,
    ...over,
  };
}

const state = (title: string): SpaceState => ({
  tabs: [{ kind: "notes", docId: "d", title }],
  activeTabIndex: 0,
});

function local(over: Partial<WorkspaceLocal> = {}): WorkspaceLocal {
  return {
    spaces: [],
    states: new Map(),
    stateMeta: {},
    tombstones: {},
    ...over,
  };
}

function remote(over: Partial<WorkspaceEnvelope> = {}): WorkspaceEnvelope {
  return {
    v: 1,
    spaces: [],
    states: {},
    stateMeta: {},
    tombstones: {},
    ...over,
  };
}

describe("space merge", () => {
  it("content clock wins; LRU updatedAt cannot beat a rename", () => {
    const l = space("a", {
      name: "visited",
      updatedAt: 900,
      contentUpdatedAt: 100,
    });
    const r = space("a", {
      name: "renamed",
      updatedAt: 500,
      contentUpdatedAt: 200,
    });
    const m = mergeWorkspace(local({ spaces: [l] }), remote({ spaces: [r] }));
    expect(m.spaces[0].name).toBe("renamed");
    expect(m.changedSpaces).toEqual(["a"]);
  });

  it("keeps local order and appends unseen remote spaces", () => {
    const m = mergeWorkspace(
      local({ spaces: [space("b"), space("a")] }),
      remote({ spaces: [space("c"), space("a")] }),
    );
    expect(m.spaces.map((s) => s.id)).toEqual(["b", "a", "c"]);
  });

  it("never adopts or pushes worktree spaces", () => {
    const wt = space("wt", { worktree: { repoRoot: "/r", branch: "b" } });
    const m = mergeWorkspace(
      local({ spaces: [wt] }),
      remote({
        spaces: [space("rwt", { worktree: { repoRoot: "/x", branch: "y" } })],
      }),
    );
    expect(m.spaces.map((s) => s.id)).toEqual(["wt"]);
    expect(m.pushNeeded).toBe(false);
  });
});

describe("tombstones", () => {
  // Real-clock-relative: mergeTombstoneMaps prunes entries past its TTL, so
  // epoch-era literals would silently vanish and prove nothing.
  const NOW = Date.now();

  it("removes a locally-present space deleted elsewhere, and cleans its state", () => {
    const l = local({
      spaces: [
        space("dead", { contentUpdatedAt: NOW - 900, updatedAt: NOW - 50 }),
      ],
      states: new Map([["dead", state("t")]]),
      stateMeta: { dead: { at: NOW - 900 } },
    });
    const m = mergeWorkspace(l, remote({ tombstones: { dead: NOW - 500 } }));
    expect(m.spaces).toEqual([]);
    expect(m.removedSpaces).toEqual(["dead"]);
    expect(m.states.has("dead")).toBe(false);
    expect(m.stateMeta.dead).toBeUndefined();
  });

  it("does not resurrect a deleted space from a remote copy", () => {
    const m = mergeWorkspace(
      local({ tombstones: { dead: NOW - 500 } }),
      remote({ spaces: [space("dead", { contentUpdatedAt: NOW - 900 })] }),
    );
    expect(m.spaces).toEqual([]);
    expect(m.pushNeeded).toBe(true);
  });

  it("a recreate or post-delete edit survives the tombstone", () => {
    const recreated = space("dead", {
      createdAt: NOW - 400,
      contentUpdatedAt: NOW - 400,
    });
    const edited = space("dead2", {
      createdAt: NOW - 900,
      contentUpdatedAt: NOW - 300,
    });
    const m = mergeWorkspace(
      local({ spaces: [recreated, edited] }),
      remote({ tombstones: { dead: NOW - 500, dead2: NOW - 500 } }),
    );
    expect(m.spaces.map((s) => s.id).sort()).toEqual(["dead", "dead2"]);
  });

  it("merges tombstone clocks as max", () => {
    const now = Date.now();
    const m = mergeWorkspace(
      local({ tombstones: { x: now - 100 } }),
      remote({ tombstones: { x: now - 50, y: now - 10 } }),
    );
    expect(m.tombstones).toEqual({ x: now - 50, y: now - 10 });
  });

  it("mergeTombstoneMaps keeps max per id and prunes past the TTL", () => {
    const now = 1_000_000_000_000;
    const ninetyOneDays = 91 * 24 * 3600_000;
    const merged = mergeTombstoneMaps(
      { a: now - 100, old: now - ninetyOneDays },
      { a: now - 50, b: now - 10 },
      now,
    );
    expect(merged).toEqual({ a: now - 50, b: now - 10 });
  });
});

describe("layout state merge", () => {
  it("newer stateMeta wins; a stamped side beats an unstamped side", () => {
    const l = local({
      spaces: [space("a"), space("b")],
      states: new Map([
        ["a", state("localA")],
        ["b", state("localB")],
      ]),
      stateMeta: { a: { at: 200 } },
    });
    const r = remote({
      spaces: [space("a"), space("b")],
      states: { a: state("remoteA"), b: state("remoteB") },
      stateMeta: { a: { at: 100 }, b: { at: 300 } },
    });
    const m = mergeWorkspace(l, r);
    expect(m.states.get("a")).toEqual(state("localA"));
    expect(m.states.get("b")).toEqual(state("remoteB"));
    expect(m.changedStates).toEqual(["b"]);
  });

  it("keeps local when both sides are unstamped (pre-sync data)", () => {
    const m = mergeWorkspace(
      local({ spaces: [space("a")], states: new Map([["a", state("L")]]) }),
      remote({ spaces: [space("a")], states: { a: state("R") } }),
    );
    expect(m.states.get("a")).toEqual(state("L"));
  });

  it("adopts a state for a newly-appended remote space", () => {
    const m = mergeWorkspace(
      local(),
      remote({
        spaces: [space("new")],
        states: { new: state("N") },
        stateMeta: { new: { at: 50 } },
      }),
    );
    expect(m.spaces.map((s) => s.id)).toEqual(["new"]);
    expect(m.states.get("new")).toEqual(state("N"));
  });
});

describe("pushNeeded", () => {
  it("is false when fully converged", () => {
    const a = space("a");
    const m = mergeWorkspace(
      local({
        spaces: [a],
        states: new Map([["a", state("t")]]),
        stateMeta: { a: { at: 100 } },
      }),
      remote({
        spaces: [a],
        states: { a: state("t") },
        stateMeta: { a: { at: 100 } },
      }),
    );
    expect(m.pushNeeded).toBe(false);
  });

  it("is true for a local-only space, a newer local state, or a newer tombstone", () => {
    expect(
      mergeWorkspace(local({ spaces: [space("only")] }), remote()).pushNeeded,
    ).toBe(true);
    expect(
      mergeWorkspace(
        local({
          spaces: [space("a")],
          states: new Map([["a", state("x")]]),
          stateMeta: { a: { at: 200 } },
        }),
        remote({
          spaces: [space("a")],
          states: { a: state("y") },
          stateMeta: { a: { at: 100 } },
        }),
      ).pushNeeded,
    ).toBe(true);
    expect(
      mergeWorkspace(local({ tombstones: { z: 100 } }), remote()).pushNeeded,
    ).toBe(true);
  });
});
