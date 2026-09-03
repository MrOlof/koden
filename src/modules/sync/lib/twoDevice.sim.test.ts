// Two devices, one host, no restarts in between (ADR-025). A simulation of
// the real pipeline: the real stamping rule, the real merge, the real live
// planners, an in-memory host with gens. Replays the 2026-09-03 incident
// step by step, then fuzzes seeded interleavings of user edits, tmux
// materialization, pushes, live pulls and boots, asserting the invariants
// the rework exists for:
//   (A) an author's edit is on the host after both devices' next cycle;
//   (B) a device never shows an older value than the latest authored one;
//   (C) both devices converge after boot.
// This is the regression failsafe for the CLASS of bug, not the instance.
import type {
  SerializedNode,
  SerializedTab,
} from "@/modules/spaces/lib/serialize";
import {
  hydrateTreeReusing,
  serializeTabs,
} from "@/modules/spaces/lib/serialize";
import type {
  SpaceMeta,
  SpaceState,
  SpaceStateMeta,
} from "@/modules/spaces/lib/store";
import {
  stampTabs,
  tabIdentities,
  tabStructureJson,
} from "@/modules/spaces/lib/tabClocks";
import type { Tab } from "@/modules/tabs/lib/useTabs";
import type { PaneNode } from "@/modules/terminal/lib/panes";
import { describe, expect, it } from "vitest";
import {
  liveTabIdentity,
  planLiveDocAdoption,
  planLiveRenames,
  planLiveTrees,
} from "./liveAdopt";
import { mergeWorkspace, type WorkspaceLocal } from "./mergeWorkspace";
import type { WorkspaceEnvelope } from "./types";

const SPACE = "sp-ai";
const spaceMeta: SpaceMeta = {
  id: SPACE,
  name: "ai-server",
  root: "/home/snorlax",
  env: { kind: "ssh", host: "ai-server", path: "/home/snorlax" },
  sshTmux: true,
  createdAt: 1,
  updatedAt: 1,
  contentUpdatedAt: 1,
};

class Host {
  gen = 0;
  env: WorkspaceEnvelope | null = null;
  write(env: WorkspaceEnvelope, now: number): void {
    this.gen = now;
    this.env = JSON.parse(JSON.stringify(env)) as WorkspaceEnvelope;
  }
  tab(key: string): SerializedTab | undefined {
    const st = this.env?.states[SPACE];
    if (!st) return undefined;
    const ids = tabIdentities(st.tabs);
    const i = ids.indexOf(`t:${key}`);
    return i >= 0 ? st.tabs[i] : undefined;
  }
}

/** tmux truth: which windows exist on the host. */
class Tmux {
  windows = new Set<string>();
}

type Ledger = Map<
  string,
  { clock: number; match?: (t: SerializedTab) => boolean }
>;

function firstLeafId(n: PaneNode): number {
  return n.kind === "leaf" ? n.id : firstLeafId(n.children[0]);
}

class Device {
  tabs: Tab[] = [];
  nextId = 1;
  keys = new Map<number, string>();
  disk: SpaceState | undefined;
  diskMeta: SpaceStateMeta | undefined;
  ledger: Ledger = new Map();
  colors = new Map<number, string>();
  lastLiveGen = 0;
  pushes = 0;
  constructor(
    readonly name: string,
    readonly tmux: Tmux,
  ) {}

  leafKey = (id: number): string | undefined => this.keys.get(id);

  private newLeaf(key: string): PaneNode {
    const id = this.nextId++;
    this.keys.set(id, key);
    return { kind: "leaf", id, cwd: "/home/snorlax" };
  }

  find(key: string): Tab | undefined {
    return this.tabs.find(
      (t) =>
        t.kind === "terminal" &&
        liveTabIdentity(t, this.leafKey) === `t:${key}`,
    );
  }

  label(key: string): string | undefined {
    const t = this.find(key);
    return t && t.kind === "terminal" ? (t.customTitle ?? "") : undefined;
  }

  hasNotePane(key: string): boolean {
    const t = this.find(key);
    if (!t || t.kind !== "terminal") return false;
    const walk = (n: PaneNode): boolean =>
      n.kind === "leaf" ? n.content === "note" : n.children.some(walk);
    return walk(t.paneTree);
  }

  private addTerminal(key: string, customTitle?: string): void {
    const leaf = this.newLeaf(key);
    this.tabs.push({
      id: this.nextId++,
      spaceId: SPACE,
      kind: "terminal",
      title: "shell",
      activeLeafId: leaf.id,
      paneTree: leaf,
      ...(customTitle !== undefined && { customTitle }),
    } as Tab);
  }

