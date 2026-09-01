import { describe, expect, it } from "vitest";
import type { SpaceMeta, SpaceState } from "@/modules/spaces/lib/store";
import {
  fromWirePath,
  mapSpacePaths,
  mapStatePaths,
  toWirePath,
  WIRE_ROOT,
} from "./pathMap";

const WIN_ROOT = "C:/Users/Snorlax/Snorlax";
const NIX_ROOT = "/home/snorlax/Snorlax";

describe("toWirePath / fromWirePath", () => {
  it("rewrites the root prefix to the wire token and back", () => {
    const wire = toWirePath(`${WIN_ROOT}/Products/koden`, WIN_ROOT);
    expect(wire).toBe(`${WIRE_ROOT}/Products/koden`);
    expect(fromWirePath(wire, NIX_ROOT)).toBe(`${NIX_ROOT}/Products/koden`);
  });

  it("normalizes backslashes and tolerates a trailing slash on the root", () => {
    const wire = toWirePath(
      "C:\\Users\\Snorlax\\Snorlax\\Work",
      `${WIN_ROOT}/`,
    );
    expect(wire).toBe(`${WIRE_ROOT}/Work`);
  });

  it("maps the root itself to the bare token", () => {
    expect(toWirePath(WIN_ROOT, WIN_ROOT)).toBe(WIRE_ROOT);
    expect(fromWirePath(WIRE_ROOT, NIX_ROOT)).toBe(NIX_ROOT);
  });

  it("does not rewrite a sibling path sharing the prefix text", () => {
    expect(toWirePath(`${WIN_ROOT}-backup/x`, WIN_ROOT)).toBe(
      `${WIN_ROOT}-backup/x`,
    );
  });

  it("passes paths through when no root is configured", () => {
    expect(toWirePath("/tmp/x", "")).toBe("/tmp/x");
    expect(fromWirePath(`${WIRE_ROOT}/x`, "")).toBe(`${WIRE_ROOT}/x`);
  });
});

describe("mapStatePaths / mapSpacePaths", () => {
  const map = (p: string) => toWirePath(p, NIX_ROOT);

  it("rewrites leaf cwds through a split tree and editor tab paths", () => {
    const state: SpaceState = {
      activeTabIndex: 0,
      tabs: [
        {
          kind: "terminal",
          tree: {
            kind: "split",
            dir: "row",
            children: [
              { kind: "leaf", cwd: `${NIX_ROOT}/a` },
              { kind: "leaf", content: "note", docId: "d1" },
            ],
          },
        },
        { kind: "editor", path: `${NIX_ROOT}/b.ts` },
        { kind: "notes", docId: "d2", title: "Notes" },
      ],
    };
    const mapped = mapStatePaths(state, map);
    const tree = mapped.tabs[0];
    if (tree.kind !== "terminal" || tree.tree.kind !== "split")
      throw new Error("shape");
    expect(tree.tree.children[0]).toMatchObject({ cwd: `${WIRE_ROOT}/a` });
    expect(tree.tree.children[1]).toMatchObject({ docId: "d1" });
    expect(mapped.tabs[1]).toMatchObject({ path: `${WIRE_ROOT}/b.ts` });
    expect(mapped.tabs[2]).toEqual(state.tabs[2]);
  });

  it("rewrites a local space root but never touches an ssh space", () => {
    const local: SpaceMeta = {
      id: "s1",
      name: "L",
      root: `${NIX_ROOT}/p`,
      env: { kind: "local" },
      createdAt: 1,
      updatedAt: 1,
    };
    const ssh: SpaceMeta = {
      ...local,
      id: "s2",
      root: "/remote/path",
      env: { kind: "ssh", host: "ai-server", path: "/remote/path" },
    };
    expect(mapSpacePaths(local, map).root).toBe(`${WIRE_ROOT}/p`);
    expect(mapSpacePaths(ssh, map)).toBe(ssh);
  });
});
