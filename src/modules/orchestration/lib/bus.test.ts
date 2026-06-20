import { describe, expect, it } from "vitest";
import { readNewCommands } from "./bus";

describe("readNewCommands", () => {
  it("returns only complete lines and advances the offset", () => {
    const text =
      '{"cmd":"spawn","role":"coder","task":"a"}\n{"cmd":"message","text":"hi"}\n';
    const { commands, processedLines } = readNewCommands(text, 0);
    expect(commands).toHaveLength(2);
    expect(processedLines).toBe(2);
  });

  it("does not process a trailing partial line until it is completed", () => {
    const partial = '{"cmd":"spawn","role":"coder","task":"a"}\n{"cmd":"sp';
    const first = readNewCommands(partial, 0);
    expect(first.commands).toHaveLength(1);
    expect(first.processedLines).toBe(1);

    const completed = `${partial}awn","role":"qa","task":"b"}\n`;
    const second = readNewCommands(completed, first.processedLines);
    expect(second.commands).toHaveLength(1);
    expect(second.commands[0]).toMatchObject({ cmd: "spawn", role: "qa" });
  });

  it("skips malformed and unknown lines", () => {
    const text = 'not json\n{"foo":1}\n{"cmd":"status","agent":"QA","status":"done"}\n';
    const { commands } = readNewCommands(text, 0);
    expect(commands).toHaveLength(1);
    expect(commands[0]).toMatchObject({ cmd: "status" });
  });

  it("processes nothing when already caught up", () => {
    const text = '{"cmd":"message","text":"x"}\n';
    expect(readNewCommands(text, 1).commands).toHaveLength(0);
  });

  it("derives a named subagent-start (with agentType) from a raw Task hook input", () => {
    const raw =
      '{"tool_name":"Task","tool_input":{"description":"Survey the repo","subagent_type":"architect","prompt":"long..."}}\n';
    const { commands } = readNewCommands(raw, 0);
    expect(commands).toHaveLength(1);
    expect(commands[0]).toEqual({
      cmd: "subagent-start",
      name: "Survey the repo",
      agentType: "architect",
    });
  });

  it("falls back to subagent_type when no description is present", () => {
    const raw = '{"tool_input":{"subagent_type":"code-reviewer"}}\n';
    const { commands } = readNewCommands(raw, 0);
    expect(commands[0]).toEqual({
      cmd: "subagent-start",
      name: "code-reviewer",
      agentType: "code-reviewer",
    });
  });
});
