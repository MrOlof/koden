import { useSpaces } from "@/modules/spaces/lib/useSpaces";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SpaceInfo, ToolContext } from "./context";
import { buildSpaceTools, resolveSpaceTarget, snapshotSpaces } from "./spaces";

// The spaces store persists through the Tauri store plugin; stub it so the
// real zustand store is exercisable in node.
vi.mock("@tauri-apps/plugin-store", () => ({
  LazyStore: class {
    async set() {}
    async get() {
      return undefined;
    }
    async entries() {
      return [];
    }
    async delete() {
      return true;
    }
    async save() {}
  },
}));

const CALL_OPTS = { toolCallId: "t", messages: [] };

function info(over: Partial<SpaceInfo>): SpaceInfo {
  return { id: "sp-x", name: "X", active: false, tabCount: 0, ...over };
}

describe("snapshotSpaces", () => {
  it("maps store spaces to model shape with tab counts", () => {
    const out = snapshotSpaces(
      [
        { id: "a", name: "One" },
        { id: "b", name: "Two" },
      ],
      "b",
      [{ spaceId: "a" }, { spaceId: "a" }, { spaceId: "b" }],
    );
    expect(out).toEqual([
      { id: "a", name: "One", active: false, tabCount: 2 },
      { id: "b", name: "Two", active: true, tabCount: 1 },
    ]);
  });
});

describe("resolveSpaceTarget", () => {
  const spaces = [
    info({ id: "sp-1", name: "Default", active: true, tabCount: 2 }),
    info({ id: "sp-2", name: "deploy", tabCount: 1 }),
    info({ id: "sp-3", name: "Deploy" }),
    info({ id: "sp-4", name: "Research notes" }),
  ];

  it("space id wins outright", () => {
    expect(resolveSpaceTarget("sp-3", spaces)).toEqual({
      ok: true,
      space: spaces[2],
      via: "space-id",
    });
  });

  it("exact name beats the case-insensitive tier", () => {
    expect(resolveSpaceTarget("deploy", spaces)).toMatchObject({
      ok: true,
      space: { id: "sp-2" },
      via: "name",
    });
  });

  it("case-insensitive name beats substring", () => {
    expect(resolveSpaceTarget("default", spaces)).toMatchObject({
      ok: true,
      space: { id: "sp-1" },
      via: "name-ci",
    });
  });

  it("a single loose fragment resolves via substring", () => {
    expect(resolveSpaceTarget("research", spaces)).toMatchObject({
      ok: true,
      space: { id: "sp-4" },
      via: "name-substring",
    });
  });

  it("ambiguity errors with the candidates, never a best-effort pick", () => {
    const r = resolveSpaceTarget("DEPLOY", spaces);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toContain("ambiguous");
      expect(r.error).toContain("sp-2");
      expect(r.error).toContain("sp-3");
    }
  });

  it("no match errors listing every space", () => {
    const r = resolveSpaceTarget("zzz", spaces);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toContain("no space matches 'zzz'");
      expect(r.error).toContain("'Default'");
      expect(r.error).toContain("'Research notes'");
    }
  });

  it("rejects an empty target", () => {
    expect(resolveSpaceTarget("   ", spaces)).toMatchObject({
      ok: false,
      error: expect.stringContaining("empty target"),
    });
  });
});

// Why this seam: the Live bridge is a React hook (no renderHook dep in the
// repo), so its glue is replayed here against the REAL zustand spaces store,
// exactly as layout.test.ts replays useTabs at the pane primitives. create
// mirrors App's handleNewSpace (create + setActive); switch mirrors the
// bridge's switchSpace; list is snapshotSpaces over live store state.
function storeCtx(): ToolContext {
  return {
    listSpaces: () => {
      const { spaces, activeId } = useSpaces.getState();
      return snapshotSpaces(spaces, activeId, []);
    },
    createSpace: (name: string) => {
      const { spaces, create, setActive } = useSpaces.getState();
      const meta = create({
        name: name.trim() || `Space ${spaces.length + 1}`,
        root: null,
      });
      setActive(meta.id);
      return { spaceId: meta.id, name: meta.name, switched: true as const };
    },
    switchSpace: (id: string) => {
      const { spaces, setActive } = useSpaces.getState();
      if (!spaces.some((s) => s.id === id)) return false;
      setActive(id);
      return true;
    },
  } as unknown as ToolContext;
}

describe("space tools at the store seam", () => {
  beforeEach(() => {
    useSpaces.getState().hydrate(
      [
        {
          id: "sp-default",
          name: "Default",
          root: null,
          env: { kind: "local" },
          createdAt: 1,
          updatedAt: 1,
        },
      ],
      "sp-default",
    );
  });

  it("workspace_create_space creates AND switches", async () => {
    const tools = buildSpaceTools(storeCtx());
    const res = await tools.workspace_create_space.execute?.(
      { name: "Research" },
      CALL_OPTS,
    );
    const { spaces, activeId } = useSpaces.getState();
    expect(spaces.map((s) => s.name)).toEqual(["Default", "Research"]);
    expect(activeId).toBe(spaces[1].id);
    expect(res).toEqual({
      spaceId: spaces[1].id,
      name: "Research",
      switched: true,
    });
  });

  it("create then switch back by loose name", async () => {
    const tools = buildSpaceTools(storeCtx());
    await tools.workspace_create_space.execute?.(
      { name: "Research" },
      CALL_OPTS,
    );
    const res = await tools.workspace_switch_space.execute?.(
      { target: "default" },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      spaceId: "sp-default",
      switched: true,
      matched_by: "name-ci",
    });
    expect(useSpaces.getState().activeId).toBe("sp-default");
  });

  it("switching to the already-active space is a flagged no-op", async () => {
    const tools = buildSpaceTools(storeCtx());
    const res = await tools.workspace_switch_space.execute?.(
      { target: "Default" },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      spaceId: "sp-default",
      switched: false,
      note: expect.stringContaining("already"),
    });
    expect(useSpaces.getState().activeId).toBe("sp-default");
  });

  it("duplicate names create fine (UI rule) but switch-by-name goes ambiguous", async () => {
    const tools = buildSpaceTools(storeCtx());
    await tools.workspace_create_space.execute?.(
      { name: "Research" },
      CALL_OPTS,
    );
    const dup = await tools.workspace_create_space.execute?.(
      { name: "Research" },
      CALL_OPTS,
    );
    expect(dup).toMatchObject({
      switched: true,
      note: expect.stringContaining("shares this name"),
    });
    expect(
      useSpaces.getState().spaces.filter((s) => s.name === "Research"),
    ).toHaveLength(2);
    const sw = await tools.workspace_switch_space.execute?.(
      { target: "Research" },
      CALL_OPTS,
    );
    expect(sw).toMatchObject({
      error: expect.stringContaining("ambiguous"),
    });
  });

  it("workspace_list_spaces reports count and the active name", async () => {
    const tools = buildSpaceTools(storeCtx());
    await tools.workspace_create_space.execute?.({ name: "Ops" }, CALL_OPTS);
    const res = await tools.workspace_list_spaces.execute?.({}, CALL_OPTS);
    expect(res).toMatchObject({ count: 2, active: "Ops" });
  });

  it("rejects a blank name", async () => {
    const tools = buildSpaceTools(storeCtx());
    const res = await tools.workspace_create_space.execute?.(
      { name: "   " },
      CALL_OPTS,
    );
    expect(res).toEqual({ error: "space name is empty" });
    expect(useSpaces.getState().spaces).toHaveLength(1);
  });
});
