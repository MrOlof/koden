/**
 * Pure per-tick line processing for the always-on agent bus tail
 * (AgentBusBridge). Extracted so the whole read cycle is unit-testable:
 * prime-to-end, truncation reset, partial-trailing-line deferral, per-line
 * event parsing (user-turn / agent-status / subagent-stop) and the tolerant
 * subagent-start recovery all live here; the bridge only does IO + effects.
 */

import { extractSubagentStarts, type SubagentStart } from "./subagentBus";

export type AgentBusState = {
  /** Complete lines already consumed. */
  processed: number;
  /** First read after (re)mount adopts the file end instead of replaying. */
  primed: boolean;
};

export type AgentBusEvents = {
  /** UserPromptSubmit bus hook: one entry per captured user turn. */
  turns: { pty: number; prompt: string }[];
  statuses: { pty: number; state: string }[];
  stops: { parent: number }[];
  starts: SubagentStart[];
};

type BusLine = {
  cmd?: string;
  // pty id; the user-turn hook emits it as a quoted string, others may too
  // (KODEN_SESSION is interpolated as a shell string).
  id?: number | string;
  state?: string;
  parent?: number | string;
  // user-turn: the raw UserPromptSubmit hook payload (carries `prompt`).
  data?: { prompt?: string };
};

function asPty(v: number | string | undefined): number | null {
  if (v == null) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

/**
 * Consume the bus file content from the last processed offset.
 *
 * @param seenToolUse Persistent tool_use_id dedup set, owned by the caller.
 *   Cleared here when the file shrank (truncation/rotation), mutated by the
 *   subagent-start recovery.
 */
export function readAgentBus(
  content: string,
  state: AgentBusState,
  seenToolUse: Set<string>,
): { events: AgentBusEvents; state: AgentBusState } {
  const events: AgentBusEvents = {
    turns: [],
    statuses: [],
    stops: [],
    starts: [],
  };
  const lines = content.split("\n");
  // The trailing element is a partial line until its newline arrives.
  const complete = Math.max(0, lines.length - 1);
  // First successful read after (re)mount: skip the pre-existing backlog (a
  // previous run's events) and only process what's appended from now on.
  if (!state.primed) {
    return { events, state: { processed: complete, primed: true } };
  }
  let processed = state.processed;
  if (complete < processed) {
    processed = 0; // file cleared/rotated: re-read from the top
    seenToolUse.clear();
  }
  const start = processed;
  for (let i = start; i < complete; i++) {
    const line = lines[i].trim();
    if (!line) continue;
    let evt: BusLine;
    try {
      evt = JSON.parse(line);
    } catch {
      continue;
    }
    if (evt.cmd === "user-turn") {
      const pty = asPty(evt.id);
      const prompt =
        typeof evt.data?.prompt === "string" ? evt.data.prompt.trim() : "";
      if (pty !== null && prompt) events.turns.push({ pty, prompt });
    } else if (evt.cmd === "agent-status") {
      const pty = asPty(evt.id);
      if (pty !== null && evt.state)
        events.statuses.push({ pty, state: evt.state });
    } else if (evt.cmd === "subagent-stop") {
      const parent = asPty(evt.parent);
      if (parent !== null) events.stops.push({ parent });
    }
    // subagent-start is NOT handled per-line: the hook that writes it is
    // non-atomic, so parallel Tasks interleave into corrupt multi-line JSON.
    // It is recovered below by a tolerant tool_use_id-keyed scan instead.
  }
  if (complete > start) {
    const newContent = lines.slice(start, complete).join("\n");
    events.starts = extractSubagentStarts(newContent, seenToolUse);
  }
  return { events, state: { processed: complete, primed: true } };
}
