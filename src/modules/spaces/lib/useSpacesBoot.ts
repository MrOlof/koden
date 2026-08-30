import { useEffect, useRef } from "react";
import { native } from "@/modules/ai/lib/native";
import { brainRecoveredPanes, brainResumePlan } from "@/modules/brain/lib/bindings";
import {
  matchRecoveredPanes,
  resumeBaseLaunch,
  type RestoredLeafRef,
} from "@/modules/brain/lib/resumeCards";
import { markRecoveredPaneConsumed } from "@/modules/brain/lib/useRecoveredPanes";
import { usePreferencesStore } from "@/modules/settings/preferences";
import type { Tab } from "@/modules/tabs";
import { DEFAULT_SPACE_ID } from "@/modules/tabs/lib/useTabs";
import { usePaneTitleStore } from "@/modules/terminal/lib/paneTitles";
import { isLeaf, type PaneNode } from "@/modules/terminal/lib/panes";
import { preloadRestoredBuffer } from "@/modules/terminal/lib/rendererPool";
import {
  submitToLeaf,
  whenSessionReady,
} from "@/modules/terminal/lib/useTerminalSession";
import {
  loadScrollbackSnapshots,
  type RestorableLeaf,
  seedLeafRestoreKey,
  snapshotEntryKey,
} from "./scrollbackStore";
import { freshTerminalTab, hydrateTabs } from "./serialize";
import { loadAll, saveActiveId, saveSpacesList, type SpaceMeta } from "./store";
import { useSpaces } from "./useSpaces";

type Params = {
  ready: boolean;
  launchCwd: string | null;
  home: string | null;
  allocId: () => number;
  replaceTabs: (tabs: Tab[], activeId: number) => void;
  markBooted: () => void;
  setActiveSpaceForNewTabs: (id: string) => void;
};

// Prefs hydrate in one async set from DEFAULTS; restore and auto-resume must
// read the stored values, not the defaults, so boot waits (bounded) for them.
const PREFS_WAIT_MS = 3000;
// A restored shell has to spawn, run its profile and print a prompt before the
// resume command can be typed; the interactive launch paths use 4s, boot is
// slower because every warmed tab spawns at once.
const AUTO_RESUME_READY_MS = 15_000;

function uniqueCwds(tabs: Tab[]): string[] {
  const set = new Set<string>();
  const walk = (n: PaneNode) => {
    if (isLeaf(n)) {
      if (n.cwd) set.add(n.cwd);
      return;
    }
    for (const c of n.children) walk(c);
  };
  for (const t of tabs) if (t.kind === "terminal") walk(t.paneTree);
  return [...set];
}

function shellLeafRefs(tabs: Tab[]): RestoredLeafRef[] {
  const out: RestoredLeafRef[] = [];
  const walk = (tabId: number, n: PaneNode) => {
    if (isLeaf(n)) {
      if (n.content === undefined) out.push({ leafId: n.id, tabId, cwd: n.cwd });
      return;
    }
    for (const c of n.children) walk(tabId, c);
  };
  for (const t of tabs) {
    if (t.kind === "terminal" && !t.private && !t.blocks) walk(t.id, t.paneTree);
  }
  return out;
}

function whenPrefsHydrated(): Promise<void> {
  if (usePreferencesStore.getState().hydrated) return Promise.resolve();
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      unsub();
      resolve();
    };
    const timer = setTimeout(finish, PREFS_WAIT_MS);
    const unsub = usePreferencesStore.subscribe((s) => {
      if (s.hydrated) finish();
    });
  });
}

/** Auto-resume (pref): type the Tier-2 resume plan into every restored shell
 * leaf whose recovered pane matches by cwd. Matched tabs come back warm: a
 * cold tab never mounts, so its PTY would never spawn and the command would
 * have nowhere to go. Fail-open at every step: no panes, no match, no plan or
 * an IPC error just means the card path handles it instead. */
