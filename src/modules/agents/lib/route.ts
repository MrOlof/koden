import { usePreferencesStore } from "@/modules/settings/preferences";
import { showAgentToast } from "../components/AgentToast";
import { useAgentStore } from "../store/agentStore";
import { createCoalescer } from "./coalesce";
import { osNotify } from "./notify";
import type { AgentSource, NotificationKind } from "./types";

type RouteArgs = {
  source: AgentSource;
  agent: string;
  kind: NotificationKind;
  title: string;
  body?: string;
  focused: boolean;
  /** True when the user is currently looking at this agent. */
  visible: boolean;
  /** Allow an in-app toast when focused but not looking at the agent. */
  allowToast: boolean;
  tabId?: number;
  leafId?: number;
  onActivate: () => void;
};

/** Kinds that surface immediately whatever the notification mode. */
function isUrgent(kind: NotificationKind): boolean {
  return kind === "attention" || kind === "error";
}

const calmOsNotify = createCoalescer((title, body) => void osNotify(title, body));

export function routeAgentNotification({
  source,
  agent,
  kind,
  title,
  body,
  focused,
  visible,
  allowToast,
  tabId = 0,
  leafId = 0,
  onActivate,
}: RouteArgs): void {
  const prefs = usePreferencesStore.getState();
  if (!prefs.agentNotifications) return;
  if (focused && visible) return;

  useAgentStore.getState().pushNotification({ source, agent, kind, tabId, leafId });

  // The bell above records everything; from here the mode decides loudness.
  if (prefs.agentNotificationMode === "important" && !isUrgent(kind)) return;

  if (!focused) {
    if (prefs.agentNotificationMode === "smart" && !isUrgent(kind)) {
      // Calm events (finished/memory) batch: N agents done ≈ one OS toast.
      calmOsNotify.add({ kind, agent, title, body: body ?? agent });
    } else {
      void osNotify(title, body ?? agent);
    }
    return;
  }
  if (allowToast) {
    showAgentToast({ agent, title, body, onActivate });
  }
}
