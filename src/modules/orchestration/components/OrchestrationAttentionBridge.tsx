import {
  getCurrentWindow,
  UserAttentionType,
} from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { useOrchestrationStore } from "../store/orchestrationStore";

/**
 * App-level "something needs you" signal. When any agent is waiting for input
 * and the window is not focused, flash the taskbar / dock so you notice even
 * with Koden minimized; clear it the moment you focus the window or nothing is
 * waiting anymore. This is the cross-tab escalation of the per-tab amber pill:
 * the pill tells you which tab, this tells you to come back at all.
 */
export function OrchestrationAttentionBridge() {
  const anyWaiting = useOrchestrationStore((s) => {
    for (const a of Object.values(s.agents)) {
      if (a.status === "waiting") return true;
    }
    return false;
  });

  const [focused, setFocused] = useState(() =>
    typeof document !== "undefined" ? document.hasFocus() : true,
  );

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged(({ payload }) => setFocused(payload))
      .then((u) => {
        if (alive) unlisten = u;
        else u();
      })
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const win = getCurrentWindow();
    // requestUserAttention(null) cancels a pending flash. Both calls are
    // no-ops if the window capability isn't granted yet (older build), so they
    // fail silently rather than throwing into the render tree.
    const request =
      anyWaiting && !focused ? UserAttentionType.Critical : null;
    win.requestUserAttention(request).catch(() => {});
  }, [anyWaiting, focused]);

  return null;
}