async function scheduleAutoResume(tabs: Tab[]): Promise<Tab[]> {
  const panes = await brainRecoveredPanes().catch(() => []);
  if (panes.length === 0) return tabs;
  const matches = matchRecoveredPanes(panes, shellLeafRefs(tabs));
  if (matches.length === 0) return tabs;
  const base = resumeBaseLaunch();
  const warm = new Set<number>();
  for (const m of matches) {
    const plan = await brainResumePlan(m.pane.key, base).catch(() => null);
    if (plan?.tier !== "tier2") continue;
    markRecoveredPaneConsumed(m.pane.key);
    warm.add(m.tabId);
    const command = plan.command;
    void whenSessionReady(m.leafId, AUTO_RESUME_READY_MS).then(() =>
      submitToLeaf(m.leafId, command),
    );
  }
  if (warm.size === 0) return tabs;
  return tabs.map((t) => (warm.has(t.id) ? { ...t, cold: false } : t));
}

export function useSpacesBoot({
  ready,
  launchCwd,
  home,
  allocId,
  replaceTabs,
  markBooted,
  setActiveSpaceForNewTabs,
}: Params) {
  const done = useRef(false);

  useEffect(() => {
    if (!ready || done.current) return;
    done.current = true;

    void (async () => {
      try {
        const [{ spaces, activeId, states }] = await Promise.all([
          loadAll(),
          whenPrefsHydrated(),
        ]);
        const prefs = usePreferencesStore.getState();
        const restoreLines = prefs.terminalScrollbackRestoreLines;
        const snapshotsP =
          restoreLines > 0
            ? loadScrollbackSnapshots().catch((e) => {
                console.warn("[koden] scrollback load failed:", e);
                return new Map<string, string>();
              })
            : Promise.resolve(new Map<string, string>());

        if (spaces.length === 0) {
          const root = launchCwd ?? home ?? null;
          const meta: SpaceMeta = {
            id: DEFAULT_SPACE_ID,
            name: "Default",
            root,
            env: { kind: "local" },
            createdAt: Date.now(),
            updatedAt: Date.now(),
          };
          await saveSpacesList([meta]);
          await saveActiveId(DEFAULT_SPACE_ID);
          setActiveSpaceForNewTabs(DEFAULT_SPACE_ID);
          useSpaces.getState().hydrate([meta], DEFAULT_SPACE_ID);
          return;
        }

        let restored: Tab[] = [];
        const seededLeaves: RestorableLeaf[] = [];
        for (const space of spaces) {
          const st = states.get(space.id);
          if (!st) continue;
          restored.push(
            ...hydrateTabs(
              st.tabs,
              space.id,
              allocId,
              (leafId, title, color) =>
                usePaneTitleStore
                  .getState()
                  .setPaneTitle(leafId, title ?? "", false, color),
              (leafId, key) => {
                seedLeafRestoreKey(leafId, key);
                seededLeaves.push({ leafId, spaceId: space.id, key });
              },
            ),
          );
        }

        const active =
          activeId && spaces.some((s) => s.id === activeId)
            ? activeId
            : spaces[0].id;
        setActiveSpaceForNewTabs(active);

        // Active space must never be empty, else its tab list shows nothing.
        if (!restored.some((t) => t.spaceId === active)) {
          restored.push(freshTerminalTab(active, launchCwd ?? home, allocId));
        }

        await Promise.allSettled(
          uniqueCwds(restored).map((cwd) => native.workspaceAuthorize(cwd)),
        );

        // Queue each saved buffer for its leaf's first bind (before any PTY
        // byte reaches the grid). Cold tabs hold theirs until first activation.
        const snapshots = await snapshotsP;
        for (const leaf of seededLeaves) {
          const text = snapshots.get(snapshotEntryKey(leaf.spaceId, leaf.key));
          if (text) preloadRestoredBuffer(leaf.leafId, text);
        }

        if (prefs.autoResumeAgents) {
          restored = await scheduleAutoResume(restored);
        }

        const initialActiveIndex: Record<string, number> = {};
        for (const [id, st] of states)
          initialActiveIndex[id] = st.activeTabIndex;
        useSpaces.getState().hydrate(spaces, active, initialActiveIndex);

        const inActive = restored.filter((t) => t.spaceId === active);
        const idx = states.get(active)?.activeTabIndex ?? 0;
        const activeTab = inActive[idx] ?? inActive[0] ?? restored[0];
        replaceTabs(restored, activeTab.id);
      } catch (e) {
        console.error("[koden] spaces boot failed:", e);
      } finally {
        markBooted();
      }
    })();
  }, [
    ready,
    launchCwd,
    home,
    allocId,
    replaceTabs,
    markBooted,
    setActiveSpaceForNewTabs,
  ]);
}
