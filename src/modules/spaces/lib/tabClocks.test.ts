import { describe, expect, it } from "vitest";
import type { SerializedTab } from "./serialize";
import type { SpaceState } from "./store";
import {
  stampTabs,
  TAB_GONE_TTL_MS,
  tabClocksOf,
  tabIdentities,
  tabIdentity,
} from "./tabClocks";

const term = (key: string, customTitle?: string): SerializedTab => ({
  kind: "terminal",
  tree: { kind: "leaf", key, cwd: "/x" },
  ...(customTitle !== undefined && { customTitle }),
});
const st = (...tabs: SerializedTab[]): SpaceState => ({
  tabs,
  activeTabIndex: 0,
});
const noLedger = () => undefined;

describe("tabIdentity", () => {
  it("keys terminals by oldest terminal pane key, docs by id, files by path", () => {
    expect(tabIdentity(term("k1"), 0)).toBe("t:k1");
    expect(
      tabIdentity(
        {
          kind: "terminal",
          tree: {
            kind: "split",
            dir: "row",
            children: [
              { kind: "leaf", content: "note", docId: "n1", key: "kn" },
              { kind: "leaf", key: "k9" },
              { kind: "leaf", key: "k2" },
            ],
          },
        },
        0,
      ),
    ).toBe("t:k2");
    expect(tabIdentity({ kind: "notes", docId: "d", title: "N" }, 0)).toBe(
      "n:d",
    );
    expect(tabIdentity({ kind: "editor", path: "/a.ts" }, 0)).toBe("f:/a.ts");
    expect(tabIdentity({ kind: "library" }, 0)).toBe("s:library");
    // A terminal with no restore key yet is positional, never merged.
    expect(tabIdentity({ kind: "terminal", tree: { kind: "leaf" } }, 3)).toBe(
      "t#3",
    );
  });

  it("disambiguates accidental duplicates positionally", () => {
    expect(tabIdentities([term("k1"), term("k1")])).toEqual(["t:k1", "t:k1#1"]);
  });
});

describe("tabClocksOf", () => {
  it("fills the space stamp for pre-ADR-025 metas and 0 for none", () => {
    const s = st(term("a"), term("b"));
    expect(tabClocksOf(s, { at: 50 })).toEqual({ "t:a": 50, "t:b": 50 });
    expect(tabClocksOf(s, { at: 50, tabs: { "t:a": 70 } })).toEqual({
      "t:a": 70,
      "t:b": 50,
    });
    expect(tabClocksOf(s, undefined)).toEqual({ "t:a": 0, "t:b": 0 });
  });
});

describe("stampTabs", () => {
  it("keeps an unchanged tab's clock, stamps a changed one now", () => {
    const prev = st(term("a", "x"), term("b"));
    const prevMeta = { at: 10, tabs: { "t:a": 10, "t:b": 5 } };
    const m = stampTabs(
      prev,
      prevMeta,
      st(term("a", "y"), term("b")),
      99,
      noLedger,
    );
    expect(m.tabs).toEqual({ "t:a": 10, "t:b": 5 });
    expect(m.titles).toEqual({ "t:a": 99, "t:b": 5 });
    expect(m.at).toBe(99);
  });

  it("a new tab takes the ledger's clock when the ledger knows it, else now", () => {
    const ledger = (id: string) => (id === "t:seen" ? 0 : undefined);
    const m = stampTabs(
      st(),
      undefined,
      st(term("seen"), term("mine", "named")),
      42,
      ledger,
    );
    expect(m.tabs).toEqual({ "t:seen": 0, "t:mine": 42 });
  });

  it("a changed tab also honours the ledger (live rename adoption)", () => {
    const prev = st(term("a", "old"));
    const m = stampTabs(
      prev,
      { at: 10, tabs: { "t:a": 10 } },
      st(term("a", "new")),
      99,
      (id, tab) =>
        id === "t:a" && tab.kind === "terminal" && tab.customTitle === "new"
          ? 77
          : undefined,
    );
    expect(m.titles?.["t:a"]).toBe(77);
    expect(m.tabs?.["t:a"]).toBe(10);
  });

  it("a closed tab gets a tombstone; reopening clears it", () => {
    const closed = stampTabs(
      st(term("a"), term("b")),
      { at: 10, tabs: { "t:a": 10, "t:b": 10 } },
      st(term("a")),
      50,
      noLedger,
    );
    expect(closed.gone).toEqual({ "t:b": 50 });
    expect(closed.at).toBe(50);
    const back = stampTabs(
      st(term("a")),
      closed,
      st(term("a"), term("b")),
      60,
      noLedger,
    );
    expect(back.gone).toEqual({});
    expect(back.tabs?.["t:b"]).toBe(60);
  });

  it("prunes tombstones past the TTL", () => {
    const now = TAB_GONE_TTL_MS * 2;
    const m = stampTabs(
      st(term("a")),
      { at: 1, tabs: { "t:a": 1 }, gone: { "t:old": 1, "t:recent": now - 1 } },
      st(term("a")),
      now,
      noLedger,
    );
    expect(m.gone).toEqual({ "t:recent": now - 1 });
  });

  it("cwd, colour and active pane are not edits; labels and structure are", () => {
    const prev = st({
      kind: "terminal",
      customTitle: "T",
      tree: { kind: "leaf", key: "a", cwd: "/x", color: "#111", active: true },
    });
    const prevMeta = { at: 10, tabs: { "t:a": 10 } };
    const churned = stampTabs(
      prev,
      prevMeta,
      st({
        kind: "terminal",
        customTitle: "T",
        tree: { kind: "leaf", key: "a", cwd: "/y", color: "#222" },
      }),
      99,
      noLedger,
    );
    expect(churned.tabs?.["t:a"]).toBe(10);
    expect(churned.titles?.["t:a"]).toBe(10);
    const labelled = stampTabs(
      prev,
      prevMeta,
      st({
        kind: "terminal",
        customTitle: "T",
        tree: { kind: "leaf", key: "a", cwd: "/x", color: "#111", title: "L" },
      }),
      99,
      noLedger,
    );
    expect(labelled.tabs?.["t:a"]).toBe(99);
  });

  it("the space clock never goes backwards", () => {
    const m = stampTabs(
      st(term("a")),
      { at: 500, tabs: { "t:a": 500 } },
      st(term("a"), term("b")),
      100,
      () => 0,
    );
    expect(m.at).toBe(500);
    expect(m.tabs?.["t:b"]).toBe(0);
  });
});
