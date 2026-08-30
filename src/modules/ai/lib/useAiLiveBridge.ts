import { useManagedAgentsStore } from "@/modules/agents/store/managedAgentsStore";
import { useOrchestrationStore } from "@/modules/orchestration/store/orchestrationStore";
import { useSpaces } from "@/modules/spaces";
import { labelFor, type Tab } from "@/modules/tabs";
import {
  findLeafCwd,
  leafHasForegroundProcess,
  type TerminalPaneHandle,
  terminalLeaves,
  usePaneTitleStore,
  whenSessionReady,
  writeToSession,
} from "@/modules/terminal";
import { invoke } from "@tauri-apps/api/core";
import { type RefObject, useEffect, useRef } from "react";
import type { Live } from "../store/chatStore";
import { snapshotSpaces, type TerminalTargetInfo } from "../tools/context";
import { redactSensitive } from "./redact";

type TuiWaitResult = "ready" | "gone" | "timeout";

// Enter must land as its own chunk: Claude-style TUIs treat a same-chunk
// trailing CR as a literal newline (see tools/agent.ts SUBMIT_DELAY_MS);
// shells don't care about the split, so one primitive serves both.
const SEND_ENTER_DELAY_MS = 120;

async function waitForClaudeTuiReady(
  readBuf: () => string | null,
  timeoutMs = 8000,
): Promise<TuiWaitResult> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const buf = readBuf();
    if (buf === null) return "gone";
    if (buf.includes("shortcuts") || buf.includes("? for")) return "ready";
    await new Promise((r) => setTimeout(r, 120));
  }
  return "timeout";
}

type Params = {
  setLive: (live: Live) => void;
  activeId: number;
  tabs: Tab[];
  explorerRoot: string | null;
  launchCwd: string | null;
  home: string | null;
  openPreviewTab: (url: string) => void;
  newAgentTab: (
    cwd: string | undefined,
    title: string,
  ) => { tabId: number; leafId: number };
  terminalRefs: RefObject<Map<number, TerminalPaneHandle>>;
  // Layout lane (create/arrange only — no close callbacks by design, ADR-017).
  openWorkspaceTab: Live["openWorkspaceTab"];
  splitWorkspacePane: Live["splitWorkspacePane"];
  focusWorkspacePane: Live["focusWorkspacePane"];
  getWorkspaceLayout: Live["getWorkspaceLayout"];
  // Spaces lane: create rides App's handleNewSpace (it also opens the
  // space's first tab); list/switch act on the spaces store directly.
  createSpace: Live["createSpace"];
};

/**
 * Publishes the live workspace context (cwd, terminal buffer, active file,
 * managed-agent spawning, ...) into the chat store so AI tools can read and
 * act on the foreground state.
 *
 * The live object's getters read the latest state through a ref, so the bridge
 * is published once instead of re-running on every tab/cwd change — cwd updates
 * arrive from terminal OSC on shell output and would otherwise churn constantly.
 */
