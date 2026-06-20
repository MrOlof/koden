import { usePreferencesStore } from "@/modules/settings/preferences";
import { leafIdForPty, writeToSession } from "@/modules/terminal";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { routeAgentNotification } from "../lib/route";
import type { AgentSignal, UsageSignal } from "../lib/types";
import { useWindowFocus } from "../lib/useWindowFocus";
import { useAgentStore } from "../store/agentStore";
import { useUsageStore } from "../store/usageStore";

const USAGE_AGENT = "claude";

type Thresholds = { warnPct: number; pausePct: number };

function humanTime(epochMs: number | null): string | undefined {
  if (epochMs === null || !Number.isFinite(epochMs)) return undefined;
  try {
    return new Date(epochMs).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return undefined;
  }
}

function notify(title: string, body: string | undefined, focused: boolean): void {
  routeAgentNotification({
    source: "terminal",
    agent: USAGE_AGENT,
    kind: "attention",
    title,
    body,
    focused,
    // Account-scoped, not tied to a tab the user is "looking at", so never
    // suppressed as already-visible.
    visible: false,
    allowToast: true,
    onActivate: () => {},
  });
}

// ponytail: hard-stop sends Ctrl-C (\x03) to every armed claude leaf. Opt-in
// only (usageGuardHardStop, default off). Kept minimal: a single per-leaf
// SIGINT, no escalation loop.
function hardStopArmedClaudeLeaves(): void {
  const sessions = useAgentStore.getState().sessions;
  for (const session of Object.values(sessions)) {
    if (session.agent === USAGE_AGENT) writeToSession(session.leafId, "\x03");
  }
}

function handleUsage(
  sig: UsageSignal,
  thresholds: Thresholds,
  hardStop: boolean,
  focused: boolean,
): void {
  const store = useUsageStore.getState();
  store.ingest(sig, thresholds.warnPct, thresholds.pausePct);
  const after = useUsageStore.getState();

  if (sig.telemetryLost) {
    if (!after.telemetryLostNotified) {
      store.markTelemetryLostNotified();
      notify(
        "Usage telemetry unavailable",
        "Guarding by time estimate.",
        focused,
      );
    }
    return;
  }

  if (sig.thresholdCrossed === "pause") {
    store.setPauseActive(true);
    const when = humanTime(sig.resetEpochMs);
    const estimate = sig.source === "time" ? " (estimate)" : "";
    notify(
      "Usage limit reached",
      `New subagents paused${estimate}.${when ? ` Resets ~${when}.` : ""}`,
      focused,
    );
    if (hardStop) hardStopArmedClaudeLeaves();
    return;
  }

  if (sig.thresholdCrossed === "warn" && !after.warnedOnce) {
    store.markWarned();
    const when = humanTime(sig.resetEpochMs);
    const estimate = sig.source === "time" ? " (estimate)" : "";
    notify(
      "Usage approaching limit",
      `${
        sig.percentUsed !== null ? `${Math.round(sig.percentUsed)}% used` : "Near limit"
      }${estimate}.${when ? ` Resets ~${when}.` : ""}`,
      focused,
    );
  }
}

function handleAgentSignal(sig: AgentSignal): void {
  if (sig.kind !== "exited") return;
  // When the last terminal agent exits there's nothing left to guard: clear
  // pauseActive and every latch so a future run starts clean. Count agents
  // OTHER than the one exiting — independent of whether AgentNotificationsBridge
  // has already removed it from the store (the two bridges race on the same
  // signal, so a plain length<=1 check could wipe state while a second agent
  // is still running).
  const exitingLeaf = leafIdForPty(sig.id);
  const sessions = useAgentStore.getState().sessions;
  const remaining = Object.keys(sessions).filter(
    (k) => Number(k) !== exitingLeaf,
  ).length;
  if (remaining === 0) useUsageStore.getState().reset();
}

/**
 * Listens for account-wide usage signals from the Rust usage guard and raises a
 * one-shot warn / pause notification (reusing the agent notification router).
 * Soft gate: on pause it sets useUsageStore.pauseActive so the orchestrator
 * refuses to start new subagents. An optional opt-in hard-stop (usageGuardHardStop,
 * default off) additionally sends Ctrl-C to armed claude leaves. No-op while
 * usageGuardEnabled is false.
 */
export function UsageBridge() {
  const enabled = usePreferencesStore((p) => p.usageGuardEnabled);
  const warnPct = usePreferencesStore((p) => p.usageGuardWarnPct);
  const pausePct = usePreferencesStore((p) => p.usageGuardPausePct);
  const hardStop = usePreferencesStore((p) => p.usageGuardHardStop);
  const focused = useWindowFocus();

  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  const thresholdsRef = useRef<Thresholds>({ warnPct, pausePct });
  thresholdsRef.current = { warnPct, pausePct };
  const hardStopRef = useRef(hardStop);
  hardStopRef.current = hardStop;
  const focusedRef = useRef(focused);
  focusedRef.current = focused;

  // Push the guard config to the Rust poller. Without this the poller stays at
  // its safe default (enabled=false) and never honours the user's prefs — so
  // the endpoint is only polled once the user actually turns the guard on, and
  // with their thresholds. Re-fires on every change (incl. after prefs hydrate).
  useEffect(() => {
    void invoke("usage_guard_set", { enabled, warnPct, pausePct }).catch(
      () => {},
    );
  }, [enabled, warnPct, pausePct]);

  useEffect(() => {
    let alive = true;
    const unlisteners: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      p.then((u) => {
        if (alive) unlisteners.push(u);
        else u();
      }).catch(() => {});
    };

    track(
      listen<UsageSignal>("koden:usage-signal", (e) => {
        if (!enabledRef.current) return;
        handleUsage(
          e.payload,
          thresholdsRef.current,
          hardStopRef.current,
          focusedRef.current,
        );
      }),
    );
    track(
      listen<AgentSignal>("koden:agent-signal", (e) => {
        if (!enabledRef.current) return;
        handleAgentSignal(e.payload);
      }),
    );

    return () => {
      alive = false;
      for (const u of unlisteners) u();
    };
  }, []);

  return null;
}
