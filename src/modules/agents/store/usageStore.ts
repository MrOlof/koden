import { create } from "zustand";
import type { UsageSignal } from "../lib/types";

type UsageState = {
  /** Latest snapshot from the Rust usage guard, or null before the first. */
  latest: UsageSignal | null;
  /** Resets when % drops back below warn or a fresh window starts, so the warn
   *  notification fires at most once per window. */
  warnedOnce: boolean;
  /** Soft gate: orchestrator refuses to START new subagents while set. Set at
   *  >= pause, cleared only when % < pause (hysteresis avoids flapping near the
   *  threshold). */
  pauseActive: boolean;
  /** Latch so the "telemetry unavailable" notice fires once, not every signal. */
  telemetryLostNotified: boolean;
  /** Window boundary the latches are scoped to (resetEpochMs of the snapshot
   *  that set them). A newer resetEpochMs means a fresh window → re-arm. */
  windowKey: number | null;

  /** Fold a new snapshot in: clears warn latch when % drops below warn or the
   *  window rolls over, and applies pause hysteresis. Returns the post-update
   *  state so the bridge can decide what to notify. */
  ingest: (sig: UsageSignal, warnPct: number, pausePct: number) => void;
  markWarned: () => void;
  markTelemetryLostNotified: () => void;
  setPauseActive: (active: boolean) => void;

  /** Drop all guard state (the armed claude leaf exited / no agents left). */
  reset: () => void;
};

export const useUsageStore = create<UsageState>((set) => ({
  latest: null,
  warnedOnce: false,
  pauseActive: false,
  telemetryLostNotified: false,
  windowKey: null,

  ingest: (sig, warnPct, pausePct) =>
    set((s) => {
      const pct = sig.percentUsed;
      // A newer reset time means we rolled into a fresh usage window: re-arm
      // every latch so warn/pause can fire again for the new window.
      const freshWindow =
        sig.resetEpochMs !== null &&
        s.windowKey !== null &&
        sig.resetEpochMs > s.windowKey;

      let warnedOnce = freshWindow ? false : s.warnedOnce;
      let pauseActive = freshWindow ? false : s.pauseActive;
      const telemetryLostNotified = freshWindow
        ? false
        : s.telemetryLostNotified;

      if (pct !== null) {
        // Drop the warn latch once we're back under the warn line.
        if (pct < warnPct) warnedOnce = false;
        // Pause hysteresis: arm at/above pause, release only below pause.
        if (pct >= pausePct) pauseActive = true;
        else if (pct < pausePct) pauseActive = false;
      }

      const windowKey =
        sig.resetEpochMs !== null ? sig.resetEpochMs : s.windowKey;

      return {
        latest: sig,
        warnedOnce,
        pauseActive,
        telemetryLostNotified,
        windowKey,
      };
    }),

  markWarned: () =>
    set((s) => (s.warnedOnce ? s : { warnedOnce: true })),

  markTelemetryLostNotified: () =>
    set((s) =>
      s.telemetryLostNotified ? s : { telemetryLostNotified: true },
    ),

  setPauseActive: (active) =>
    set((s) => (s.pauseActive === active ? s : { pauseActive: active })),

  reset: () =>
    set({
      latest: null,
      warnedOnce: false,
      pauseActive: false,
      telemetryLostNotified: false,
      windowKey: null,
    }),
}));