  // User operations: each lands on disk at `now`, as the 3 s debounce does.
  create(key: string, now: number, title?: string): void {
    this.tmux.windows.add(key);
    this.addTerminal(key, title);
    this.persist(now);
  }

  rename(key: string, title: string, now: number): boolean {
    const t = this.find(key);
    if (!t || t.kind !== "terminal") return false;
    (t as { customTitle?: string }).customTitle = title || undefined;
    this.persist(now);
    return true;
  }

  close(key: string, now: number): boolean {
    const t = this.find(key);
    if (!t) return false;
    this.tabs = this.tabs.filter((x) => x.id !== t.id);
    this.tmux.windows.delete(key);
    this.persist(now);
    return true;
  }

  splitNote(key: string, docId: string, now: number): boolean {
    const t = this.find(key);
    if (!t || t.kind !== "terminal") return false;
    const noteId = this.nextId++;
    this.keys.set(noteId, `${key}-note`);
    (t as { paneTree: PaneNode }).paneTree = {
      kind: "split",
      id: this.nextId++,
      dir: "row",
      children: [
        { kind: "leaf", id: noteId, content: "note", docId },
        t.paneTree,
      ],
    };
    this.persist(now);
    return true;
  }

  // Derived: the shell reports a new cwd, the pane gets an accent. Neither
  // is an edit; a device doing only this must never win a merge.
  churn(key: string, now: number): boolean {
    const t = this.find(key);
    if (!t || t.kind !== "terminal") return false;
    const leafId = firstLeafId(t.paneTree);
    const bump = (n: PaneNode): PaneNode =>
      n.kind === "leaf"
        ? n.id === leafId
          ? { ...n, cwd: `/home/snorlax/dir-${now}` }
          : n
        : { ...n, children: n.children.map(bump) };
    (t as { paneTree: PaneNode }).paneTree = bump(t.paneTree);
    this.colors.set(leafId, `#${(now % 0xffffff).toString(16)}`);
    this.persist(now);
    return true;
  }

  // Derived: the remote-space loop finds windows no local pane owns.
  materialize(now: number): void {
    let added = false;
    for (const key of this.tmux.windows) {
      if (this.find(key)) continue;
      this.ledger.set(`t:${key}`, { clock: 0 });
      this.addTerminal(key);
      added = true;
    }
    if (added) this.persist(now);
  }

  serialize(): SpaceState {
    return {
      tabs: serializeTabs(
        this.tabs,
        (leafId) => {
          const color = this.colors.get(leafId);
          return color ? { color } : undefined;
        },
        this.leafKey,
      ),
      activeTabIndex: 0,
    };
  }

  persist(now: number): void {
    const next = this.serialize();
    this.diskMeta = stampTabs(
      this.disk,
      this.diskMeta,
      next,
      now,
      (id, tab) => {
        const e = this.ledger.get(id);
        if (!e) return undefined;
        this.ledger.delete(id);
        if (e.match && !e.match(tab)) return undefined;
        return e.clock;
      },
    );
    this.disk = next;
  }

  local(): WorkspaceLocal {
    return {
      spaces: [spaceMeta],
      states: new Map(this.disk ? [[SPACE, this.disk]] : []),
      stateMeta: this.diskMeta ? { [SPACE]: this.diskMeta } : {},
      tombstones: {},
    };
  }

  push(host: Host, now: number): void {
    this.pushes++;
    const local = this.local();
    if (!host.env) {
      host.write(
        {
          v: 1,
          spaces: local.spaces,
          states: Object.fromEntries(local.states),
          stateMeta: local.stateMeta,
          tombstones: {},
        },
        now,
      );
      return;
    }
    const m = mergeWorkspace(local, host.env, now);
    host.write(
      {
        v: 1,
        spaces: m.spaces,
        states: Object.fromEntries(m.states),
        stateMeta: m.stateMeta,
        tombstones: m.tombstones,
      },
      now,
    );
  }

