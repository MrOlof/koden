import { beforeEach, describe, expect, it } from "vitest";
import type { PaneNode } from "@/modules/terminal/lib/panes";
import type { Tab } from "@/modules/tabs/lib/useTabs";
import {
  capSnapshotText,
  isValidRestoreKey,
  leafRestoreKey,
  peekLeafRestoreKey,
  planGc,
  planScrollbackSave,
  pruneLeafRestoreKeys,
  resetLeafRestoreKeys,
  restorableLeaves,
  seedLeafRestoreKey,
  snapshotEntryKey,
} from "./scrollbackStore";
import { hydrateTabs, serializeTabs, type SerializedTab } from "./serialize";

function counter(start = 100): () => number {
  let n = start;
  return () => n++;
}

function term(over: Partial<Extract<Tab, { kind: "terminal" }>>): Tab {
  return {
    id: 1,
    kind: "terminal",
    spaceId: "s1",
    title: "shell",
    paneTree: { kind: "leaf", id: 2, cwd: "/a" },
    activeLeafId: 2,
    ...over,
  } as Tab;
}

function leafIdsOf(node: PaneNode): number[] {
  return node.kind === "leaf" ? [node.id] : node.children.flatMap(leafIdsOf);
}

beforeEach(() => resetLeafRestoreKeys());

describe("restore keys", () => {
  it("mints once per leaf and re-reads the same key", () => {
    const a = leafRestoreKey(7);
    expect(isValidRestoreKey(a)).toBe(true);
    expect(leafRestoreKey(7)).toBe(a);
    expect(leafRestoreKey(8)).not.toBe(a);
  });

  it("seeds only well-formed keys and prunes dead leaves", () => {
    seedLeafRestoreKey(1, "rk-abc-123");
    seedLeafRestoreKey(2, "../../etc/passwd");
    seedLeafRestoreKey(3, "");
    expect(peekLeafRestoreKey(1)).toBe("rk-abc-123");
    expect(peekLeafRestoreKey(2)).toBeUndefined();
    expect(peekLeafRestoreKey(3)).toBeUndefined();
    pruneLeafRestoreKeys(new Set([2]));
    expect(peekLeafRestoreKey(1)).toBeUndefined();
  });

  it("stays stable across serialize -> hydrate -> serialize (a restart)", () => {
    const tree: PaneNode = {
      kind: "split",
      id: 10,
      dir: "row",
      children: [
        { kind: "leaf", id: 11, cwd: "/a" },
        { kind: "leaf", id: 12, cwd: "/b" },
      ],
    };
    const before = serializeTabs(
      [term({ paneTree: tree, activeLeafId: 12 })],
      undefined,
      leafRestoreKey,
    );
    const k11 = leafRestoreKey(11);
    const k12 = leafRestoreKey(12);
    const node = before[0] as Extract<SerializedTab, { kind: "terminal" }>;
    if (node.tree.kind !== "split") throw new Error("expected split");
    expect(node.tree.children[0]).toMatchObject({ cwd: "/a", key: k11 });
    expect(node.tree.children[1]).toMatchObject({ cwd: "/b", key: k12 });

    // "Restart": fresh runtime ids, keys re-seeded from the file.
    resetLeafRestoreKeys();
    const json = JSON.parse(JSON.stringify(before)) as SerializedTab[];
    const seeded: Array<[number, string]> = [];
    const [restored] = hydrateTabs(json, "s1", counter(500), undefined, (id, k) =>
      {
        seedLeafRestoreKey(id, k);
        seeded.push([id, k]);
      });
    if (restored.kind !== "terminal") throw new Error("expected terminal");
    const [n11, n12] = leafIdsOf(restored.paneTree);
    expect(n11).toBeGreaterThanOrEqual(500);
    expect(seeded).toEqual([
      [n11, k11],
      [n12, k12],
    ]);
    // Snapshot entries resolve to the same identity as before the restart.
    expect(snapshotEntryKey("s1", leafRestoreKey(n11))).toBe(
      snapshotEntryKey("s1", k11),
    );
    const again = serializeTabs([restored], undefined, leafRestoreKey);
    expect(again).toEqual(before);
  });

  it("hydrates a pre-restore layout file (no keys) and mints keys afterwards", () => {
    const old: SerializedTab[] = [
      {
        kind: "terminal",
        tree: {
          kind: "split",
          dir: "col",
          children: [
            { kind: "leaf", cwd: "/a", active: true, title: "one" },
            { kind: "leaf", cwd: "/b" },
          ],
        },
        blocks: false,
      } as SerializedTab,
      { kind: "editor", path: "/x.ts" },
    ];
    const seeded: unknown[] = [];
    const tabs = hydrateTabs(old, "s9", counter(), undefined, (...a) =>
      seeded.push(a),
    );
    expect(seeded).toEqual([]);
    expect(tabs.map((t) => t.kind)).toEqual(["terminal", "editor"]);
    const t = tabs[0];
    if (t.kind !== "terminal") throw new Error("expected terminal");
    expect(t.cwd).toBe("/a");
    // Next save assigns keys; the layout stays valid for older readers.
    const out = serializeTabs(tabs, undefined, leafRestoreKey);
    const node = out[0] as Extract<SerializedTab, { kind: "terminal" }>;
    if (node.tree.kind !== "split") throw new Error("expected split");
    expect(node.tree.children.every((c) => "key" in c)).toBe(true);
    const withoutKeys = serializeTabs(tabs);
    expect(JSON.stringify(withoutKeys)).not.toContain('"key"');
  });
});

