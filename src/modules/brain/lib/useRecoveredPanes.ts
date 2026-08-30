import { useCallback, useEffect, useMemo } from "react";
import { create } from "zustand";
import {
  submitToLeaf,
  whenSessionReady,
} from "@/modules/terminal/lib/useTerminalSession";
import {
  brainDismissRecovered,
  brainRecoveredPanes,
  brainResumePlan,
  type RecoveredPane,
} from "./bindings";
import {
  basenameOf,
  buildResumeCards,
  frontendCwd,
  recoveredLauncherSections,
  resumeBaseLaunch,
  resumeCommandFor,
} from "./resumeCards";

// The worker folds the journals early in its start, but nothing orders that
// before the webview's boot; an empty first read is re-checked a couple of
// times (an in-memory read, so the common "nothing to recover" case is cheap).
const LOAD_RETRY_MS = [1500, 4000];

type State = {
  panes: RecoveredPane[];
  /** Dismissed, resumed, or auto-resumed this boot: never shown again. */
  hidden: ReadonlySet<string>;
  loaded: boolean;
  load: () => Promise<void>;
  hide: (key: string) => void;
};

export const useRecoveredPanesStore = create<State>((set, get) => ({
  panes: [],
  hidden: new Set<string>(),
  loaded: false,
  load: async () => {
    if (get().loaded) return;
    set({ loaded: true });
    for (const delay of [0, ...LOAD_RETRY_MS]) {
      if (delay > 0) await new Promise((r) => setTimeout(r, delay));
      try {
        const panes = await brainRecoveredPanes();
        if (panes.length > 0) {
          set({ panes });
          return;
        }
      } catch {
        // Fail-open: no recovery data just means no cards.
        return;
      }
    }
  },
  hide: (key) => {
    if (get().hidden.has(key)) return;
    const hidden = new Set(get().hidden);
    hidden.add(key);
    set({ hidden });
  },
}));

/** Boot auto-resume handled this pane: keep it off the card strip. */
export function markRecoveredPaneConsumed(key: string): void {
  useRecoveredPanesStore.getState().hide(key);
}

/** Opens a terminal tab in `cwd` (current Space) and returns its shell leaf.
 * `useTabs().newAgentTab` has exactly this shape. */
export type OpenTerminalForResume = (
  cwd: string,
  title: string,
) => { leafId: number } | null | undefined;

type Params = {
  enabled: boolean;
  home: string | null;
  openTerminal: OpenTerminalForResume;
};

export function useRecoveredPanes({ enabled, home, openTerminal }: Params) {
  const panes = useRecoveredPanesStore((s) => s.panes);
  const hidden = useRecoveredPanesStore((s) => s.hidden);
  const load = useRecoveredPanesStore((s) => s.load);
  const hide = useRecoveredPanesStore((s) => s.hide);

  useEffect(() => {
    if (enabled) void load();
  }, [enabled, load]);

  const cards = useMemo(
    () => buildResumeCards(panes, { now: Date.now(), home, hidden }),
    [panes, hidden, home],
  );

  const dismiss = useCallback(
    (key: string) => {
      hide(key);
      void brainDismissRecovered(key).catch(() => {});
    },
    [hide],
  );

  const dismissAll = useCallback(() => {
    for (const c of cards) dismiss(c.key);
  }, [cards, dismiss]);

  const resume = useCallback(
    async (key: string) => {
      const pane = useRecoveredPanesStore
        .getState()
        .panes.find((p) => p.key === key);
      if (!pane) return;
      hide(key);
      const base = resumeBaseLaunch();
      // Plan before dismiss: dismissing drops the pane from the Rust list the
      // plan reads. The relaunched session journals under the same key, so its
      // own lifecycle supersedes the dismiss marker from here on.
      const plan = await brainResumePlan(key, base).catch(() => null);
      void brainDismissRecovered(key).catch(() => {});
      const cwd = frontendCwd(pane.cwd);
      const opened = openTerminal(cwd, basenameOf(cwd));
      const command = resumeCommandFor(plan, pane, base);
      if (!opened || !command) return;
      await whenSessionReady(opened.leafId);
      submitToLeaf(opened.leafId, command);
    },
    [hide, openTerminal],
  );

  const resumeSync = useCallback((key: string) => void resume(key), [resume]);

  const sections = useMemo(
    () => recoveredLauncherSections(cards, resumeSync),
    [cards, resumeSync],
  );

  return { cards, resume: resumeSync, dismiss, dismissAll, sections };
}
