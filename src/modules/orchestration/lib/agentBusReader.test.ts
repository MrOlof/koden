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
      { pty: 5, prompt: "hi" },
      { pty: 5, prompt: "5+5" },
      { pty: 5, prompt: "hiii" },
      { pty: 5, prompt: "30 countries list them" },
    ]);
    expect(state.processed).toBe(4);
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
    expect(second.events.turns).toEqual([{ pty: 5, prompt: "late" }]);
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
    expect(events.turns).toEqual([{ pty: 7, prompt: "x" }]);
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
    expect(events.turns).toEqual([{ pty: 5, prompt: "ok" }]);
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