describe("planScrollbackSave", () => {
  const keyFor = (id: number) => `k${id}`;

  it("never persists private tabs, blocks terminals, or note panes", () => {
    const tabs: Tab[] = [
      term({ id: 1, paneTree: { kind: "leaf", id: 2, cwd: "/a" } }),
      term({
        id: 3,
        private: true,
        paneTree: { kind: "leaf", id: 4, cwd: "/secret" },
        activeLeafId: 4,
      }),
      term({
        id: 5,
        blocks: true,
        paneTree: { kind: "leaf", id: 6, cwd: "/b" },
        activeLeafId: 6,
      }),
      term({
        id: 7,
        paneTree: {
          kind: "split",
          id: 8,
          dir: "row",
          children: [
            { kind: "leaf", id: 9, cwd: "/c" },
            { kind: "leaf", id: 10, content: "note", docId: "d" },
          ],
        },
        activeLeafId: 9,
      }),
    ];
    expect(restorableLeaves(tabs, keyFor).map((l) => l.leafId)).toEqual([
      2, 9,
    ]);
    const captured: number[] = [];
    const plan = planScrollbackSave(
      tabs,
      (id) => {
        captured.push(id);
        return `buf${id}`;
      },
      keyFor,
    );
    expect(captured).toEqual([2, 9]);
    expect([...plan.writes.keys()]).toEqual(["snap:s1/k2", "snap:s1/k9"]);
    expect([...plan.writes.values()].join()).not.toContain("buf4");
  });

  it("keeps the on-disk entry for a leaf that has not rendered yet", () => {
    const tabs: Tab[] = [
      term({ id: 1, paneTree: { kind: "leaf", id: 2, cwd: "/a" } }),
      term({
        id: 3,
        paneTree: { kind: "leaf", id: 4, cwd: "/b" },
        activeLeafId: 4,
      }),
    ];
    const plan = planScrollbackSave(
      tabs,
      (id) => (id === 2 ? "live" : null),
      keyFor,
    );
    expect([...plan.writes.keys()]).toEqual(["snap:s1/k2"]);
    expect([...plan.keep]).toEqual(["snap:s1/k2", "snap:s1/k4"]);
    expect(
      planGc(["snap:s1/k2", "snap:s1/k4", "snap:s1/gone", "snap:s2/x"], plan.keep),
    ).toEqual(["snap:s1/gone", "snap:s2/x"]);
  });

  it("caps each snapshot at a line boundary and resets styling at the cut", () => {
    const line = `\x1b[31mred line\x1b[0m\r\n`;
    const text = line.repeat(100);
    const capped = capSnapshotText(text, line.length * 10 + 3);
    expect(capped.length).toBeLessThanOrEqual(line.length * 10 + 3 + 4);
    expect(capped.startsWith("\x1b[0m\x1b[31mred line")).toBe(true);
    expect(capped.endsWith("\r\n")).toBe(true);
    expect(capped.split("\r\n").length - 1).toBe(10);
    expect(capSnapshotText("short", 100)).toBe("short");
    // A single oversized line with no boundary is dropped rather than split.
    expect(capSnapshotText("x".repeat(50), 10)).toBe("");
    const plan = planScrollbackSave(
      [term({ id: 1, paneTree: { kind: "leaf", id: 2, cwd: "/a" } })],
      () => text,
      keyFor,
      line.length * 2,
    );
    expect(plan.writes.get("snap:s1/k2")?.length).toBeLessThanOrEqual(
      line.length * 2 + 4,
    );
  });
});