  livePull(host: Host, now: number): void {
    if (!host.env || host.gen === this.lastLiveGen) return;
    this.lastLiveGen = host.gen;
    const st = host.env.states[SPACE];
    const meta = host.env.stateMeta[SPACE];
    if (st) {
      let dirty = false;
      const rClocks = new Map<string, number>();
      for (const id of tabIdentities(st.tabs))
        rClocks.set(id, meta?.tabs?.[id] ?? meta?.at ?? 0);
      // Structure first: a peer's split arrives as the split (ADR-025).
      const inPanes = new Set<string>();
      for (const p of planLiveTrees(
        SPACE,
        this.tabs,
        this.disk,
        this.diskMeta,
        st,
        meta,
        this.leafKey,
      )) {
        this.ledger.set(p.identity, {
          clock: p.clock,
          match: (tab) =>
            tabStructureJson(tab) === tabStructureJson(p.remoteTab),
        });
        const t = this.tabs.find((x) => x.id === p.tabId);
        if (!t || t.kind !== "terminal") continue;
        const existing = new Map<string, Extract<PaneNode, { kind: "leaf" }>>();
        const walk = (n: PaneNode): void => {
          if (n.kind === "leaf") {
            const k = this.keys.get(n.id);
            if (k) existing.set(k, n);
          } else for (const c of n.children) walk(c);
        };
        walk(t.paneTree);
        const live = hydrateTreeReusing(
          p.tree,
          existing,
          () => this.nextId++,
          undefined,
          (id, key) => this.keys.set(id, key),
        );
        (t as { paneTree: PaneNode }).paneTree = live.tree;
        for (const d of p.docIds) inPanes.add(d);
        dirty = true;
      }
      for (const d of planLiveDocAdoption(SPACE, this.tabs, st).create) {
        if (inPanes.has(d.id)) continue;
        const identity = `n:${d.id}`;
        this.ledger.set(identity, { clock: rClocks.get(identity) ?? 0 });
        this.tabs.push({
          id: this.nextId++,
          spaceId: SPACE,
          kind: "notes",
          docId: d.id,
          title: d.title,
        } as Tab);
        dirty = true;
      }
      for (const r of planLiveRenames(
        SPACE,
        this.tabs,
        this.disk,
        this.diskMeta,
        st,
        meta,
        this.leafKey,
      )) {
        this.ledger.set(r.identity, {
          clock: r.clock,
          match: (tab) =>
            tab.kind === "terminal" && (tab.customTitle ?? "") === r.title,
        });
        const t = this.tabs.find((x) => x.id === r.tabId);
        if (t)
          (t as { customTitle?: string }).customTitle = r.title || undefined;
        dirty = true;
      }
      if (dirty) this.persist(now);
    }
    const m = mergeWorkspace(this.local(), host.env, now);
    if (m.pushNeeded) this.push(host, now);
  }

  private rebuild(n: SerializedNode): PaneNode {
    if (n.kind === "leaf") {
      const id = this.nextId++;
      if (n.key) this.keys.set(id, n.key);
      return {
        kind: "leaf",
        id,
        ...(n.cwd !== undefined && { cwd: n.cwd }),
        ...(n.content !== undefined && { content: n.content }),
        ...(n.docId !== undefined && { docId: n.docId }),
      };
    }
    return {
      kind: "split",
      id: this.nextId++,
      dir: n.dir,
      children: n.children.map((c) => this.rebuild(c)),
    };
  }

  boot(host: Host, now: number): void {
    if (host.env) {
      const m = mergeWorkspace(this.local(), host.env, now);
      const st = m.states.get(SPACE);
      if (st) {
        this.disk = st;
        this.diskMeta = m.stateMeta[SPACE];
      }
      if (m.pushNeeded) this.push(host, now);
    }
    this.tabs = [];
    this.keys.clear();
    this.ledger.clear();
    for (const s of this.disk?.tabs ?? []) {
      if (s.kind === "terminal") {
        const tree = this.rebuild(s.tree);
        this.tabs.push({
          id: this.nextId++,
          spaceId: SPACE,
          kind: "terminal",
          title: "shell",
          activeLeafId: firstLeafId(tree),
          paneTree: tree,
          ...(s.customTitle !== undefined && { customTitle: s.customTitle }),
        } as Tab);
      } else if (s.kind === "notes") {
        this.tabs.push({
          id: this.nextId++,
          spaceId: SPACE,
          kind: "notes",
          docId: s.docId,
          title: s.title,
        } as Tab);
      }
    }
    this.lastLiveGen = host.gen;
  }
}

function world() {
  const tmux = new Tmux();
  return {
    host: new Host(),
    hq: new Device("hq", tmux),
    laptop: new Device("laptop", tmux),
  };
}

