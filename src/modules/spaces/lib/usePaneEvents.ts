// Impure shell for M2.8 remote agent status: one `tail -F` watcher per ssh
// host with terminal tabs (ssh_pane_events_start), events joined to tabs via
// the pane -> window-name -> restore-key chain and escalated into the tab
// status store. All decisions live in paneEvents.ts; this hook only owns
// lifecycle: start/stop per host, reconnect on drop, and the pane-map cache.

import { useTabStatusStore } from "@/modules/tabs";
import { leafIds, type PaneNode } from "@/modules/terminal";
import { Channel, invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef } from "react";
import { spaceEnv } from "./envSwitch";
import { paneEventStep, paneKeyMap, parsePaneEventLine } from "./paneEvents";
import type { RemoteWindow } from "./remoteSessions";
import { peekLeafRestoreKey } from "./scrollbackStore";
import type { SpaceMeta } from "./store";
import { tmuxKeyFor } from "./tmuxKey";

type TailEvent = { kind: "line"; line: string } | { kind: "end" };

type TabLike = {
  id: number;
  spaceId: string;
  kind: string;
  paneTree?: PaneNode;
};

type Watcher = {
  id: number | null;
  gone: boolean;
  timer: number | null;
};

type PaneMapEntry = {
  at: number;
  refreshing: Promise<void> | null;
  map: Map<string, string>;
};

const RECONNECT_MS = 5_000;
const DEAD_HOST_RETRY_MS = 30_000;
// tmux reuses %N ids as windows die and spawn, so the join map is a cache,
// not a fact: expire it, and refresh on unknown panes (rate-limited so a
// foreign writer spamming bogus pane ids can't turn into an ssh probe loop).
const PANE_MAP_TTL_MS = 60_000;
const PANE_MAP_MIN_REFRESH_MS = 10_000;

