import { describe, expect, it } from "vitest";
import { type AgentBusState, readAgentBus } from "./agentBusReader";

const line = (o: object) => `${JSON.stringify(o)}\n`;

const FRESH: AgentBusState = { processed: 0, primed: false };

// The four verified session-5 prompts from the incident bus dump: every one
// must reach the turn store once the bridge tails the right file.
const FOUR_TURNS =
  line({ cmd: "user-turn", id: "5", data: { prompt: "hi" } }) +
  line({ cmd: "user-turn", id: "5", data: { prompt: "5+5" } }) +
  line({ cmd: "user-turn", id: "5", data: { prompt: "hiii" } }) +
  line({
    cmd: "user-turn",
    id: "5",
    data: { prompt: "30 countries list them" },
  });

describe("readAgentBus", () => {
  it("primes to the file end, skipping a previous run's backlog", () => {
    const seen = new Set<string>();
    const backlog = FOUR_TURNS;
    const { events, state } = readAgentBus(backlog, FRESH, seen);
    expect(events.turns).toHaveLength(0);
    expect(state).toEqual({ processed: 4, primed: true });
  });

  it("delivers all four appended user turns with quoted-id coercion", () => {
    const seen = new Set<string>();
    const { state: primed } = readAgentBus("", FRESH, seen);
    const { events, state } = readAgentBus(FOUR_TURNS, primed, seen);
    expect(events.turns).toEqual([
      { pty: 5, prompt: "hi", sessionId: null },
      { pty: 5, prompt: "5+5", sessionId: null },
      { pty: 5, prompt: "hiii", sessionId: null },
      { pty: 5, prompt: "30 countries list them", sessionId: null },
    ]);
    expect(state.processed).toBe(4);
  });

  it("captures the hook payload's session_id and rejects anything off the allowlist", () => {
    const seen = new Set<string>();
    const uuid = "0198d2fc-3c4b-7a10-9f2e-1b2c3d4e5f60";
    const short = "01C6fAURUuomAXbxsbYFRB2Rh";
    // A real-shaped UserPromptSubmit payload, then the same line without the
    // field, then garbage ids that must never reach a `--resume` splice.
    const content =
      line({
        cmd: "user-turn",
        id: "5",
        data: {
          session_id: uuid,
          transcript_path: "/home/me/.claude/projects/x/y.jsonl",
          cwd: "/work/proj",
          hook_event_name: "UserPromptSubmit",
          prompt: "fix the login bug",
        },
      }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: short, prompt: "b" } }) +
      line({ cmd: "user-turn", id: "5", data: { prompt: "no field" } }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: "../x", prompt: "p" } }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: "abc 123 def", prompt: "s" } }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: "", prompt: "e" } }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: "abcdefg", prompt: "7" } }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: "a".repeat(200), prompt: "l" } }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: "abc;rm -rf ~", prompt: "m" } }) +
      line({ cmd: "user-turn", id: "5", data: { session_id: 12345678, prompt: "n" } });
    const { events } = readAgentBus(content, { processed: 0, primed: true }, seen);
    expect(events.turns.map((t) => t.sessionId)).toEqual([
      uuid,
      short,
      ...Array<null>(8).fill(null),
    ]);
    // The prompt path is untouched by a bad id.
    expect(events.turns.map((t) => t.prompt)).toEqual([
      "fix the login bug",
      "b",
      "no field",
      "p",
      "s",
      "e",
      "7",
      "l",
      "m",
      "n",
    ]);
  });

  it("defers a partial trailing line until its newline arrives", () => {
    const seen = new Set<string>();
    const partial = `${FOUR_TURNS}{"cmd":"user-tur`;
    const first = readAgentBus(partial, { processed: 0, primed: true }, seen);
    expect(first.events.turns).toHaveLength(4);
    expect(first.state.processed).toBe(4);
    const completed = `${FOUR_TURNS}${line({
      cmd: "user-turn",
      id: "5",
      data: { prompt: "late" },
    })}`;
    const second = readAgentBus(completed, first.state, seen);
    expect(second.events.turns).toEqual([
      { pty: 5, prompt: "late", sessionId: null },
    ]);
  });

  it("resets to the top when the file shrank (truncation/rotation)", () => {
    const seen = new Set<string>(["stale_id"]);
    const truncated = line({
      cmd: "user-turn",
      id: "7",
      data: { prompt: "x" },
    });
    const { events, state } = readAgentBus(
      truncated,
      { processed: 9, primed: true },
      seen,
    );
    expect(events.turns).toEqual([{ pty: 7, prompt: "x", sessionId: null }]);
    expect(state.processed).toBe(1);
    expect(seen.size).toBe(0);
  });

  it("skips malformed lines, empty prompts and unknown commands", () => {
    const seen = new Set<string>();
    const content =
      "not json\n" +
      line({ cmd: "user-turn", id: "5", data: { prompt: "   " } }) +
      line({ cmd: "user-turn", data: { prompt: "no id" } }) +
      line({ cmd: "director-active", parent: "5" }) +
      line({ cmd: "user-turn", id: "5", data: { prompt: "ok" } });
    const { events } = readAgentBus(
      content,
      { processed: 0, primed: true },
      seen,
    );
    expect(events.turns).toEqual([{ pty: 5, prompt: "ok", sessionId: null }]);
  });

  it("parses agent-status and quoted-parent subagent-stop lines", () => {
    const seen = new Set<string>();
    const content =
      line({ cmd: "agent-status", id: 12, state: "working" }) +
      line({ cmd: "subagent-stop", parent: "12" });
    const { events } = readAgentBus(
      content,
      { processed: 0, primed: true },
      seen,
    );
    expect(events.statuses).toEqual([{ pty: 12, state: "working" }]);
    expect(events.stops).toEqual([{ parent: 12 }]);
  });

  it("recovers subagent-starts from the parent/task wrapper with dedup", () => {
    const seen = new Set<string>();
    const start = line({
      parent: "9",
      task: {
        tool_name: "Task",
        tool_input: { description: "Survey", subagent_type: "architect" },
        tool_use_id: "toolu_abc",
      },
    });
    const first = readAgentBus(start, { processed: 0, primed: true }, seen);
    expect(first.events.starts).toEqual([
      {
        parent: 9,
        description: "Survey",
        subagentType: "architect",
        toolUseId: "toolu_abc",
      },
    ]);
    // Re-reading the same bytes (poll overlap) never double-spawns.
    const again = readAgentBus(start, { processed: 0, primed: true }, seen);
    expect(again.events.starts).toHaveLength(0);
  });
});
