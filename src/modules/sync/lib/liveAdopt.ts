// Live additive adoption (ADR-024, the "two devices at once" layer on top of
// ADR-023): while both machines run, a doc created on one materializes on the
// other within a poll cycle instead of at next boot. Deliberately ADDITIVE
// only — new doc tabs and doc-tab renames. Never closes, never reorders,
// never rewrites pane trees of a live UI; full structural reconcile stays
// boot's job (mergeWorkspace), so every ADR-023 invariant is untouched.
//
// Terminals need no live layer: their existence is tmux truth and the
// remote-space adoption loop (App.syncRemoteSpace) already materializes new
// windows within 15 s.

import type { SerializedNode } from "@/modules/spaces/lib/serialize";
import type { SpaceState, SpaceStateMeta } from "@/modules/spaces/lib/store";
import {
  oldestKey,
  tabClocksOf,
  tabIdentities,
} from "@/modules/spaces/lib/tabClocks";
import type { Tab } from "@/modules/tabs/lib/useTabs";
import { isLeaf, type PaneNode } from "@/modules/terminal/lib/panes";

export type LiveDocKind = "notes" | "board" | "tasks";

export type LiveAdopters = {
  /** Current tabs, all spaces (the App's live tabs array). */
  listTabs: () => readonly Tab[];
  /** Create a doc tab in a space without stealing focus (useTabs.adoptDocTab). */
  adoptDocTab: (
    spaceId: string,
    kind: LiveDocKind,
    docId: string,
    title: string,
  ) => void;
  /** Retitle a doc tab. */
  renameTab: (tabId: number, title: string) => void;
  /** Set (or clear with "") a terminal tab's user label (ADR-025). */
  setCustomTitle: (tabId: number, title: string) => void;
  /** Restore key of a live terminal leaf, if it has one. */
  leafKey: (leafId: number) => string | undefined;
};

let adopters: LiveAdopters | null = null;

/** App hands its tab operations over once; the engine consumes them. */
export function registerLiveAdopters(a: LiveAdopters): void {
  adopters = a;
}

export function getLiveAdopters(): LiveAdopters | null {
  return adopters;
}

export type LiveDoc = { kind: LiveDocKind; id: string; title: string };

function docLeavesOfSerialized(node: SerializedNode, out: LiveDoc[]): void {
  if (node.kind === "split") {
    for (const c of node.children) docLeavesOfSerialized(c, out);
    return;
  }
  if (node.content && node.docId) {
    out.push({
      kind: node.content === "note" ? "notes" : "tasks",
      id: node.docId,
      title: node.title || (node.content === "note" ? "Notes" : "Tasks"),
    });
  }
}

/** Every doc entity a persisted space state references: doc tabs and doc
 * panes inside terminal trees. Doc panes count for existence (so the other
 * device doesn't re-create something it already shows as a split) but they
 * arrive HERE as tabs — split-injection into a live layout is boot's job. */
export function docsInRemoteState(state: SpaceState): LiveDoc[] {
  const out: LiveDoc[] = [];
  for (const t of state.tabs) {
    if (t.kind === "notes")
      out.push({ kind: "notes", id: t.docId, title: t.title });
    else if (t.kind === "board")
      out.push({ kind: "board", id: t.boardId, title: t.title });
    else if (t.kind === "tasks")
      out.push({ kind: "tasks", id: t.listId, title: t.title });
    else if (t.kind === "terminal") docLeavesOfSerialized(t.tree, out);
  }
  return out;
}

function docLeavesOfPaneTree(tree: PaneNode, out: Map<string, number>): void {
  if (isLeaf(tree)) {
    if (tree.content && tree.docId) {
      out.set(
        `${tree.content === "note" ? "notes" : "tasks"}:${tree.docId}`,
        -1,
      );
    }
    return;
  }
  for (const c of tree.children) docLeavesOfPaneTree(c, out);
}

export type LiveDocPlan = {
  create: LiveDoc[];
  rename: { tabId: number; title: string }[];
};

/** Diff one space's remote persisted state against the live local tabs.
 * Additive semantics: create what is missing, retitle doc TABS whose remote
 * title differs, touch nothing else. Values in the local map: the tab id for
 * doc tabs (renameable), -1 for doc panes (existence only). */