export function usePaneEventsBridge(
  tabs: readonly TabLike[],
  spaces: readonly SpaceMeta[],
): void {
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const spacesRef = useRef(spaces);
  spacesRef.current = spaces;

  const watchers = useRef(new Map<string, Watcher>());
  const paneMaps = useRef(new Map<string, PaneMapEntry>());
  const midTurn = useRef(new Map<string, boolean>());

  // A host is watched while any of its ssh Spaces has a terminal tab, active
  // Space or not: background tabs still need their dots.
  const desiredKey = useMemo(() => {
    const withTabs = new Set(
      tabs.filter((t) => t.kind === "terminal").map((t) => t.spaceId),
    );
    const hosts = new Set<string>();
    for (const s of spaces) {
      if (!withTabs.has(s.id)) continue;
      const env = spaceEnv(s);
      if (env.kind === "ssh" && tmuxKeyFor(s)) hosts.add(env.host);
    }
    return [...hosts].sort().join("\n");
  }, [tabs, spaces]);

  useEffect(() => {
    const desired = new Set(desiredKey ? desiredKey.split("\n") : []);

    const refreshPaneMap = (host: string): Promise<void> => {
      const existing = paneMaps.current.get(host);
      if (existing?.refreshing) return existing.refreshing;
      const run = (async () => {
        const tmuxKeys = new Set<string>();
        for (const s of spacesRef.current) {
          const env = spaceEnv(s);
          if (env.kind !== "ssh" || env.host !== host) continue;
          const tk = tmuxKeyFor(s);
          if (tk) tmuxKeys.add(tk);
        }
        const merged = new Map<string, string>();
        await Promise.all(
          [...tmuxKeys].map(async (spaceKey) => {
            try {
              const ws = await invoke<RemoteWindow[]>("ssh_tmux_windows", {
                host,
                spaceKey,
              });
              for (const [pane, key] of paneKeyMap(ws)) merged.set(pane, key);
            } catch {
              // host briefly unreachable: keep whatever we knew
            }
          }),
        );
        paneMaps.current.set(host, {
          at: Date.now(),
          refreshing: null,
          map: merged,
        });
      })();
      paneMaps.current.set(host, {
        at: existing?.at ?? 0,
        refreshing: run,
        map: existing?.map ?? new Map(),
      });
      return run;
    };

    const resolveKey = async (
      host: string,
      pane: string,
    ): Promise<string | null> => {
      let entry = paneMaps.current.get(host);
      const now = Date.now();
      const stale = !entry || now - entry.at > PANE_MAP_TTL_MS;
      const unknown = !entry?.map.has(pane);
      if (
        (stale || unknown) &&
        (!entry || now - entry.at > PANE_MAP_MIN_REFRESH_MS)
      ) {
        await refreshPaneMap(host);
        entry = paneMaps.current.get(host);
      }
      return entry?.map.get(pane) ?? null;
    };

    const tabIdForKey = (key: string): number | null => {
      for (const t of tabsRef.current) {
        if (t.kind !== "terminal" || !t.paneTree) continue;
        for (const lid of leafIds(t.paneTree)) {
          if (peekLeafRestoreKey(lid) === key) return t.id;
        }
      }
      return null;
    };

    const handleLine = async (host: string, line: string): Promise<void> => {
      const ev = parsePaneEventLine(line);
      if (!ev) return;
      // A fresh session in a pane is the moment a recycled %N is most likely
      // to point at a different window than the cache thinks: forget the pane
      // so the next resolvable event re-fetches instead of misrouting a dot.
      if (ev.event === "session-start") {
        paneMaps.current.get(host)?.map.delete(ev.pane);
      }
      const key = await resolveKey(host, ev.pane);
      if (!key) return;
      const turnKey = `${host}\0${ev.pane}`;
      const step = paneEventStep(ev.event, midTurn.current.get(turnKey) ?? false);
      midTurn.current.set(turnKey, step.midTurn);
      if (!step.tab) return;
      const tabId = tabIdForKey(key);
      if (tabId === null) return;
      useTabStatusStore.getState().escalate(tabId, step.tab);
    };

    const startWatcher = (host: string): void => {
      const w: Watcher = { id: null, gone: false, timer: null };
      watchers.current.set(host, w);
      const connect = async (): Promise<void> => {
        if (w.gone) return;
        const chan = new Channel<TailEvent>();
        chan.onmessage = (ev) => {
          if (w.gone) return;
          if (ev.kind === "line") {
            void handleLine(host, ev.line);
          } else {
            w.id = null;
            w.timer = window.setTimeout(() => {
              w.timer = null;
              void connect();
            }, RECONNECT_MS);
          }
        };
        try {
          w.id = await invoke<number>("ssh_pane_events_start", {
            host,
            onEvent: chan,
          });
          if (w.gone && w.id !== null) {
            void invoke("ssh_pane_events_stop", { id: w.id }).catch(() => {});
          }
        } catch {
          if (w.gone) return;
          w.timer = window.setTimeout(() => {
            w.timer = null;
            void connect();
          }, DEAD_HOST_RETRY_MS);
        }
      };
      void connect();
    };

    const stopWatcher = (host: string, w: Watcher): void => {
      w.gone = true;
      if (w.timer !== null) window.clearTimeout(w.timer);
      if (w.id !== null) {
        void invoke("ssh_pane_events_stop", { id: w.id }).catch(() => {});
      }
      watchers.current.delete(host);
    };

    for (const host of desired) {
      if (!watchers.current.has(host)) startWatcher(host);
    }
    for (const [host, w] of [...watchers.current]) {
      if (!desired.has(host)) stopWatcher(host, w);
    }
  }, [desiredKey]);

  useEffect(() => {
    const all = watchers.current;
    return () => {
      for (const [host, w] of [...all]) {
        w.gone = true;
        if (w.timer !== null) window.clearTimeout(w.timer);
        if (w.id !== null) {
          void invoke("ssh_pane_events_stop", { id: w.id }).catch(() => {});
        }
        all.delete(host);
      }
    };
  }, []);
}
