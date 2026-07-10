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
  // `parent` is the emitting session's pty id (KODEN_SESSION): the bus file is
  // shared by EVERY Koden pane's hooks, so dispatch must be scoped to the
  // Director's own session (see acceptDirectorCommand).
  | {
      cmd: "subagent-start";
      name?: string;
      agentType?: string;
      parent?: number;
    }
  | { cmd: "subagent-stop"; name?: string; parent?: number }
  // Emitted by the PostToolUse hook after any tool the Director runs, so it
  // stays shown as "working" through tool-answer resumes.
  | { cmd: "director-active"; parent?: number };

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

// The hooks interpolate $KODEN_SESSION as a quoted shell string, so `parent`
// arrives as "5"; normalize to a number (or drop it if unparsable).
function parseParent(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && /^\d+$/.test(value)) return Number(value);
  return undefined;
}

// Pull a subagent-start out of a raw Claude Code PreToolUse(Task) hook input
// (which carries the subagent's task in tool_input).
function fromToolInput(
  ti: unknown,
  parent: number | undefined,
): DirectorCommand {
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
  return { cmd: "subagent-start", name, agentType, parent };
}

// Accepts an explicit command (normalizing its quoted `parent`), a raw Claude
// Code PreToolUse(Task) hook input, or the bus_cat_cmd wrapper
// {"parent":"<pty>","task":<raw hook input>} around one, and turns the Task
// shapes into a named subagent-start.
function toCommand(parsed: unknown): DirectorCommand | null {
  if (isCommand(parsed)) {
    if (
      parsed.cmd === "subagent-start" ||
      parsed.cmd === "subagent-stop" ||
      parsed.cmd === "director-active"
    ) {
      const raw = (parsed as { parent?: unknown }).parent;
      return { ...parsed, parent: parseParent(raw) };
    }
    return parsed;
  }
  if (!parsed || typeof parsed !== "object") return null;
  const obj = parsed as { parent?: unknown; task?: unknown };
  const parent = parseParent(obj.parent);
  if (obj.task && typeof obj.task === "object" && "tool_input" in obj.task) {
    return fromToolInput(
      (obj.task as { tool_input?: unknown }).tool_input,
      parent,
    );
  }
  if ("tool_input" in obj) {
    return fromToolInput((obj as { tool_input?: unknown }).tool_input, parent);
  }
  return null;
}

/**
 * Session attribution gate for Director dispatch: the bus file is shared by
 * every Koden pane's hooks, so hook-emitted lifecycle lines (director-active /
 * subagent-start / subagent-stop) must come from the Director's OWN session or
 * any pane's `claude` would steer the Director's roster. Lines with no parent
 * (pre-fix hooks still loaded in older sessions) are rejected too: the
 * Director itself always has fresh hooks (installed before it launches).
 * Explicit Director-authored commands (spawn/message/status) always pass.
 */
export function acceptDirectorCommand(
  cmd: DirectorCommand,
  directorPty: number | null,
): boolean {
  if (
    cmd.cmd !== "director-active" &&
    cmd.cmd !== "subagent-start" &&
    cmd.cmd !== "subagent-stop"
  ) {
    return true;
  }
  if (directorPty === null) return true;
  return cmd.parent === directorPty;
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
