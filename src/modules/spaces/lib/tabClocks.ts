// Per-tab identity and clocks (ADR-025). The cross-machine layout merge used
// to carry ONE stamp per space, so whichever device wrote last replaced every
// tab on the other: and a device that merely materialized a tmux window
// stamped just like a device that renamed it. Now every tab has a stable
// identity both devices derive without coordination, its own clock, and a
// tombstone when closed; stamping is a pure diff so it can be tested and so
// no caller can accidentally mint "now" for a tab it only observed.
import type { SerializedNode, SerializedTab } from "./serialize";
import type { SpaceState } from "./store";

export type TabClocks = Record<string, number>;

export type SpaceStateMeta = {
  /** Space-level clock: max over the tab clocks. Kept for 0.12.0 peers,
   * whose envelopes carry no per-tab map and merge on this alone. */
  at: number;
  /** identity -> clock of the last edit to the tab's STRUCTURE (pane tree,
   * doc id). Absent on envelopes written before ADR-025: every tab then
   * takes `at`. */
  tabs?: TabClocks;
  /** identity -> clock of the last edit to the tab's NAME. A rename here
   * and a split there are two edits to one tab; with one clock the later
   * one erased the other (fuzz seed 4). Absent: falls back to `tabs`. */
  titles?: TabClocks;
  /** identity -> time the tab was closed. Beats a tab whose clock is older. */
  gone?: TabClocks;
};

/** A closed tab's tombstone outlives any plausible offline window; pruned so
 * the map stays bounded. Ceiling: a device offline longer resurrects it. */
export const TAB_GONE_TTL_MS = 90 * 24 * 60 * 60 * 1000;

function collectLeafKeys(
  node: SerializedNode,
  out: { terminal: string[]; all: string[] },
): void {
  if (node.kind === "split") {
    for (const c of node.children) collectLeafKeys(c, out);
    return;
  }
  if (!node.key) return;
  out.all.push(node.key);
  if (!node.content) out.terminal.push(node.key);
}

/** Every restore key in a serialized tree, panes of any kind. */
export function serializedLeafKeys(node: SerializedNode): string[] {
  const out = { terminal: [], all: [] };
  collectLeafKeys(node, out);
  return out.all;
}

/** Restore keys are `rk-<base36 ms>-<rand>`: same length until 2059, so the
 * lexicographic minimum is the oldest pane. */
export function oldestKey(keys: readonly string[]): string | undefined {
  let best: string | undefined;
  for (const k of keys) if (best === undefined || k < best) best = k;
  return best;
}

/** Machine-independent identity of a serialized tab. A terminal tab is its
 * OLDEST terminal pane's restore key (that key already names the tmux
 * window `w-<key>`), which survives splitting notes or newer terminals in
 * at any position; doc ids are shared by the docs domain; file/url tabs are
 * their target. Kinds with nothing to key on are singletons per space. */
export function tabIdentity(tab: SerializedTab, index: number): string {
  switch (tab.kind) {
    case "terminal": {
      const keys = { terminal: [], all: [] };
      collectLeafKeys(tab.tree, keys);
      const k = oldestKey(keys.terminal.length > 0 ? keys.terminal : keys.all);
      return k ? `t:${k}` : `t#${index}`;
    }
    case "notes":
      return `n:${tab.docId}`;
    case "board":
      return `b:${tab.boardId}`;
    case "tasks":
      return `k:${tab.listId}`;
    case "editor":
    case "markdown":
      return `f:${tab.path}`;
    case "preview":
      return `u:${tab.url}`;
    default:
      return `s:${tab.kind}`;
  }
}

/** Identities for a tab list, made unique positionally on collision so two
 * accidental duplicates never merge into one. */
export function tabIdentities(tabs: readonly SerializedTab[]): string[] {
  const seen = new Set<string>();
  return tabs.map((t, i) => {
    let id = tabIdentity(t, i);
    if (seen.has(id)) id = `${id}#${i}`;
    seen.add(id);
    return id;
  });
}

/** Every tab's clock, filling in the space clock for pre-ADR-025 metas. An
 * unstamped side (no meta at all) is 0 everywhere: pre-sync data loses to
 * anything stamped, as ADR-023 already specified for the space. */
export function tabClocksOf(
  state: SpaceState | undefined,
  meta: SpaceStateMeta | undefined,
): TabClocks {
  const out: TabClocks = {};
  if (!state) return out;
  const ids = tabIdentities(state.tabs);
  for (const id of ids) out[id] = meta?.tabs?.[id] ?? meta?.at ?? 0;
  return out;
}

/** Lookup consulted for a NEW or CHANGED tab: the adoption ledger's clock when
 * this device is persisting something it adopted from a peer (0 for "only
 * observed to exist"), undefined for a real local edit. */
export type LedgerLookup = (
  identity: string,
  tab: SerializedTab,
) => number | undefined;

/** Key-order independent JSON: a merge composes tabs from two sources and
 * object key order must not read as a content difference. */
