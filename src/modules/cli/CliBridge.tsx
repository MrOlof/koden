import { routeAgentNotification } from "@/modules/agents/lib/route";
import { useAgentStore } from "@/modules/agents/store/agentStore";
import { useChatStore } from "@/modules/ai/store/chatStore";
import type { TerminalTargetInfo } from "@/modules/ai/tools/context";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { leafIdForPty, readLeafTail } from "@/modules/terminal";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { type CliContext, dispatch, type NotifyVia } from "./lib/dispatch";
import {
  CLI_REQUEST_EVENT,
  type CliRequest,
  type CliResult,
  isCliRequest,
} from "./lib/protocol";

// Thin adapter: Rust parks a socket connection and emits the request here;
// this answers with `cli_reply`. All behavior lives in lib/dispatch.ts; this
// file only assembles the context from live stores.

type Activate = (tabId: number, leafId: number) => void;

function buildContext(onActivate: Activate): CliContext {
  const live = useChatStore.getState().live;
  const prefs = usePreferencesStore.getState();
  return {
    prefs: {
      cliEnabled: prefs.cliEnabled,
      cliTerminalRead: prefs.cliTerminalRead,
      cliTerminalInput: prefs.cliTerminalInput,
      cliPanelControl: prefs.cliPanelControl,
      cliNotify: prefs.cliNotify,
    },
    listTerminalTargets: () => live.listTerminalTargets(),
    currentPaneId: (session) => {
      const id = Number(session);
      return Number.isInteger(id) && id > 0 ? leafIdForPty(id) : null;
    },
    agentState: (paneId) => {
      const s = useAgentStore.getState().sessions[paneId];
      return s ? { name: s.agent, status: s.status } : null;
    },
    readBuffer: (paneId, lines, raw) => readLeafTail(paneId, lines, raw),
    hasForeground: (paneId) => live.terminalHasForegroundProcess(paneId),
    send: (paneId, data, submit) => live.sendToTerminal(paneId, data, submit),
    openTab: (kind, opts) => live.openWorkspaceTab(kind, opts),
    splitPane: (kind, side, title) =>
      live.splitWorkspacePane(kind, side, title),
    listSpaces: () => live.listSpaces(),
    createSpace: (name, root) => live.createSpace(name, root),
    fallbackCwd: () => live.getCwd(),
    isDir: async (path) => {
      try {
        const st = await invoke<{ kind: string }>("fs_stat", { path });
        return st.kind === "dir";
      } catch {
        return false;
      }
    },
    notify: ({ message, pane }) => notifyUser(message, pane, onActivate),
  };
}

/** An explicit notify always surfaces (toast when focused, OS when not), so
 * `visible` is forced false; the router's focused-and-visible suppression is
 * for ambient agent state, not for a message the agent chose to send. */
function notifyUser(
  message: string,
  pane: TerminalTargetInfo | null,
  onActivate: Activate,
): NotifyVia {
  if (!usePreferencesStore.getState().agentNotifications) return "muted";
  const focused = typeof document !== "undefined" && document.hasFocus();
  routeAgentNotification({
    source: "terminal",
    agent: pane?.agent?.name ?? pane?.title ?? "koden",
    kind: "attention",
    title: message,
    body: pane ? `${pane.title} (koden notify)` : "koden notify",
    focused,
    visible: false,
    allowToast: true,
    tabId: pane?.tabId,
    leafId: pane?.paneId,
    onActivate: () => {
      if (pane) onActivate(pane.tabId, pane.paneId);
    },
  });
  return focused ? "toast" : "os";
}

async function answer(req: CliRequest, onActivate: Activate): Promise<void> {
  let res: CliResult;
  try {
    res = await dispatch(
      req.cmd,
      req.args,
      req.session ?? null,
      buildContext(onActivate),
    );
  } catch (e) {
    res = { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
  try {
    await invoke("cli_reply", {
      id: req.id,
      ok: res.ok,
      result: res.ok ? res.result : null,
      error: res.ok ? null : res.error,
    });
  } catch (e) {
    console.warn("[koden] cli_reply failed:", e);
  }
}

export function CliBridge({ onActivate }: { onActivate: Activate }) {
  const activateRef = useRef(onActivate);
  activateRef.current = onActivate;

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    listen<unknown>(CLI_REQUEST_EVENT, (e) => {
      if (!isCliRequest(e.payload)) return;
      void answer(e.payload, (tabId, leafId) =>
        activateRef.current(tabId, leafId),
      );
    })
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

  return null;
}