export function planLiveDocAdoption(
  spaceId: string,
  localTabs: readonly Tab[],
  remote: SpaceState,
): LiveDocPlan {
  const local = new Map<string, number>();
  for (const t of localTabs) {
    if (t.spaceId !== spaceId) continue;
    if (t.kind === "notes") local.set(`notes:${t.docId}`, t.id);
    else if (t.kind === "board") local.set(`board:${t.boardId}`, t.id);
    else if (t.kind === "tasks") local.set(`tasks:${t.listId}`, t.id);
    else if (t.kind === "terminal") docLeavesOfPaneTree(t.paneTree, local);
  }
  const create: LiveDoc[] = [];
  const rename: { tabId: number; title: string }[] = [];
  const seen = new Set<string>();
  for (const d of docsInRemoteState(remote)) {
    const key = `${d.kind}:${d.id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const tabId = local.get(key);
    if (tabId === undefined) {
      create.push(d);
    } else if (tabId >= 0 && d.title) {
      const t = localTabs.find((x) => x.id === tabId);
      if (t && t.title !== d.title) rename.push({ tabId, title: d.title });
    }
  }
  return { create, rename };
}

export type LiveRename = {
  tabId: number;
  /** tabClocks identity, for the adoption ledger. */
  identity: string;
  kind: "terminal" | "doc";
  /** New label; "" clears a terminal's custom title. */
  title: string;
  /** The remote clock the rename carries. */
  clock: number;
  before: string;
};

function collectLiveKeys(
  tree: PaneNode,
  leafKey: (leafId: number) => string | undefined,
  out: { terminal: string[]; all: string[] },
): void {
  if (!isLeaf(tree)) {
    for (const c of tree.children) collectLiveKeys(c, leafKey, out);
    return;
  }
  const k = leafKey(tree.id);
  if (!k) return;
  out.all.push(k);
  if (!tree.content) out.terminal.push(k);
}

/** Identity of a LIVE tab, matching tabClocks.tabIdentity of its serialized
 * form (oldest terminal pane's key). Null for tabs without one yet. */
export function liveTabIdentity(
  tab: Tab,
  leafKey: (leafId: number) => string | undefined,
): string | null {
  switch (tab.kind) {
    case "terminal": {
      const keys = { terminal: [], all: [] };
      collectLiveKeys(tab.paneTree, leafKey, keys);
      const k = oldestKey(keys.terminal.length > 0 ? keys.terminal : keys.all);
      return k ? `t:${k}` : null;
    }
    case "notes":
      return `n:${tab.docId}`;
    case "board":
      return `b:${tab.boardId}`;
    case "tasks":
      return `k:${tab.listId}`;
    default:
      return null;
  }
}

function liveLabel(tab: Tab): string | null {
  if (tab.kind === "terminal") return tab.customTitle ?? "";
  if (tab.kind === "notes" || tab.kind === "board" || tab.kind === "tasks")
    return tab.title;
  return null;
}

/** Renames a remote layout carries for tabs this device already shows,
 * whose remote clock beats the clock on THIS device's disk (ADR-025).
 * Additive like everything live: labels only, never structure. A tab not
 * yet on local disk is skipped (it is a fresh local edit, clocked now). */
export function planLiveRenames(
  spaceId: string,
  localTabs: readonly Tab[],
  localState: SpaceState | undefined,
  localMeta: SpaceStateMeta | undefined,
  remoteState: SpaceState,
  remoteMeta: SpaceStateMeta | undefined,
  leafKey: (leafId: number) => string | undefined,
): LiveRename[] {
  const lClocks = tabClocksOf(localState, localMeta);
  const rClocks = tabClocksOf(remoteState, remoteMeta);
  const rIds = tabIdentities(remoteState.tabs);
  const remoteLabel = new Map<string, string>();
  rIds.forEach((id, i) => {
    const t = remoteState.tabs[i];
    if (t.kind === "terminal") remoteLabel.set(id, t.customTitle ?? "");
    else if (t.kind === "notes" || t.kind === "board" || t.kind === "tasks")
      remoteLabel.set(id, t.title);
  });
  const out: LiveRename[] = [];
  for (const t of localTabs) {
    if (t.spaceId !== spaceId) continue;
    const id = liveTabIdentity(t, leafKey);
    if (id === null) continue;
    const lc = lClocks[id];
    if (lc === undefined) continue;
    const want = remoteLabel.get(id);
    if (want === undefined) continue;
    const have = liveLabel(t);
    if (have === null || have === want) continue;
    const rc = rClocks[id] ?? 0;
    if (rc <= lc) continue;
    out.push({
      tabId: t.id,
      identity: id,
      kind: t.kind === "terminal" ? "terminal" : "doc",
      title: want,
      clock: rc,
      before: have,
    });
  }
  return out;
}
