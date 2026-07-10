/**
 * Tolerant recovery of `subagent-start` Task events from the agent bus.
 *
 * The Claude Code hook that records a Task subagent is NOT atomic: it does
 * `{ printf prefix; cat stdin; printf suffix } >> director-bus.jsonl` as three
 * separate writes. When the model dispatches two Task subagents in PARALLEL the
 * two hook processes interleave their appends, producing corrupt, multi-line,
 * nested-and-concatenated JSON (doubled `subagent-start` wrappers, payloads
 * glued with `}{`, a stray `}` that lands on the next file line). A per-line
 * `JSON.parse` silently drops those subagents — which is why a session that
 * dispatches Tasks concurrently shows no subagent nodes while one that happens
 * to serialize them shows clean lines.
 *
 * The fix is to stop trusting the line framing for subagent-start and instead
 * scan the raw text. Every real Task subagent carries a UNIQUE `tool_use_id`,
 * and within its payload the `description` and `subagent_type` precede that id,
 * while the agent-bus `parent` (pty) precedes the whole payload. So we recover
 * subagents by `tool_use_id` regardless of how the surrounding JSON is mangled,
 * dedup on the id (a Set), and never double-spawn from the doubled wrapper,
 * re-reads, or duplicated fragments.
 *
 * Pure: no React/Tauri/store imports, so it is unit-testable in isolation.
 */

export type SubagentStart = {
  /** Owning terminal's pty id (Claude Code KODEN_SESSION). */
  parent: number;
  /** Task description (may be empty if the hook only carried a type). */
  description: string;
  /** Task `subagent_type` (may be empty). */
  subagentType: string;
  /** Unique Task tool_use_id — the dedup + identity key. */
  toolUseId: string;
};

// Every real Task subagent has a unique tool_use_id; this is the anchor we
// scan for. `g` so we can walk every occurrence in the new content.
const TOOL_USE_ID_RE = /"tool_use_id"\s*:\s*"([^"\\]*)"/g;
// Fields that PRECEDE the tool_use_id within the same payload. We search the
// prefix (text up to the id match) and take the LAST occurrence, so an earlier
// payload's value never bleeds into a later one.
const DESCRIPTION_RE = /"description"\s*:\s*"((?:[^"\\]|\\.)*)"/g;
const SUBAGENT_TYPE_RE = /"subagent_type"\s*:\s*"([^"\\]*)"/g;
// The agent-bus parent (pty) wrapper precedes the payload. The hook writes it
// as a quoted shell interpolation ("parent":"5"); bare digits also accepted.
const PARENT_RE = /"parent"\s*:\s*"?(\d+)"?/g;

/** Last (right-most) capture of a global regex within `text`, or null. */
function lastMatch(re: RegExp, text: string): string | null {
  re.lastIndex = 0;
  let found: string | null = null;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    found = m[1];
    // Guard against zero-width matches looping forever.
    if (m.index === re.lastIndex) re.lastIndex++;
  }
  return found;
}

/**
 * Extract every new Task `subagent-start` from `content`, keyed by tool_use_id.
 *
 * @param content New bus text to scan (typically the newly-completed lines for
 *   this tick joined with "\n", so a payload split across a file-line boundary
 *   is still scanned as one string).
 * @param seen    Persistent set of tool_use_ids already spawned. Ids found here
 *   are added to it and skipped, so re-reads / doubled wrappers / duplicated
 *   fragments never double-spawn. Mutated in place (caller owns the ref).
 * @returns One entry per NEW tool_use_id, in file order.
 */
export function extractSubagentStarts(
  content: string,
  seen: Set<string>,
): SubagentStart[] {
  const out: SubagentStart[] = [];
  // Running parent fallback: if a payload's prefix has no `parent` of its own
  // (e.g. the wrapper for the second interleaved Task got eaten), reuse the
  // last parent seen earlier in the content, walking ids in file order.
  let lastParent: number | null = null;

  TOOL_USE_ID_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = TOOL_USE_ID_RE.exec(content)) !== null) {
    const toolUseId = m[1];
    const idEnd = TOOL_USE_ID_RE.lastIndex;
    if (m.index === idEnd) TOOL_USE_ID_RE.lastIndex++; // zero-width guard

    // The payload for THIS id lives in the text before the id match.
    const prefix = content.slice(0, m.index);

    const parentStr = lastMatch(PARENT_RE, prefix);
    if (parentStr !== null) lastParent = Number(parentStr);
    const parent = parentStr !== null ? Number(parentStr) : lastParent;

    if (seen.has(toolUseId)) continue; // dedup — still update lastParent above
    seen.add(toolUseId);

    if (parent === null) continue; // no pty to attach to, anywhere

    const description = lastMatch(DESCRIPTION_RE, prefix) ?? "";
    const subagentType = lastMatch(SUBAGENT_TYPE_RE, prefix) ?? "";

    out.push({ parent, description, subagentType, toolUseId });
  }

  return out;
}
