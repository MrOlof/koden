// Last-Librarian-activity store (ADR-020). Written by BrainActivityBridge on
// every `koden:brain-activity` event; read by the status bar's ambient brain
// segment (hover summary + flash) — a tiny always-on mirror, not a history
// (the NotificationBell keeps the reviewable list).
import { create } from "zustand";
import type { BrainActivityEvent } from "./bindings";

export type LastBrainActivity = {
  event: BrainActivityEvent;
  /** Local receipt time (ms) — drives the "3m ago" hover text + the flash. */
  at: number;
};

type State = {
  last: LastBrainActivity | null;
  record: (event: BrainActivityEvent) => void;
};

export const useBrainActivityStore = create<State>((set) => ({
  last: null,
  record: (event) => set({ last: { event, at: Date.now() } }),
}));