describe("two devices live: the 2026-09-03 incident", () => {
  it("an observed tab never overwrites the author's name and note split", () => {
    const { host, hq, laptop } = world();
    let t = 100;
    hq.create("k1", t++);
    hq.push(host, t++);
    // Laptop sees the tmux window and materializes it (clock 0).
    laptop.materialize(t++);
    // HQ names it and splits a note in, before the laptop's push lands.
    hq.rename("k1", "TESTING TAB", t++);
    hq.splitNote("k1", "note1", t++);
    hq.push(host, t++);
    // The laptop's push comes later on the wall clock: the incident.
    laptop.push(host, t++);
    expect(host.tab("k1")).toMatchObject({ customTitle: "TESTING TAB" });
    const k1 = host.tab("k1");
    expect(k1?.kind === "terminal" ? k1.tree.kind : null).toBe("split");
    // HQ pulls the laptop's generation: nothing of its own is lost.
    hq.livePull(host, t++);
    expect(hq.label("k1")).toBe("TESTING TAB");
    expect(hq.hasNotePane("k1")).toBe(true);
    // Laptop, live: the name arrives, and so does the split itself: the
    // terminal pane it already runs stays, the note pane appears beside it,
    // and no standalone note tab is raised for a doc shown as a pane.
    laptop.livePull(host, t++);
    expect(laptop.label("k1")).toBe("TESTING TAB");
    expect(laptop.hasNotePane("k1")).toBe(true);
    expect(laptop.tabs.filter((x) => x.kind === "notes")).toHaveLength(0);
    // Laptop, boot: unchanged, and its copy carries HQ's clocks, so its
    // next push changes nothing on the host.
    laptop.boot(host, t++);
    expect(laptop.hasNotePane("k1")).toBe(true);
    laptop.push(host, t++);
    const k1After = host.tab("k1");
    expect(k1After?.kind === "terminal" ? k1After.tree.kind : null).toBe(
      "split",
    );
    // And HQ's next boot keeps its tab.
    hq.boot(host, t++);
    expect(hq.label("k1")).toBe("TESTING TAB");
    expect(hq.hasNotePane("k1")).toBe(true);
  });

  it("a new tab named on one device shows up named on the other without a restart", () => {
    const { host, hq, laptop } = world();
    let t = 100;
    hq.create("k2", t++, "123");
    laptop.materialize(t++);
    laptop.push(host, t++);
    hq.push(host, t++);
    laptop.livePull(host, t++);
    expect(laptop.label("k2")).toBe("123");
    // The laptop's copy carries the author's clock, so its later push
    // changes nothing on the host.
    laptop.push(host, t++);
    expect(host.tab("k2")).toMatchObject({ customTitle: "123" });
  });

  it("a push that lost a race is repaired by the next live poll", () => {
    const { host, hq, laptop } = world();
    let t = 100;
    hq.create("a", t++, "A");
    laptop.create("b", t++, "B");
    // Both push from an empty host view: the second overwrites the first.
    hq.push(host, t++);
    host.env = null;
    laptop.push(host, t++);
    expect(host.tab("a")).toBeUndefined();
    // HQ has no local change, yet the live poll notices the host lacks
    // its tab and pushes.
    hq.livePull(host, t++);
    expect(host.tab("a")).toMatchObject({ customTitle: "A" });
    expect(host.tab("b")).toMatchObject({ customTitle: "B" });
  });
});

// ------------------------------------------------------------- fuzz harness

function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let x = a;
    x = Math.imul(x ^ (x >>> 15), x | 1);
    x ^= x + Math.imul(x ^ (x >>> 7), x | 61);
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}

type Authored = {
  createdAt: number;
  labelAt: number;
  label: string;
  splitAt: number;
  closedAt: number;
  /** Devices that ever split this tab; with exactly one, the other must
   * show the split LIVE (additive adoption), not just after boot. */
  splitBy: Set<string>;
};
// Names and structure are separate authored fields; a tab is alive iff any
// authored edit (create, rename, split) is newer than the last close.
const alive = (a: Authored) =>
  Math.max(a.createdAt, a.labelAt, a.splitAt) > a.closedAt;

