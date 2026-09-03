import type { SpaceMeta, SpaceState } from "@/modules/spaces/lib/store";
import { describe, expect, it } from "vitest";
import {
  mergeTombstoneMaps,
  mergeWorkspace,
  spaceIdentityKey,
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
    expect(m.changedStates).toContain("b");
    expect(m.stateMeta.a.tabs).toEqual({ "n:d": 200 });
    expect(m.stateMeta.b.tabs).toEqual({ "n:d": 300 });
  });

  it("resolves an unstamped tie the same way from either side (ADR-025)", () => {
    const fromL = mergeWorkspace(
      local({ spaces: [space("a")], states: new Map([["a", state("L")]]) }),
      remote({ spaces: [space("a")], states: { a: state("R") } }),
    );
    const fromR = mergeWorkspace(
      local({ spaces: [space("a")], states: new Map([["a", state("R")]]) }),
      remote({ spaces: [space("a")], states: { a: state("L") } }),
    );
    expect(fromL.states.get("a")).toEqual(fromR.states.get("a"));
  });

  it("merges tab by tab: a rename here and a new tab there both survive", () => {
    const mine: SpaceState = {
      activeTabIndex: 0,
      tabs: [
        {
          kind: "terminal",
          customTitle: "Power",
          tree: { kind: "leaf", key: "k1" },
        },
      ],
    };
    const theirs: SpaceState = {
      activeTabIndex: 0,
      tabs: [
        { kind: "terminal", tree: { kind: "leaf", key: "k1" } },
        {
          kind: "terminal",
          customTitle: "123",
          tree: { kind: "leaf", key: "k2" },
        },
      ],
    };
    const m = mergeWorkspace(
      local({
        spaces: [space("a")],
        states: new Map([["a", mine]]),
        stateMeta: { a: { at: 500, tabs: { "t:k1": 500 } } },
      }),
      remote({
        spaces: [space("a")],
        states: { a: theirs },
        stateMeta: { a: { at: 600, tabs: { "t:k1": 0, "t:k2": 600 } } },
      }),
    );
    const got = m.states.get("a");
    expect(
      got?.tabs.map((t) => (t.kind === "terminal" ? t.customTitle : "")),
    ).toEqual(["Power", "123"]);
    expect(m.stateMeta.a.tabs).toEqual({ "t:k1": 500, "t:k2": 600 });
    // Local won a tab on clock: the host must learn about it.
    expect(m.pushNeeded).toBe(true);
    expect(m.stateChanges.a).toEqual([
      { id: "t:k2", kind: "added", after: theirs.tabs[1] },
    ]);
  });

  it("a 0.12.0 peer without per-tab clocks is clocked at its space stamp", () => {
    const mine: SpaceState = {
      activeTabIndex: 0,
      tabs: [
        {
          kind: "terminal",
          customTitle: "new",
          tree: { kind: "leaf", key: "k1" },
        },
      ],
    };
    const theirs: SpaceState = {
      activeTabIndex: 0,
      tabs: [
        {
          kind: "terminal",
          customTitle: "old",
          tree: { kind: "leaf", key: "k1" },
        },
      ],
    };
    const m = mergeWorkspace(
      local({
        spaces: [space("a")],
        states: new Map([["a", mine]]),
        stateMeta: { a: { at: 700, tabs: { "t:k1": 700 } } },
      }),
      remote({
        spaces: [space("a")],
        states: { a: theirs },
        stateMeta: { a: { at: 300 } },
      }),
    );
    expect(m.states.get("a")).toEqual(mine);
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

describe("spaceIdentityKey", () => {
  it("folds path variants and is null for null-root locals and worktrees", () => {
    const a = space("a", { root: "C:/Users/Snorlax/Snorlax" });
    const b = space("b", { root: "c:\\Users\\Snorlax\\Snorlax\\" });
    expect(spaceIdentityKey(a)).toBe(spaceIdentityKey(b));
    expect(spaceIdentityKey(space("c", { root: null }))).toBeNull();
    expect(
      spaceIdentityKey(
        space("d", {
          root: "/x",
          worktree: { repoRoot: "/r", branch: "b" },
        }),
      ),
    ).toBeNull();
    const ssh1 = space("e", {
      root: null,
      env: { kind: "ssh", host: "ai-server", path: "/home/snorlax/Snorlax" },
    });
    const ssh2 = space("f", {
      root: null,
      env: { kind: "ssh", host: "ai-server", path: "/home/snorlax/Other" },
    });
    expect(spaceIdentityKey(ssh1)).not.toBe(spaceIdentityKey(ssh2));
  });
});

describe("identity fold (per-device duplicates)", () => {
  // The live incident: each device ran "Open folder as Space" on the same
  // tree, so each holds its own id; device A renamed hers, device B kept the
  // derived "Snorlax". First sync must yield ONE space with A's name.
  it("folds same-root spaces to the older id with the newer name", () => {
    const mine = space("sp-b", {
      name: "Snorlax",
      root: "/home/snorlax/Snorlax",
      createdAt: 2000,
      contentUpdatedAt: 2000,
    });
    const theirs = space("sp-a", {
      name: "Main",
      root: "/home/snorlax/Snorlax",
      createdAt: 1000,
      contentUpdatedAt: 5000,
    });
    const m = mergeWorkspace(
      local({
        spaces: [mine],
        states: new Map([["sp-b", state("localLayout")]]),
        stateMeta: { "sp-b": { at: 400 } },
      }),
      remote({
        spaces: [theirs],
        states: { "sp-a": state("remoteLayout") },
        stateMeta: { "sp-a": { at: 300 } },
      }),
    );
    expect(m.spaces).toHaveLength(1);
    const folded = m.spaces[0];
    expect(folded.id).toBe("sp-a");
    expect(folded.name).toBe("Main");
    expect(folded.createdAt).toBe(1000);
    expect(folded.contentUpdatedAt).toBe(5000);
    expect(m.idRemap).toEqual({ "sp-b": "sp-a" });
    expect(m.removedSpaces).toContain("sp-b");
    // The survivor's layout is the tab-by-tab merge: same doc id on both
    // sides, the better-stamped copy (local, 400) wins.
    expect(m.states.get("sp-a")).toEqual(state("localLayout"));
    expect(m.stateMeta["sp-a"]).toMatchObject({
      at: 400,
      tabs: { "n:d": 400 },
    });
    expect(m.states.has("sp-b")).toBe(false);
    expect(m.pushNeeded).toBe(true);
  });

  it("keeps the folded space at the local list position", () => {
    const dupLocal = space("sp-z", { root: "/t", createdAt: 500 });
    const other = space("sp-o", { root: "/elsewhere" });
    const dupRemote = space("sp-a", { root: "/t", createdAt: 100 });
    const m = mergeWorkspace(
      local({ spaces: [dupLocal, other] }),
      remote({ spaces: [dupRemote] }),
    );
    expect(m.spaces.map((s) => s.id)).toEqual(["sp-a", "sp-o"]);
  });

  it("does not fold distinct roots or null-root spaces", () => {
    const m = mergeWorkspace(
      local({
        spaces: [space("a", { root: "/x" }), space("n1", { root: null })],
      }),
      remote({
        spaces: [space("b", { root: "/y" }), space("n2", { root: null })],
      }),
    );
    expect(m.spaces).toHaveLength(4);
    expect(m.idRemap).toEqual({});
  });

  it("folds a three-way group deterministically regardless of side", () => {
    const s1 = space("sp-1", {
      root: "/t",
      createdAt: 100,
      contentUpdatedAt: 100,
    });
    const s2 = space("sp-2", {
      root: "/t",
      createdAt: 200,
      contentUpdatedAt: 300,
      name: "newest",
    });
    const s3 = space("sp-3", {
      root: "/t",
      createdAt: 300,
      contentUpdatedAt: 200,
    });
    const fromA = mergeWorkspace(
      local({ spaces: [s2, s3] }),
      remote({ spaces: [s1] }),
    );
    const fromB = mergeWorkspace(
      local({ spaces: [s1] }),
      remote({ spaces: [s2, s3] }),
    );
    for (const m of [fromA, fromB]) {
      expect(m.spaces).toHaveLength(1);
      expect(m.spaces[0].id).toBe("sp-1");
      expect(m.spaces[0].name).toBe("newest");
    }
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
