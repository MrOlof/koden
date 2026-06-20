import { create } from "zustand";

export type TabStatus = "working" | "waiting" | "done" | "error";

// Urgency rank for the per-tab roll-up. A tab can host several terminals; its
// pill shows the most actionable unseen status across them: a broken or
// input-needing terminal beats a freshly-finished one, which beats one still
// working. Cleared once you switch to the tab (you've seen it).
const TAB_STATUS_RANK: Record<TabStatus, number> = {
  working: 1,
  done: 2,
  waiting: 3,
  error: 4,
};

/** The more-urgent of two tab statuses (worst-wins roll-up). */
export function worseTabStatus(a: TabStatus, b: TabStatus): TabStatus {
  return TAB_STATUS_RANK[a] >= TAB_STATUS_RANK[b] ? a : b;
}

type TabStatusState = {
  /** Per-tab activity status, driven by terminal agent signals. */
  statuses: Record<number, TabStatus>;
  setStatus: (tabId: number, status: TabStatus) => void;
  /**
   * Raise a tab's pill to `status` only if it is more urgent than what's
   * already shown, so a later "working" signal from one terminal never masks an
   * unseen "waiting"/"done" from another terminal in the same tab.
   */
  escalate: (tabId: number, status: TabStatus) => void;
  clear: (tabId: number) => void;
};

export const useTabStatusStore = create<TabStatusState>((set) => ({
  statuses: {},
  setStatus: (tabId, status) =>
    set((s) =>
      s.statuses[tabId] === status
        ? s
        : { statuses: { ...s.statuses, [tabId]: status } },
    ),
  escalate: (tabId, status) =>
    set((s) => {
      const cur = s.statuses[tabId];
      const next = cur ? worseTabStatus(cur, status) : status;
      return next === cur
        ? s
        : { statuses: { ...s.statuses, [tabId]: next } };
    }),
  clear: (tabId) =>
    set((s) => {
      if (!(tabId in s.statuses)) return s;
      const next = { ...s.statuses };
      delete next[tabId];
      return { statuses: next };
    }),
}));
