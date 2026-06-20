import { usePreferencesStore } from "@/modules/settings/preferences";
import { leafIdForPty, submitToLeaf, writeToSession } from "@/modules/terminal";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import type { AgentSignal, RetrySignal } from "../lib/types";
import { useRetryStore } from "../store/retryStore";

// Verbatim resubmit text (mirrors claude-auto-retry's `continue`, adapted to a
// full prompt so the agent has the context to pick up).
const RETRY_PROMPT =
  "Continue where you left off. The previous attempt was rate limited.";

// Don't trust a wildly stale or far-future reset time: clamp the wait so a
// misparse can't pin a leaf forever or fire instantly. 30 days upper bound.
const MAX_WAIT_MS = 30 * 24 * 60 * 60 * 1000;

function scheduleRetry(sig: RetrySignal): void {
  const leafId = leafIdForPty(sig.id);
  if (leafId === null) return;

  const store = useRetryStore.getState();
  if (!store.isEnabled(leafId)) return;
  if (!store.canRetry(leafId)) return;
  // One pending resubmit per leaf; a fresh signal supersedes the old timer.
  if (store.hasTimer(leafId)) return;

  const delay = Math.min(MAX_WAIT_MS, Math.max(0, sig.resetEpochMs - Date.now()));
  const timer = setTimeout(() => {
    const s = useRetryStore.getState();
    s.clearTimer(leafId);
    if (!s.isEnabled(leafId) || !s.canRetry(leafId)) return;
    if (leafIdForPty(sig.id) === null) return;
    s.bumpRetries(leafId);
    // Current Claude Code auto-opens an interactive /rate-limit-options menu
    // after the banner. Submitting now would land the resume text in that
    // menu's filter, not as a prompt. Esc dismisses the menu first.
    writeToSession(leafId, "\x1b");
    submitToLeaf(leafId, RETRY_PROMPT);
  }, delay);
  store.setTimer(leafId, timer);
}

function handleAgentSignal(sig: AgentSignal, autoRetryDefault: boolean): void {
  const leafId = leafIdForPty(sig.id);
  if (leafId === null) return;
  const store = useRetryStore.getState();

  switch (sig.kind) {
    case "started":
      if (sig.agent === "claude") store.seedEnabled(leafId, autoRetryDefault);
      return;
    case "working":
      // A real working transition means the prior limit cleared: cancel any
      // pending timer and let the next limit re-fire.
      store.clearTimer(leafId);
      return;
    case "exited":
      store.clearLeaf(leafId);
      return;
    default:
      return;
  }
}

/**
 * Listens for usage-limit signals from the Rust retry detector and, per leaf,
 * waits until the parsed reset time then resubmits a continue prompt. Each leaf
 * has independent enabled state, retry count, and pending timer, so three
 * rate-limited terminals retry on their own schedules. Scheduling is in-memory
 * (setTimeout): a pending retry is lost on app restart, which is acceptable v1.
 */
export function RetryBridge() {
  const autoRetryDefault = usePreferencesStore((p) => p.autoRetryEnabled);
  const defaultRef = useRef(autoRetryDefault);
  defaultRef.current = autoRetryDefault;

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
      listen<RetrySignal>("koden:retry-signal", (e) => scheduleRetry(e.payload)),
    );
    track(
      listen<AgentSignal>("koden:agent-signal", (e) =>
        handleAgentSignal(e.payload, defaultRef.current),
      ),
    );

    return () => {
      alive = false;
      for (const u of unlisteners) u();
    };
  }, []);

  return null;
}
