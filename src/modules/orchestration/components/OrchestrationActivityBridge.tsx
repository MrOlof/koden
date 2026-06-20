import { leafIdForPty } from "@/modules/terminal";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import type { AgentStatus } from "../lib/types";
import { useOrchestrationStore } from "../store/orchestrationStore";

type TerminalAgentSignal = { id: number; kind: string };

// Maps a terminal coding-agent's lifecycle onto the orchestration agent backed
// by that terminal. A finished turn returns the agent to idle (calm) rather than
// pulsing forever; the agent exiting marks it done. Native subagents (the
// Director's Task tool) are surfaced separately via the command bus.
const STATUS_BY_SIGNAL: Record<string, AgentStatus> = {
  started: "working",
  working: "working",
  attention: "waiting",
  finished: "idle",
  exited: "done",
};

export function OrchestrationActivityBridge() {
  useEffect(() => {
    const unlisten = listen<TerminalAgentSignal>("koden:agent-signal", (e) => {
      const leafId = leafIdForPty(e.payload.id);
      if (leafId === null) return;
      const { agents, setStatus } = useOrchestrationStore.getState();
      const agent = Object.values(agents).find((a) => a.leafId === leafId);
      if (!agent) return;
      const status = STATUS_BY_SIGNAL[e.payload.kind];
      if (status) setStatus(agent.id, status);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);
  return null;
}
