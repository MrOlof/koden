import { describe, expect, it } from "vitest";
import { extractSubagentStarts } from "./subagentBus";

// Real corrupt line: two parallel Task hooks interleaved their non-atomic
// appends — doubled `subagent-start` wrapper, two payloads glued with `}{`, a
// stray `}` that landed on the NEXT file line.
const CORRUPT_TWO_PARALLEL =
  '{"cmd":"subagent-start","parent":29,"task":{"cmd":"subagent-start","parent":29,"task":' +
  '{"tool_name":"Task","tool_input":{"description":"Test agent 1","prompt":"do a","subagent_type":"general-purpose"},"tool_use_id":"call_ce64"}' +
  '{"tool_name":"Task","tool_input":{"description":"Test agent 2","prompt":"do b","subagent_type":"general-purpose"},"tool_use_id":"call_1a98"}}\n' +
  "}";

// Clean Max line: one well-formed subagent-start, one tool_use_id.
const CLEAN_SINGLE =
  '{"cmd":"subagent-start","parent":33,"task":{"tool_name":"Task","tool_input":{"description":"Test agent one","prompt":"x","subagent_type":"general-purpose"},"tool_use_id":"toolu_016"}}';

describe("extractSubagentStarts", () => {
  it("(a) recovers exactly 2 subagents from the corrupt two-parallel line", () => {
    const seen = new Set<string>();
    const got = extractSubagentStarts(CORRUPT_TWO_PARALLEL, seen);
    expect(got).toHaveLength(2);
    expect(got[0]).toEqual({
      parent: 29,
      description: "Test agent 1",
      subagentType: "general-purpose",
      toolUseId: "call_ce64",
    });
    expect(got[1]).toEqual({
      parent: 29,
      description: "Test agent 2",
      subagentType: "general-purpose",
      toolUseId: "call_1a98",
    });
  });

  it("(b) recovers 1 subagent from a clean single line", () => {
    const seen = new Set<string>();
    const got = extractSubagentStarts(CLEAN_SINGLE, seen);
    expect(got).toHaveLength(1);
    expect(got[0]).toEqual({
      parent: 33,
      description: "Test agent one",
      subagentType: "general-purpose",
      toolUseId: "toolu_016",
    });
  });

  it("(c) dedups across runs sharing the same seen set", () => {
    const seen = new Set<string>();
    expect(extractSubagentStarts(CORRUPT_TWO_PARALLEL, seen)).toHaveLength(2);
    // Re-reading the same content (e.g. the 400ms poll re-sees old bytes) must
    // not double-spawn.
    expect(extractSubagentStarts(CORRUPT_TWO_PARALLEL, seen)).toHaveLength(0);
    expect(extractSubagentStarts(CLEAN_SINGLE, seen)).toHaveLength(1);
    expect(extractSubagentStarts(CLEAN_SINGLE, seen)).toHaveLength(0);
  });

  it("(d) recovers a payload whose fields are split across a newline", () => {
    // The hook's three writes can land the parent + description on a different
    // file line than the tool_use_id. Joined with "\n", the scan still pairs
    // them via the nearest-preceding match.
    const split =
      '{"cmd":"subagent-start","parent":7,"task":{"tool_input":{"description":"Split task","subagent_type":"coder"\n,"prompt":"p"},"tool_use_id":"call_split"}}';
    const seen = new Set<string>();
    const got = extractSubagentStarts(split, seen);
    expect(got).toHaveLength(1);
    expect(got[0]).toEqual({
      parent: 7,
      description: "Split task",
      subagentType: "coder",
      toolUseId: "call_split",
    });
  });

  it("(e) yields nothing for agent-status / subagent-stop lines (no tool_use_id)", () => {
    const seen = new Set<string>();
    const status =
      '{"cmd":"agent-status","id":12,"state":"working"}\n' +
      '{"cmd":"subagent-stop","parent":12}';
    expect(extractSubagentStarts(status, seen)).toHaveLength(0);
  });

  it("parses the quoted parent the hook wrapper actually emits", () => {
    // bus_cat_cmd interpolates $KODEN_SESSION as a shell string, so parent
    // arrives quoted: {"parent":"5",...}. The old digits-only PARENT_RE never
    // matched it and every recovered subagent was dropped as parentless.
    const wrapped =
      '{"parent":"5","task":{"tool_name":"Task","tool_input":{"description":"Quoted parent","subagent_type":"worker"},"tool_use_id":"call_q1"}}';
    const seen = new Set<string>();
    const got = extractSubagentStarts(wrapped, seen);
    expect(got).toHaveLength(1);
    expect(got[0]).toEqual({
      parent: 5,
      description: "Quoted parent",
      subagentType: "worker",
      toolUseId: "call_q1",
    });
  });

  it("attributes interleaved corrupt wrappers to their quoted parents", () => {
    // Two parallel hooks from DIFFERENT panes interleave their non-atomic
    // writes; each payload must keep its own nearest-preceding parent.
    const corrupt =
      '{"parent":"5","task":{"parent":"5","task":' +
      '{"tool_input":{"description":"A","subagent_type":"worker"},"tool_use_id":"call_i1"}' +
      '{"parent":"8","task":{"tool_input":{"description":"B","subagent_type":"worker"},"tool_use_id":"call_i2"}}}\n' +
      "}";
    const seen = new Set<string>();
    const got = extractSubagentStarts(corrupt, seen);
    expect(got).toHaveLength(2);
    expect(got[0].parent).toBe(5);
    expect(got[1].parent).toBe(8);
    // Dedup on re-read still holds for the quoted shape.
    expect(extractSubagentStarts(corrupt, seen)).toHaveLength(0);
  });

  it("falls back to the last parent seen when a payload's prefix has none", () => {
    // Second payload's own `parent` wrapper got eaten by the interleave; it
    // inherits the running lastParent (15) from the first.
    const content =
      '{"parent":15,"task":{"tool_input":{"description":"first","subagent_type":"worker"},"tool_use_id":"id_a"}}' +
      '{"task":{"tool_input":{"description":"second","subagent_type":"worker"},"tool_use_id":"id_b"}}';
    const seen = new Set<string>();
    const got = extractSubagentStarts(content, seen);
    expect(got).toHaveLength(2);
    expect(got[0].parent).toBe(15);
    expect(got[1].parent).toBe(15);
  });
});
