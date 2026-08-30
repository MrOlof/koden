import { useCallback, useEffect, useRef } from "react";
import { usePreferencesStore } from "@/modules/settings/preferences";
import type { Tab } from "@/modules/tabs";
import { usePaneTitleStore } from "@/modules/terminal/lib/paneTitles";
import { captureLeafForRestore } from "@/modules/terminal/lib/useTerminalSession";
import {
  clearScrollbackSnapshots,
  leafRestoreKey,
  saveScrollbackSnapshots,
} from "./scrollbackStore";
import { isSerializableTab, serializeTabs } from "./serialize";
import { saveState } from "./store";
import { useSpaces } from "./useSpaces";

const DEBOUNCE_MS = 3000;
// Scrollback capture is a buffer copy per live terminal and PTY output never
// touches React state, so it runs on its own slow cadence plus every close
// path (hidden window, unload, unmount); unchanged buffers cost no IPC.
const SCROLLBACK_INTERVAL_MS = 8000;

type Snapshot = { tabs: Tab[]; activeId: number; activeSpaceId: string };

type Params = Snapshot & {
  /** Gate writes until boot hydration finished, so restore never round-trips. */
  enabled: boolean;
};

type LastWrite = { json: string; activeTabIndex: number };

export function useSpacePersistence({
  tabs,
  activeId,
  activeSpaceId,
  enabled,
}: Params) {
  const last = useRef<Map<string, LastWrite>>(new Map());
  const seeded = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latest = useRef<Snapshot>({ tabs, activeId, activeSpaceId });
  latest.current = { tabs, activeId, activeSpaceId };
  const scrollbackOff = useRef(false);

  // Seed each space's last-known active index from disk so the first flush
  // preserves it for spaces the user never opens (empty json forces one write
  // with the correct index rather than clobbering it to 0).
  if (enabled && !seeded.current) {
    seeded.current = true;
    for (const [id, idx] of Object.entries(
      useSpaces.getState().initialActiveIndex,
    )) {
      last.current.set(id, { json: "", activeTabIndex: idx });
    }
  }

  const flush = useCallback((snap: Snapshot) => {
    const groups = new Map<string, Tab[]>();
    for (const t of snap.tabs) {
      const arr = groups.get(t.spaceId);
      if (arr) arr.push(t);
      else groups.set(t.spaceId, [t]);
    }

    for (const [spaceId, group] of groups) {
      // Persist only unlocked per-pane titles/colors; locked panes (Director,
      // agents) are recreated on boot, not restored from disk.
      const serialized = serializeTabs(
        group,
        (leafId) => {
          const e = usePaneTitleStore.getState().titles[leafId];
          return e && !e.locked ? { label: e.label, color: e.color } : undefined;
        },
        leafRestoreKey,
      );
      const prev = last.current.get(spaceId);
      let activeTabIndex = prev?.activeTabIndex ?? 0;
      if (spaceId === snap.activeSpaceId) {
        const idx = group
          .filter(isSerializableTab)
          .findIndex((t) => t.id === snap.activeId);
        if (idx >= 0) activeTabIndex = idx;
      }
      const json = JSON.stringify(serialized);
      if (
        prev &&
        prev.json === json &&
        prev.activeTabIndex === activeTabIndex
      ) {
        continue;
      }
      last.current.set(spaceId, { json, activeTabIndex });
      void saveState(spaceId, { tabs: serialized, activeTabIndex });
    }
  }, []);

  const flushScrollback = useCallback((snap: Snapshot) => {
    const cap = usePreferencesStore.getState().terminalScrollbackRestoreLines;
    if (cap <= 0) {
      // Off wipes what an earlier setting left on disk, once.
      if (!scrollbackOff.current) {
        scrollbackOff.current = true;
        void clearScrollbackSnapshots().catch(() => {});
      }
      return;
    }
    scrollbackOff.current = false;
    void saveScrollbackSnapshots(snap.tabs, (leafId) =>
      captureLeafForRestore(leafId, cap),
    );
  }, []);

  useEffect(() => {
    if (!enabled) return;
    const snap: Snapshot = { tabs, activeId, activeSpaceId };
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      timer.current = null;
      flush(snap);
    }, DEBOUNCE_MS);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [tabs, activeId, activeSpaceId, enabled, flush]);

  useEffect(() => {
    if (!enabled) return;
    const id = setInterval(
      () => flushScrollback(latest.current),
      SCROLLBACK_INTERVAL_MS,
    );
    return () => clearInterval(id);
  }, [enabled, flushScrollback]);

  useEffect(() => {
    if (!enabled) return;
    const flushAll = () => {
      flush(latest.current);
      flushScrollback(latest.current);
    };
    const onHidden = () => {
      if (document.visibilityState === "hidden") flushAll();
    };
    const onBlur = () => flush(latest.current);
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("blur", onBlur);
    window.addEventListener("beforeunload", flushAll);
    return () => {
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("beforeunload", flushAll);
      flushAll();
    };
  }, [enabled, flush, flushScrollback]);
}
