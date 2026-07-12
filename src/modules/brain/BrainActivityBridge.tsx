import { useAgentStore } from "@/modules/agents/store/agentStore";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { toast } from "sonner";
import { useBrainActivityStore } from "./lib/activityStore";
import type { BrainActivityEvent } from "./lib/bindings";

const ACTIVITY_EVENT = "koden:brain-activity";

function summary(e: BrainActivityEvent): string {
  const n = e.count;
  switch (e.kind) {
    case "applied":
      return `${e.project_name}: ${n} memory update${n === 1 ? "" : "s"}`;
    case "reflected":
      return `${e.project_name}: reflected · ${n} proposal${n === 1 ? "" : "s"}${
        e.spent_usd != null ? ` · $${e.spent_usd.toFixed(4)}` : ""
      }`;
    case "reverted":
      return `${e.project_name}: memory change reverted`;
    case "registered":
      return `${e.project_name} registered — indexing`;
  }
}

/**
 * Always-mounted listener for the worker's coalesced `koden:brain-activity`
 * events (ADR-020 — one per apply-sweep batch / reflect round / revert). Feeds:
 *  - the ambient status-bar segment (always, via the activity store),
 *  - a terse sonner toast + a NotificationBell entry, both gated on the
 *    `memoryNotifications` preference (default ON).
 * Follows the AgentBusBridge / LocalAgentNotificationsBridge bridge pattern:
 * renders nothing, routes everything.
 */
export function BrainActivityBridge({
  onOpenBrainMemory,
}: {
  onOpenBrainMemory: () => void;
}) {
  // Ref'd so the long-lived listener sees the current pref + callback without
  // re-subscribing (the LocalAgentNotificationsBridge idiom).
  const enabled = usePreferencesStore((s) => s.memoryNotifications);
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  const openRef = useRef(onOpenBrainMemory);
  openRef.current = onOpenBrainMemory;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<BrainActivityEvent>(ACTIVITY_EVENT, ({ payload }) => {
      // Ambient chrome first — the status-bar segment is NOT pref-gated.
      useBrainActivityStore.getState().record(payload);
      if (!enabledRef.current) return;
      toast(payload.kind === "registered" ? "Koden Brain" : "Librarian", {
        description: summary(payload),
        action: { label: "View", onClick: () => openRef.current() },
        duration: 5000,
      });
      // Reviewable trail for missed toasts (the bell's existing store API).
      useAgentStore.getState().pushNotification({
        source: "brain",
        agent: payload.project_name,
        kind: "memory",
        tabId: 0,
        leafId: 0,
      });
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return null;
}
