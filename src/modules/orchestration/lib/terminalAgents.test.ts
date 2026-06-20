import { describe, expect, it } from "vitest";
import type { PaneNode } from "@/modules/terminal";
import type { Tab } from "@/modules/tabs";
import { terminalsToRegister } from "./terminalAgents";

function leaf(id: number, cwd?: string): PaneNode {
  return { kind: "leaf", id, ...(cwd !== undefined && { cwd }) };
}

function notePane(id: number): PaneNode {
  return { kind: "leaf", id, content: "note", docId: `doc-${id}` };
}

function split(dir: "row" | "col", children: PaneNode[]): PaneNode {
  return { kind: "split", id: 900 + children.length, dir, children };
}

function termTab(
  id: number,
  tree: PaneNode,
  opts: { cold?: boolean; cwd?: string } = {},
): Tab {
  return {
    id,
    kind: "terminal",
    spaceId: "default",
    title: "term",
    paneTree: tree,
    activeLeafId: 0,
    ...(opts.cold && { cold: true }),
    ...(opts.cwd !== undefined && { cwd: opts.cwd }),
  };
}

describe("terminalsToRegister", () => {
  it("registers one seed per warm terminal leaf, named from its cwd", () => {
    const tabs = [termTab(1, leaf(2, "C:\\Users\\Snorlax\\my-proj"))];
    expect(terminalsToRegister(tabs, new Set())).toEqual([
      { leafId: 2, tabId: 1, name: "my-proj" },
    ]);
  });

  it("falls back to 'shell' when no cwd is known", () => {
    const tabs = [termTab(1, leaf(2))];
    expect(terminalsToRegister(tabs, new Set())).toEqual([
      { leafId: 2, tabId: 1, name: "shell" },
    ]);
  });

  it("uses the tab cwd when the leaf has none", () => {
    const tabs = [termTab(1, leaf(2), { cwd: "/home/me/app" })];
    expect(terminalsToRegister(tabs, new Set())[0].name).toBe("app");
  });

  it("excludes cold (restored, not-yet-opened) tabs", () => {
    const tabs = [termTab(1, leaf(2, "/x/proj"), { cold: true })];
    expect(terminalsToRegister(tabs, new Set())).toEqual([]);
  });

  it("excludes note panes but keeps the terminal in a split", () => {
    const tabs = [
      termTab(1, split("row", [leaf(2, "/x/proj"), notePane(3)])),
    ];
    expect(terminalsToRegister(tabs, new Set())).toEqual([
      { leafId: 2, tabId: 1, name: "proj" },
    ]);
  });

  it("skips leaves already owned by an agent (no double-registration)", () => {
    const tabs = [
      termTab(1, split("row", [leaf(2, "/x/a"), leaf(3, "/x/b")])),
    ];
    expect(terminalsToRegister(tabs, new Set([2]))).toEqual([
      { leafId: 3, tabId: 1, name: "b" },
    ]);
  });

  it("ignores non-terminal tabs", () => {
    const notesTab: Tab = {
      id: 5,
      kind: "notes",
      spaceId: "default",
      title: "Notes",
      docId: "n1",
    };
    expect(terminalsToRegister([notesTab], new Set())).toEqual([]);
  });

  it("registers every leaf across multiple warm terminal tabs", () => {
    const tabs = [
      termTab(1, leaf(2, "/x/a")),
      termTab(3, split("col", [leaf(4, "/x/b"), leaf(5, "/x/c")])),
    ];
    const seeds = terminalsToRegister(tabs, new Set());
    expect(seeds.map((s) => s.leafId)).toEqual([2, 4, 5]);
    expect(seeds.map((s) => s.name)).toEqual(["a", "b", "c"]);
  });
});
