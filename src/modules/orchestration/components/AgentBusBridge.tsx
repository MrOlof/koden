import { native } from "@/modules/ai/lib/native";
import { useTabStatusStore } from "@/modules/tabs";
import { addTurnForLeaf, leafIdForPty } from "@/modules/terminal";
import { useEffect, useRef } from "react";
import { extractSubagentStarts } from "../lib/subagentBus";
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

// Only the small, effectively-atomic lines are parsed per-line now
// (agent-status + subagent-stop). subagent-start is recovered separately by a
// tolerant scan, so its Task payload shape no longer lives here.
type BusEvent = {
  cmd?: string;
  // pty id; the user-turn hook emits it as a quoted string, others as a number.
  id?: number | string;
  state?: string;
  parent?: number;
  // user-turn: the raw Claude Code UserPromptSubmit hook payload (carries `prompt`).
  data?: { prompt?: string };
};

/**
 * Always-on reader for the per-pane agent status + subagent bus. Claude Code's
 * hooks append lines tagged with the pty id (KODEN_SESSION, injected per pane in
 * session.rs):
 *  - `{"cmd":"agent-status","id":<pty>,"state":...}` — working/attention/finished
 *  - `{"cmd":"subagent-start","parent":<pty>,"task":<Task payload>}` — a Task subagent
 *  - `{"cmd":"subagent-stop","parent":<pty>}` — a subagent finished
 * Each routes to the terminal node it belongs to, lighting the dock, the tab
 * pill, the taskbar, and (for subagents) the topology graph. Replaces the
 * OSC-777 status path and the Director-only subagent bus, so EVERY terminal's
 * status and subagents surface, not just the Director's.
 */
export function AgentBusBridge({ busPath }: { busPath: string | null }) {
  const processed = useRef(0);
  // Dedup key for recovered subagents: every real Task carries a unique
  // tool_use_id. Spawning is keyed on this (not line framing), so re-reads, the
  // non-atomic hook's doubled wrapper, and duplicated fragments never
  // double-spawn. Reset alongside `processed` when the bus path changes/clears.
  const seenToolUse = useRef<Set<string>>(new Set());
  // The bus is append-only and never cleared, so on (re)mount we adopt its
  // current end as the baseline instead of replaying it — otherwise turns and
  // subagent events from PREVIOUS app runs flood into this run's panes (e.g. an
  // old claude session's prompts showing up in a new codex pane on the same pty).
  const primed = useRef(false);

  useEffect(() => {
    if (!busPath) return;
    processed.current = 0;
    primed.current = false;
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
      const lines = res.content.split("\n");
      // The trailing element is a partial line until its newline arrives.
      const complete = Math.max(0, lines.length - 1);
      // First successful read after (re)mount: skip the pre-existing backlog (a
      // previous run's events) and only process what's appended from now on.
      if (!primed.current) {
        primed.current = true;
        processed.current = complete;
        return;
      }
      if (complete < processed.current) {
        processed.current = 0; // file cleared/rotated — re-read from the top
        seenToolUse.current = new Set();
      }
      const start = processed.current;
      for (let i = start; i < complete; i++) {
        const line = lines[i].trim();
        if (!line) continue;
        let evt: BusEvent;
        try {
          evt = JSON.parse(line);
        } catch {
          continue;
        }
        // A Claude user turn: the UserPromptSubmit hook captured the prompt text.
        // Route it to the pane's CommandMarks so the Inputs list shows every turn
        // (the reliable path; scanTurns scraping is only the no-signal fallback).
        if (evt.cmd === "user-turn") {
          if (evt.id == null) continue;
          const prompt =
            typeof evt.data?.prompt === "string" ? evt.data.prompt.trim() : "";
          if (!prompt) continue;
          const leafId = leafIdForPty(Number(evt.id));
          if (leafId !== null) addTurnForLeaf(leafId, prompt);
          continue;
        }

        const orch = useOrchestrationStore.getState();

        if (evt.cmd === "agent-status") {
          if (evt.id == null || !evt.state) continue;
          const leafId = leafIdForPty(Number(evt.id));
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
          if (evt.state === "attention") {
            if (agent.status === "working" || agent.status === "spawning") {
              orch.setStatus(agent.id, "waiting");
              if (agent.tabId !== null) {
                useTabStatusStore.getState().escalate(agent.tabId, "waiting");
              }
            }
            continue;
          }
          const next = AGENT_STATUS[evt.state];
          if (next) orch.setStatus(agent.id, next);
          if (agent.tabId !== null) {
            const tab = TAB_STATUS[evt.state];
            if (tab) useTabStatusStore.getState().escalate(agent.tabId, tab);
          }
          continue;
        }

        // subagent-start is NOT handled per-line: the Claude Code hook that
        // writes it is non-atomic, so two PARALLEL Tasks interleave their
        // appends into corrupt, multi-line, doubled-and-concatenated JSON that
        // JSON.parse would silently drop. It is instead recovered below by a
        // tolerant, tool_use_id-keyed scan over the whole new content.

        if (evt.cmd === "subagent-stop") {
          if (evt.parent == null) continue;
          const parentLeaf = leafIdForPty(Number(evt.parent));
          if (parentLeaf === null) continue;
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
      }

      // Recover subagent-start events tolerantly over the NEW content as one
      // string (newly-completed lines joined with "\n"), so a payload split
      // across a file-line boundary survives and interleaved/doubled fragments
      // from the non-atomic hook never double-spawn (dedup is by tool_use_id).
      if (complete > start) {
        const newContent = lines.slice(start, complete).join("\n");
        const starts = extractSubagentStarts(newContent, seenToolUse.current);
        if (starts.length > 0) {
          const orch = useOrchestrationStore.getState();
          for (const s of starts) {
            const parentLeaf = leafIdForPty(s.parent);
            if (parentLeaf === null) continue;
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
        }
      }

      processed.current = complete;
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
