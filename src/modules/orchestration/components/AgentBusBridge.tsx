import { native } from "@/modules/ai/lib/native";
import { brainRecordTurn } from "@/modules/brain";
import { useTabStatusStore } from "@/modules/tabs";
import { addTurnForLeaf, leafIdForPty } from "@/modules/terminal";
import { useEffect, useRef } from "react";
import { type AgentBusState, readAgentBus } from "../lib/agentBusReader";
import { AGENT_ROLES, type AgentRole } from "../lib/types";
import { useOrchestrationStore } from "../store/orchestrationStore";

const POLL_MS = 400;

// Claude Code hook state -> agent status (drives the dock and, via "waiting",
// the taskbar attention bridge). A finished turn means the agent is waiting for
// YOU to type, i.e. "ready" (green), not idle (which is for a plain shell).
const AGENT_STATUS: Record<string, "working" | "waiting" | "ready"> = {
  working: "working",
  attention: "waiting",
  finished: "ready",
};
// Hook state -> tab pill. A finished turn is a "done" (green) notification
// (ready, come look); it survives until you switch to the tab (worst-wins).
const TAB_STATUS: Record<string, "working" | "waiting" | "done"> = {
  working: "working",
  attention: "waiting",
  finished: "done",
};

/**
 * Always-on reader for the shared hook bus (~/.koden/director-bus.jsonl, the
 * ONE file every Claude/Codex hook writes; see agent.rs bus_path_str). Lines,
 * tagged with the writer's pty id (KODEN_SESSION, injected per pane in
 * session.rs):
 *  - `{"cmd":"user-turn","id":"<pty>","data":<hook json>}` — the captured
 *    prompt of every submitted turn (Claude + Codex UserPromptSubmit); the
 *    payload's `session_id` is what makes a crash-resume card Tier-2
 *  - `{"parent":"<pty>","task":<PreToolUse(Task) hook json>}` — a Task
 *    subagent started (recovered tolerantly; the hook write is non-atomic)
 *  - `{"cmd":"subagent-stop","parent":"<pty>"}` — a subagent finished
 *  - `{"cmd":"agent-status","id":<pty>,"state":...}` — reserved; no current
 *    hook writes it (status flows over OSC 777 terminalSequence instead)
 *  - `{"cmd":"director-active",...}` — Director keep-alive; ignored here
 *    (DirectorBusBridge owns it)
 * Each routes to the terminal node it belongs to. While a Director is live its
 * OWN subagent events are left to DirectorBusBridge (roster-slot claiming);
 * this bridge handles every other terminal's.
 */
export function AgentBusBridge({ busPath }: { busPath: string | null }) {
  const state = useRef<AgentBusState>({ processed: 0, primed: false });
  // Dedup key for recovered subagents: every real Task carries a unique
  // tool_use_id. Spawning is keyed on this (not line framing), so re-reads, the
  // non-atomic hook's doubled wrapper, and duplicated fragments never
  // double-spawn. Reset alongside `state` when the bus path changes/clears.
  const seenToolUse = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!busPath) return;
    state.current = { processed: 0, primed: false };
    seenToolUse.current = new Set();
    let stopped = false;

    const tick = async () => {
      if (stopped) return;
      let res: Awaited<ReturnType<typeof native.readFile>>;
      try {
        res = await native.readFile(busPath);
      } catch {
        return; // bus may not exist yet, or read denied — retry next tick
      }
      if (res.kind !== "text") return;
      const { events, state: next } = readAgentBus(
        res.content,
        state.current,
        seenToolUse.current,
      );
      state.current = next;

      // User turns: route to the pane's turn store so the Inputs list shows
      // every turn (the reliable path; scanTurns scraping is the fallback) AND
      // to the Brain worker (ADR-020 session activity — filtered, truncated and
      // REDACTED at the Rust ingest seam before storage; the Claude session id
      // rides along for the Tier-2 resume journal). Fire-and-forget: a
      // not-yet-started worker just drops the turn.
      for (const t of events.turns) {
        const leafId = leafIdForPty(t.pty);
        if (leafId !== null) addTurnForLeaf(leafId, t.prompt);
        brainRecordTurn(t.pty, t.prompt, t.sessionId).catch(() => {});
      }

      const orch = useOrchestrationStore.getState();
      // The live Director's subagent events belong to DirectorBusBridge
      // (which claims planned roster slots); handling them here too would
      // double-materialize every Task node.
      const directorLeaf =
        Object.values(orch.agents).find(
          (a) => a.role === "director" && a.leafId !== null,
        )?.leafId ?? null;

      for (const s of events.statuses) {
        const leafId = leafIdForPty(s.pty);
        if (leafId === null) continue;
        const agent = Object.values(orch.agents).find(
          (a) => a.leafId === leafId,
        );
        if (!agent) continue;
        // A Notification fires both for a real permission (mid-turn) AND for an
        // idle "waiting for your input" after the turn finished. Only the
        // former is a decision you must make: treat attention as "needs you"
        // (orange) only while the agent is mid-work; an idle notification must
        // leave a finished agent green/ready, not flip it back to orange.
        if (s.state === "attention") {
          if (agent.status === "working" || agent.status === "spawning") {
            orch.setStatus(agent.id, "waiting");
            if (agent.tabId !== null) {
              useTabStatusStore.getState().escalate(agent.tabId, "waiting");
            }
          }
          continue;
        }
        const nextStatus = AGENT_STATUS[s.state];
        if (nextStatus) orch.setStatus(agent.id, nextStatus);
        if (agent.tabId !== null) {
          const tab = TAB_STATUS[s.state];
          if (tab) useTabStatusStore.getState().escalate(agent.tabId, tab);
        }
      }

      for (const stop of events.stops) {
        const parentLeaf = leafIdForPty(stop.parent);
        if (parentLeaf === null || parentLeaf === directorLeaf) continue;
        const parent = Object.values(orch.agents).find(
          (a) => a.leafId === parentLeaf,
        );
        if (!parent) continue;
        // SubagentStop carries no id, so retire the oldest still-running
        // native child of this parent (FIFO). The subagent is gone — remove it
        // from the roster + graph rather than leaving a stale "done" node.
        const child = Object.values(orch.agents)
          .filter((a) => a.parentId === parent.id && a.native)
          .sort((a, b) => a.createdAt - b.createdAt)[0];
        if (child) orch.remove(child.id);
      }

      for (const s of events.starts) {
        const parentLeaf = leafIdForPty(s.parent);
        if (parentLeaf === null || parentLeaf === directorLeaf) continue;
        const parent = Object.values(orch.agents).find(
          (a) => a.leafId === parentLeaf,
        );
        // No resolvable parent terminal, or a junk fragment with neither a
        // description nor a type — skip rather than spawn a phantom node.
        if (!parent) continue;
        if (!s.description && !s.subagentType) continue;
        const name = (s.description || s.subagentType || "Subagent").slice(
          0,
          80,
        );
        const role: AgentRole = (AGENT_ROLES as readonly string[]).includes(
          s.subagentType,
        )
          ? (s.subagentType as AgentRole)
          : "worker";
        const sid = orch.spawn({
          role,
          name,
          parentId: parent.id,
          native: true,
          task: s.description || null,
        });
        orch.setStatus(sid, "working");
      }
    };

    const id = window.setInterval(() => void tick(), POLL_MS);
    void tick();
    return () => {
      stopped = true;
      window.clearInterval(id);
    };
  }, [busPath]);

  return null;
}
