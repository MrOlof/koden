import { create } from "zustand";

// Hard cap on auto-resubmits per leaf so a wedged session can't loop forever.
export const DEFAULT_MAX_RETRIES = 3;

type Timer = ReturnType<typeof setTimeout>;

type RetryState = {
  /** Per-leaf auto-retry toggle. Seeded from the global pref when a claude
   *  session arms; flipped per-tab from the AgentDock. */
  enabledByLeaf: Record<number, boolean>;
  /** Resubmits already fired on each leaf (against maxRetries). */
  retriesByLeaf: Record<number, number>;
  /** In-flight resubmit timer per leaf. setTimeout-based, so a pending retry
   *  is lost on app restart (acceptable v1). */
  timerByLeaf: Record<number, Timer>;
  maxRetries: number;

  /** Set the per-leaf enabled flag (no-op if unchanged). */
  setEnabled: (leafId: number, enabled: boolean) => void;
  /** Seed a leaf's enabled flag from the global default, only if not already
   *  set (so a user's explicit per-tab choice survives a re-arm). */
  seedEnabled: (leafId: number, globalDefault: boolean) => void;
  isEnabled: (leafId: number) => boolean;

  retriesOf: (leafId: number) => number;
  canRetry: (leafId: number) => boolean;
  bumpRetries: (leafId: number) => void;
  resetRetries: (leafId: number) => void;

  setTimer: (leafId: number, timer: Timer) => void;
  clearTimer: (leafId: number) => void;
  hasTimer: (leafId: number) => boolean;

  /** Drop all per-leaf state + cancel any pending timer (leaf exited/disposed). */
  clearLeaf: (leafId: number) => void;
};

export const useRetryStore = create<RetryState>((set, get) => ({
  enabledByLeaf: {},
  retriesByLeaf: {},
  timerByLeaf: {},
  maxRetries: DEFAULT_MAX_RETRIES,

  setEnabled: (leafId, enabled) =>
    set((s) => {
      if (s.enabledByLeaf[leafId] === enabled) return s;
      return { enabledByLeaf: { ...s.enabledByLeaf, [leafId]: enabled } };
    }),

  seedEnabled: (leafId, globalDefault) =>
    set((s) => {
      if (leafId in s.enabledByLeaf) return s;
      return { enabledByLeaf: { ...s.enabledByLeaf, [leafId]: globalDefault } };
    }),

  isEnabled: (leafId) => get().enabledByLeaf[leafId] ?? false,

  retriesOf: (leafId) => get().retriesByLeaf[leafId] ?? 0,

  canRetry: (leafId) => (get().retriesByLeaf[leafId] ?? 0) < get().maxRetries,

  bumpRetries: (leafId) =>
    set((s) => ({
      retriesByLeaf: {
        ...s.retriesByLeaf,
        [leafId]: (s.retriesByLeaf[leafId] ?? 0) + 1,
      },
    })),

  resetRetries: (leafId) =>
    set((s) => {
      if (!(leafId in s.retriesByLeaf)) return s;
      const next = { ...s.retriesByLeaf };
      delete next[leafId];
      return { retriesByLeaf: next };
    }),

  setTimer: (leafId, timer) =>
    set((s) => {
      const prev = s.timerByLeaf[leafId];
      if (prev) clearTimeout(prev);
      return { timerByLeaf: { ...s.timerByLeaf, [leafId]: timer } };
    }),

  clearTimer: (leafId) =>
    set((s) => {
      const prev = s.timerByLeaf[leafId];
      if (!prev) return s;
      clearTimeout(prev);
      const next = { ...s.timerByLeaf };
      delete next[leafId];
      return { timerByLeaf: next };
    }),

  hasTimer: (leafId) => leafId in get().timerByLeaf,

  clearLeaf: (leafId) =>
    set((s) => {
      const prev = s.timerByLeaf[leafId];
      if (prev) clearTimeout(prev);
      const enabledByLeaf = { ...s.enabledByLeaf };
      const retriesByLeaf = { ...s.retriesByLeaf };
      const timerByLeaf = { ...s.timerByLeaf };
      delete enabledByLeaf[leafId];
      delete retriesByLeaf[leafId];
      delete timerByLeaf[leafId];
      return { enabledByLeaf, retriesByLeaf, timerByLeaf };
    }),
}));