function fuzzRun(seed: number): void {
  const rnd = mulberry32(seed);
  const pick = <T>(xs: readonly T[]): T => xs[Math.floor(rnd() * xs.length)];
  const { host, hq, laptop } = world();
  const devices = [hq, laptop];
  const authored = new Map<string, Authored>();
  // key -> time of the last authored note split; a split is authored
  // content and must survive like a name does (unless closed after).
  const keys: string[] = [];
  let t = 1000;
  let nextKey = 1;
  const steps = 12 + Math.floor(rnd() * 20);
  for (let i = 0; i < steps; i++) {
    const d = pick(devices);
    const op = pick([
      "create",
      "create",
      "rename",
      "rename",
      "close",
      "split",
      "churn",
      "churn",
      "materialize",
      "materialize",
      "push",
      "push",
      "pull",
      "pull",
      "boot",
    ] as const);
    t += 1 + Math.floor(rnd() * 3);
    switch (op) {
      case "create": {
        const key = `k${nextKey++}`;
        const label = rnd() < 0.5 ? `name-${key}` : "";
        d.create(key, t, label || undefined);
        keys.push(key);
        authored.set(key, {
          createdAt: t,
          labelAt: t,
          label,
          splitAt: -1,
          closedAt: -1,
          splitBy: new Set(),
        });
        break;
      }
      case "rename": {
        if (keys.length === 0) break;
        const key = pick(keys);
        const label = `r${t}`;
        const a = authored.get(key);
        if (a && d.rename(key, label, t))
          authored.set(key, { ...a, labelAt: t, label });
        break;
      }
      case "close": {
        if (keys.length === 0) break;
        const key = pick(keys);
        const a = authored.get(key);
        if (a && d.close(key, t)) authored.set(key, { ...a, closedAt: t });
        break;
      }
      case "split": {
        if (keys.length === 0) break;
        const key = pick(keys);
        const a = authored.get(key);
        if (a && d.splitNote(key, `note-${t}`, t))
          authored.set(key, {
            ...a,
            splitAt: t,
            splitBy: new Set([...a.splitBy, d.name]),
          });
        break;
      }
      case "churn":
        if (keys.length > 0) d.churn(pick(keys), t);
        break;
      case "materialize":
        d.materialize(t);
        break;
      case "push":
        d.push(host, t);
        break;
      case "pull":
        d.livePull(host, t);
        break;
      case "boot":
        d.boot(host, t);
        break;
    }
  }
  // Settle: everyone pushes, everyone polls twice.
  for (const d of devices) d.push(host, ++t);
  for (let r = 0; r < 2; r++) for (const d of devices) d.livePull(host, ++t);
  // (E) a split authored on exactly one device is visible on the other
  // LIVE, before any restart, whenever the tab is alive on both.
  for (const [key, a] of authored) {
    if (!alive(a) || a.splitBy.size !== 1 || a.splitAt < a.closedAt) continue;
    for (const d of devices) {
      if (d.find(key) === undefined) continue;
      expect(
        d.hasNotePane(key),
        `seed ${seed} key ${key} live split on ${d.name}`,
      ).toBe(true);
    }
  }
  // Then everyone boots and polls once more.
  for (const d of devices) d.boot(host, ++t);
  for (const d of devices) d.livePull(host, ++t);

  for (const [key, a] of authored) {
    const onHost = host.tab(key);
    const ctx = `seed ${seed} key ${key}`;
    if (!alive(a)) {
      expect(onHost, ctx).toBeUndefined();
      for (const d of devices)
        expect(d.find(key), `${ctx} on ${d.name}`).toBeUndefined();
      continue;
    }
    // (A) the latest authored label is on the host …
    expect(onHost, ctx).toBeDefined();
    expect(
      onHost?.kind === "terminal" ? (onHost.customTitle ?? "") : "?",
      ctx,
    ).toBe(a.label);
    // (B) … and on every device that shows the tab.
    for (const d of devices) {
      const l = d.label(key);
      if (l !== undefined) expect(l, `${ctx} on ${d.name}`).toBe(a.label);
    }
    // (D) an authored split is on the host and, after boot, on both.
    if (a.splitAt > a.closedAt) {
      expect(onHost?.kind === "terminal" ? onHost.tree.kind : null, ctx).toBe(
        "split",
      );
      for (const d of devices)
        expect(d.hasNotePane(key), `${ctx} split on ${d.name}`).toBe(true);
    }
  }
  // (C) convergence: same tab set and labels on both disks.
  const view = (d: Device) =>
    new Map(
      (d.disk?.tabs ?? []).map((s, i) => [
        tabIdentities(d.disk?.tabs ?? [])[i],
        s.kind === "terminal" ? (s.customTitle ?? "") : s.kind,
      ]),
    );
  expect(view(laptop), `seed ${seed}`).toEqual(view(hq));
}

describe("two devices live: seeded interleavings", () => {
  it("never loses an authored edit and always converges", () => {
    for (let seed = 1; seed <= 400; seed++) fuzzRun(seed);
  });
});
