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
import type { SpaceState } from "@/modules/spaces/lib/store";
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
  /** Retitle a tab (doc tabs only here). */
  renameTab: (tabId: number, title: string) => void;
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