function canon(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canon).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const o = value as Record<string, unknown>;
    return `{${Object.keys(o)
      .sort()
      .filter((k) => o[k] !== undefined)
      .map((k) => `${JSON.stringify(k)}:${canon(o[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function stripDerived(node: SerializedNode): SerializedNode {
  if (node.kind === "split")
    return { ...node, children: node.children.map(stripDerived) };
  const { active: _active, cwd: _cwd, color: _color, ...rest } = node;
  return rest;
}

/** A tab's content for equality and tie-breaks: the serialized tab minus
 * everything a MACHINE writes on its own. Which pane is active, the cwd the
 * shell reports over OSC 7, and the auto-assigned pane accent all change
 * without anyone editing anything; counting them made a device that merely
 * showed a tab re-stamp it as its own edit (2026-09-03, second incident:
 * the laptop's copy of a split tab won on a colour). Structure, doc ids,
 * restore keys, pane labels and the tab name are the authored content. */
export function tabContentJson(tab: SerializedTab): string {
  if (tab.kind !== "terminal") return canon(tab);
  return canon({ ...tab, tree: stripDerived(tab.tree) });
}

/** The user-facing name a tab carries: a terminal's custom label ("" when
 * none), a doc tab's title. Kinds without a name yield "". */
export function tabTitle(tab: SerializedTab): string {
  if (tab.kind === "terminal") return tab.customTitle ?? "";
  return "title" in tab && typeof tab.title === "string" ? tab.title : "";
}

/** Content minus the name: what the structure clock covers. */
export function tabStructureJson(tab: SerializedTab): string {
  if (tab.kind === "terminal") {
    const { customTitle: _t, ...rest } = tab;
    return canon({ ...rest, tree: stripDerived(tab.tree) });
  }
  if ("title" in tab) {
    const { title: _t, ...rest } = tab;
    return canon(rest);
  }
  return canon(tab);
}

/** A tab with its name replaced (a merge composes the structure winner
 * with the title winner). */
export function withTitle(tab: SerializedTab, title: string): SerializedTab {
  if (tab.kind === "terminal") {
    const { customTitle: _t, ...rest } = tab;
    return title ? { ...rest, customTitle: title } : rest;
  }
  if ("title" in tab) return { ...tab, title };
  return tab;
}

/** Name clocks, falling back to the structure clock, then the space. */
export function titleClocksOf(
  state: SpaceState | undefined,
  meta: SpaceStateMeta | undefined,
): TabClocks {
  const out: TabClocks = {};
  if (!state) return out;
  for (const id of tabIdentities(state.tabs))
    out[id] = meta?.titles?.[id] ?? meta?.tabs?.[id] ?? meta?.at ?? 0;
  return out;
}

export function pruneGone(gone: TabClocks | undefined, now: number): TabClocks {
  const out: TabClocks = {};
  for (const [id, at] of Object.entries(gone ?? {})) {
    if (now - at <= TAB_GONE_TTL_MS) out[id] = at;
  }
  return out;
}

/** The stamping rule, as a pure function of previous disk state and the next
 * layout, per tab and per field (structure, name): an unchanged field keeps
 * its clock; a changed field takes the ledger's clock when the ledger knows
 * the tab, else `now`; a new tab takes the ledger's clock or `now` for both;
 * a tab that vanished gets a tombstone; the space clock is the max. */
export function stampTabs(
  prev: SpaceState | undefined,
  prevMeta: SpaceStateMeta | undefined,
  next: SpaceState,
  now: number,
  ledger: LedgerLookup,
): SpaceStateMeta {
  const prevIds = prev ? tabIdentities(prev.tabs) : [];
  const prevTab = new Map<string, SerializedTab>();
  prevIds.forEach((id, i) => {
    if (prev) prevTab.set(id, prev.tabs[i]);
  });
  const prevClocks = tabClocksOf(prev, prevMeta);
  const prevTitleClocks = titleClocksOf(prev, prevMeta);
  const nextIds = tabIdentities(next.tabs);

  const tabs: TabClocks = {};
  const titles: TabClocks = {};
  nextIds.forEach((id, i) => {
    const tab = next.tabs[i];
    const before = prevTab.get(id);
    if (before === undefined) {
      const c = ledger(id, tab) ?? now;
      tabs[id] = c;
      titles[id] = c;
      return;
    }
    const structChanged = tabStructureJson(before) !== tabStructureJson(tab);
    const titleChanged = tabTitle(before) !== tabTitle(tab);
    const led = structChanged || titleChanged ? ledger(id, tab) : undefined;
    tabs[id] = structChanged ? (led ?? now) : (prevClocks[id] ?? 0);
    titles[id] = titleChanged ? (led ?? now) : (prevTitleClocks[id] ?? 0);
  });

  const gone = pruneGone(prevMeta?.gone, now);
  const alive = new Set(nextIds);
  for (const id of prevIds) if (!alive.has(id)) gone[id] = now;
  for (const id of alive) delete gone[id];

  let at = prevMeta?.at ?? 0;
  for (const c of Object.values(tabs)) if (c > at) at = c;
  for (const c of Object.values(titles)) if (c > at) at = c;
  for (const c of Object.values(gone)) if (c > at) at = c;
  return { at, tabs, titles, gone };
}
