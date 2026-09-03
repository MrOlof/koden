// Per-tab layout merge (ADR-025). Replaces the per-space snapshot LWW: two
// devices now converge tab by tab, so a rename here and a materialization
// there are not a conflict, and a lost tab is at most one field of one tab
// under a genuinely simultaneous edit.
import type {
  SerializedNode,
  SerializedTab,
} from "@/modules/spaces/lib/serialize";
import type { SpaceState } from "@/modules/spaces/lib/store";
import {
  pruneGone,
  type SpaceStateMeta,
  type TabClocks,
  tabClocksOf,
  tabContentJson,
  tabIdentities,
} from "@/modules/spaces/lib/tabClocks";

export type TabChange = {
  id: string;
  kind: "added" | "replaced" | "removed";
  before?: SerializedTab;
  after?: SerializedTab;
};

export type StateMergeResult = {
  state: SpaceState;
  meta: SpaceStateMeta;
  /** The merged layout differs from the local input. */
  changed: boolean;
  /** Local holds something the remote lacks or loses on clock: push. */
  localNewer: boolean;
  /** What adoption did to local tabs, for the journal and the live layer. */
  changes: TabChange[];
};

function same(a: SerializedTab, b: SerializedTab): boolean {
  return tabContentJson(a) === tabContentJson(b);
}

function paneDocIds(node: SerializedNode, out: Set<string>): void {
  if (node.kind === "split") {
    for (const c of node.children) paneDocIds(c, out);
    return;
  }
  if (node.docId) out.add(node.docId);
}

/** A doc shown as a PANE on one device arrives on the other as a live tab
 * (ADR-024: split-injection is boot's job). Once the boot merge brings the
 * split in, the standalone tab is a duplicate: the pane wins, deterministic
 * on both sides. */
function dropDocTabsShownAsPanes(tabs: SerializedTab[]): SerializedTab[] {
  const inPanes = new Set<string>();
  for (const t of tabs) if (t.kind === "terminal") paneDocIds(t.tree, inPanes);
  if (inPanes.size === 0) return tabs;
  return tabs.filter((t) => {
    if (t.kind === "notes") return !inPanes.has(t.docId);
    if (t.kind === "tasks") return !inPanes.has(t.listId);
    return true;
  });
}

/** Equal clocks with different content must resolve the same way on both
 * devices, or each keeps its own copy forever. Content order is arbitrary
 * but deterministic. */
function remoteWinsTie(l: SerializedTab, r: SerializedTab): boolean {
  return tabContentJson(r) > tabContentJson(l);
}

export function mergeSpaceState(
  local: SpaceState | undefined,
  localMeta: SpaceStateMeta | undefined,
  remote: SpaceState | undefined,
  remoteMeta: SpaceStateMeta | undefined,
  now: number = Date.now(),
): StateMergeResult {
  const lClocks = tabClocksOf(local, localMeta);
  const rClocks = tabClocksOf(remote, remoteMeta);
  const gone: TabClocks = pruneGone(localMeta?.gone, now);
  for (const [id, at] of Object.entries(pruneGone(remoteMeta?.gone, now))) {
    gone[id] = Math.max(gone[id] ?? 0, at);
  }

  const lIds = local ? tabIdentities(local.tabs) : [];
  const rIds = remote ? tabIdentities(remote.tabs) : [];
  const lTab = new Map<string, SerializedTab>();
  const rTab = new Map<string, SerializedTab>();
  lIds.forEach((id, i) => {
    if (local) lTab.set(id, local.tabs[i]);
  });
  rIds.forEach((id, i) => {
    if (remote) rTab.set(id, remote.tabs[i]);
  });

  const picked: SerializedTab[] = [];
  const clocks: TabClocks = {};
  const changes: TabChange[] = [];
  let localNewer = false;

  const decide = (id: string): SerializedTab | null => {
    const l = lTab.get(id);
    const r = rTab.get(id);
    const lc = l ? (lClocks[id] ?? 0) : -1;
    const rc = r ? (rClocks[id] ?? 0) : -1;
    const closedAt = gone[id];
    if (closedAt !== undefined && closedAt > Math.max(lc, rc)) {
      if (l) changes.push({ id, kind: "removed", before: l });
      return null;
    }
    if (l && !r) {
      clocks[id] = lc;
      localNewer = true;
      return l;
    }
    if (r && !l) {
      clocks[id] = rc;
      changes.push({ id, kind: "added", after: r });
      return r;
    }
    if (!l || !r) return null;
    const remoteWins = rc > lc || (rc === lc && remoteWinsTie(l, r));
    if (remoteWins) {
      clocks[id] = rc;
      if (!same(l, r))
        changes.push({ id, kind: "replaced", before: l, after: r });
      return r;
    }
    clocks[id] = lc;
    if (!same(l, r)) localNewer = true;
    return l;
  };

  // Local order first, then remote-only tabs in remote order (ADR-023:
  // order stays local; unseen entries append).
  for (const id of lIds) {
    const t = decide(id);
    if (t) picked.push(t);
  }
  for (const id of rIds) {
    if (lTab.has(id)) continue;
    const t = decide(id);
    if (t) picked.push(t);
  }
  const tabs = dropDocTabsShownAsPanes(picked);
  if (tabs.length !== picked.length) {
    const kept = new Set(tabIdentities(tabs));
    for (const id of tabIdentities(picked)) {
      if (kept.has(id)) continue;
      delete clocks[id];
      const before = lTab.get(id);
      if (before) changes.push({ id, kind: "removed", before });
    }
  }
  for (const id of Object.keys(clocks)) delete gone[id];

  let at = Math.max(localMeta?.at ?? 0, remoteMeta?.at ?? 0);
  for (const c of Object.values(clocks)) if (c > at) at = c;
  for (const c of Object.values(gone)) if (c > at) at = c;

  const srcActive = local?.activeTabIndex ?? remote?.activeTabIndex ?? 0;
  const activeTabIndex =
    tabs.length === 0 ? 0 : Math.min(Math.max(srcActive, 0), tabs.length - 1);
  const state: SpaceState = { tabs, activeTabIndex };
  const changed =
    !local ||
    local.tabs.length !== tabs.length ||
    local.tabs.some((t, i) => !same(t, tabs[i]));
  return {
    state,
    meta: { at, tabs: clocks, gone },
    changed,
    localNewer,
    changes,
  };
}
