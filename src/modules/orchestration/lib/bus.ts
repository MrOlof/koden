/**
 * Director command bus. The Director (a Claude Code terminal) appends one JSON
 * command per line to a shared file; Koden tails that file and materializes the
 * commands as visible agents, messages and status changes. This mirrors the
 * shared-board coordination pattern: the file is the durable record, the UI
 * reflects it.
 */
export type DirectorCommand =
  | {
      cmd: "spawn";
      name?: string;
      role?: string;
      model?: string;
      task: string;
    }
  | { cmd: "message"; from?: string; to?: string; text: string; kind?: string }
  | { cmd: "status"; agent: string; status: string }
  // Emitted by Claude Code subagent hooks (PreToolUse Task / SubagentStop) so
  // the orchestrator's native subagents surface as live nodes. `agentType` is
  // the Task tool's subagent_type, used to claim the matching roster slot.
  | { cmd: "subagent-start"; name?: string; agentType?: string }
  | { cmd: "subagent-stop"; name?: string }
  // Emitted by the PostToolUse hook after any tool the Director runs, so it
  // stays shown as "working" through tool-answer resumes.
  | { cmd: "director-active" };

function isCommand(value: unknown): value is DirectorCommand {
  if (!value || typeof value !== "object") return false;
  const cmd = (value as { cmd?: unknown }).cmd;
  return (
    cmd === "spawn" ||
    cmd === "message" ||
    cmd === "status" ||
    cmd === "subagent-start" ||
    cmd === "subagent-stop" ||
    cmd === "director-active"
  );
}

// Accepts either an explicit command, or a raw Claude Code PreToolUse(Task)
// hook input (which carries the subagent's task) and turns it into a named
// subagent-start.
function toCommand(parsed: unknown): DirectorCommand | null {
  if (isCommand(parsed)) return parsed;
  if (parsed && typeof parsed === "object" && "tool_input" in parsed) {
    const ti = (parsed as { tool_input?: unknown }).tool_input;
    let name: string | undefined;
    let agentType: string | undefined;
    if (ti && typeof ti === "object") {
      const t = ti as { description?: unknown; subagent_type?: unknown };
      if (typeof t.subagent_type === "string" && t.subagent_type.trim())
        agentType = t.subagent_type.trim();
      const raw =
        typeof t.description === "string"
          ? t.description
          : typeof t.subagent_type === "string"
            ? t.subagent_type
            : "";
      const trimmed = raw.trim().slice(0, 48);
      if (trimmed) name = trimmed;
    }
    return { cmd: "subagent-start", name, agentType };
  }
  return null;
}

/**
 * Reads the commands that appeared after `processedLines` complete lines.
 * A trailing line with no newline is treated as still being written and is not
 * processed until the next poll. Malformed lines are skipped, never thrown.
 */
export function readNewCommands(
  text: string,
  processedLines: number,
): { commands: DirectorCommand[]; processedLines: number } {
  const parts = text.split("\n");
  // Last element is either "" (text ended with \n) or a partial line; in both
  // cases the number of complete lines is parts.length - 1.
  const complete = Math.max(0, parts.length - 1);
  const commands: DirectorCommand[] = [];
  for (let i = Math.max(0, processedLines); i < complete; i++) {
    const line = parts[i].trim();
    if (!line) continue;
    try {
      const parsed: unknown = JSON.parse(line);
      const cmd = toCommand(parsed);
      if (cmd) commands.push(cmd);
    } catch {
      // skip malformed line
    }
  }
  return { commands, processedLines: complete };
}