export function useAiLiveBridge(params: Params) {
  const { setLive, terminalRefs } = params;
  const ref = useRef(params);
  ref.current = params;

  useEffect(() => {
    const findCwd = () => {
      const { activeId, tabs, explorerRoot, launchCwd, home } = ref.current;
      const active = tabs.find((x) => x.id === activeId);
      if (active?.kind === "terminal") {
        return (
          findLeafCwd(active.paneTree, active.activeLeafId) ??
          active.cwd ??
          null
        );
      }
      for (let i = tabs.length - 1; i >= 0; i--) {
        const t = tabs[i];
        if (t.kind !== "terminal") continue;
        const cwd = findLeafCwd(t.paneTree, t.activeLeafId) ?? t.cwd;
        if (cwd) return cwd;
      }
      return explorerRoot ?? launchCwd ?? home ?? null;
    };

    setLive({
      getCwd: findCwd,
      getTerminalContext: () => {
        const { activeId, tabs } = ref.current;
        const t = tabs.find((x) => x.id === activeId);
        if (t?.kind !== "terminal") return null;
        if (t.private) return null;
        const buf = terminalRefs.current.get(t.activeLeafId)?.getBuffer(300);
        return buf ? redactSensitive(buf) : null;
      },
      isActiveTerminalPrivate: () => {
        const { activeId, tabs } = ref.current;
        const t = tabs.find((x) => x.id === activeId);
        return t?.kind === "terminal" && t.private === true;
      },
      injectIntoActivePty: (text) => {
        const { activeId, tabs } = ref.current;
        const t = tabs.find((x) => x.id === activeId);
        if (t?.kind !== "terminal") return false;
        const term = terminalRefs.current.get(t.activeLeafId);
        if (!term) return false;
        term.write(text);
        term.focus();
        return true;
      },
      getWorkspaceRoot: () => {
        const { explorerRoot, launchCwd, home } = ref.current;
        return explorerRoot ?? launchCwd ?? home ?? null;
      },
      getActiveFile: () => {
        const { activeId, tabs } = ref.current;
        const t = tabs.find((x) => x.id === activeId);
        return t?.kind === "editor" ? t.path : null;
      },
      openPreview: (url: string) => {
        ref.current.openPreviewTab(url);
        return true;
      },
      spawnManagedAgent: (prompt: string, sessionId: string) => {
        const trimmed = prompt.trim();
        if (!trimmed) return null;
        const oneLine = trimmed.replace(/\s*\r?\n\s*/g, " ");
        const cwd = findCwd();
        const short =
          oneLine.length > 32 ? `${oneLine.slice(0, 32)}…` : oneLine;
        const { tabId, leafId } = ref.current.newAgentTab(
          cwd ?? undefined,
          `claude · ${short}`,
        );
        useManagedAgentsStore
          .getState()
          .register({ leafId, tabId, sessionId, task: oneLine, cwd });
        const hooksReady = invoke("agent_enable_claude_hooks").catch(() => {});
        void (async () => {
          await Promise.all([whenSessionReady(leafId), hooksReady]);
          if (!writeToSession(leafId, "claude\r")) {
            useManagedAgentsStore.getState().remove(leafId);
            return;
          }
          const readBuf = () => {
            const term = terminalRefs.current.get(leafId);
            return term ? term.getBuffer(120) : null;
          };
          const result = await waitForClaudeTuiReady(readBuf);
          if (result !== "ready") {
            if (result === "timeout") {
              console.warn(
                "[koden] Claude TUI did not appear in time; aborting prompt send",
              );
            }
            useManagedAgentsStore.getState().remove(leafId);
            return;
          }
          if (!writeToSession(leafId, `\x1b[200~${trimmed}\x1b[201~`)) {
            useManagedAgentsStore.getState().remove(leafId);
            return;
          }
          setTimeout(() => writeToSession(leafId, "\r"), 120);
          useManagedAgentsStore.getState().setPhase(leafId, "working");
        })();
        return { tabId, leafId };
      },
      readLeafBuffer: (leafId: number) => {
        const buf = terminalRefs.current.get(leafId)?.getBuffer(300);
        return buf ? redactSensitive(buf) : null;
      },
      // Terminal targeting lane (ADR-017 addendum). Enumerates ALL spaces —
      // the layout snapshot deliberately filters to the active space, a
      // name resolver must not.
      listTerminalTargets: () => {
        const { activeId, tabs } = ref.current;
        const spaces = useSpaces.getState().spaces;
        const paneTitles = usePaneTitleStore.getState().titles;
        const managed = useManagedAgentsStore.getState().agents;
        const orchByLeaf = new Map<number, { name: string; status: string }>();
        for (const a of Object.values(
          useOrchestrationStore.getState().agents,
        )) {
          if (a.leafId !== null)
            orchByLeaf.set(a.leafId, { name: a.name, status: a.status });
        }
        const out: TerminalTargetInfo[] = [];
        for (const t of tabs) {
          if (t.kind !== "terminal") continue;
          const spaceName =
            spaces.find((s) => s.id === t.spaceId)?.name ?? t.spaceId;
          const tabTitle = labelFor(t);
          for (const leaf of terminalLeaves(t.paneTree)) {
            const paneTitle = paneTitles[leaf.id]?.label?.trim();
            const m = managed[leaf.id];
            out.push({
              paneId: leaf.id,
              tabId: t.id,
              space: spaceName,
              title: paneTitle || tabTitle,
              tabTitle,
              cwd: leaf.cwd ?? t.cwd ?? null,
              agent:
                orchByLeaf.get(leaf.id) ??
                (m ? { name: "claude", status: m.phase } : null),
              active: t.id === activeId && leaf.id === t.activeLeafId,
              tabActive: leaf.id === t.activeLeafId,
              private: t.private === true,
              cold: t.cold === true,
            });
          }
        }
        return out;
      },
      // Leaf-addressed write, deliberately WITHOUT focus/tab/space activation —
      // a background send must never hijack what the user is typing.
      sendToTerminal: (leafId: number, data: string, submit: boolean) => {
        if (!writeToSession(leafId, data)) return false;
        if (submit)
          setTimeout(() => writeToSession(leafId, "\r"), SEND_ENTER_DELAY_MS);
        return true;
      },
      terminalHasForegroundProcess: (leafId: number) =>
        leafHasForegroundProcess(leafId),
      // Latest-render closures via ref, like the getters above.
      openWorkspaceTab: (kind, opts) =>
        ref.current.openWorkspaceTab(kind, opts),
      splitWorkspacePane: (kind, side, title) =>
        ref.current.splitWorkspacePane(kind, side, title),
      focusWorkspacePane: (paneId) => ref.current.focusWorkspacePane(paneId),
      getWorkspaceLayout: () => ref.current.getWorkspaceLayout(),
      // Spaces lane (ADR-017 addendum). Switching is the exact store call
      // the header switcher rows and space-cycling shortcuts make, so the
      // App-level active-tab fallback behaves identically.
      listSpaces: () => {
        const { spaces, activeId } = useSpaces.getState();
        return snapshotSpaces(spaces, activeId, ref.current.tabs);
      },
      createSpace: (name, root) => ref.current.createSpace(name, root),
      switchSpace: (id) => {
        const { spaces, setActive } = useSpaces.getState();
        if (!spaces.some((s) => s.id === id)) return false;
        setActive(id);
        return true;
      },
    });
  }, [setLive, terminalRefs]);
}
