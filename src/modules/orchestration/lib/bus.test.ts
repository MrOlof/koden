import { describe, expect, it } from "vitest";
import {
  acceptDirectorCommand,
  type DirectorCommand,
  readNewCommands,
} from "./bus";

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
    const text =
      'not json\n{"foo":1}\n{"cmd":"status","agent":"QA","status":"done"}\n';
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

  it("unwraps the parent-stamped PreToolUse wrapper into a subagent-start", () => {
    // bus_cat_cmd now writes {"parent":"<pty>","task":<raw hook input>}.
    const raw =
      '{"parent":"29","task":{"tool_name":"Task","tool_input":{"description":"Audit auth","subagent_type":"architect"},"tool_use_id":"t1"}}\n';
    const { commands } = readNewCommands(raw, 0);
    expect(commands).toHaveLength(1);
    expect(commands[0]).toEqual({
      cmd: "subagent-start",
      name: "Audit auth",
      agentType: "architect",
      parent: 29,
    });
  });

  it("coerces the quoted parent on hook-emitted lifecycle commands", () => {
    const raw =
      '{"cmd":"subagent-stop","parent":"7"}\n{"cmd":"director-active","parent":"7"}\n';
    const { commands } = readNewCommands(raw, 0);
    expect(commands).toEqual([
      { cmd: "subagent-stop", parent: 7 },
      { cmd: "director-active", parent: 7 },
    ]);
  });
});

describe("acceptDirectorCommand", () => {
  const stop = (parent?: number): DirectorCommand => ({
    cmd: "subagent-stop",
    parent,
  });

  it("rejects lifecycle lines from a foreign session's hooks", () => {
    // Any Koden pane's claude writes to the same bus: without the gate a
    // plain conversation in another pane retires the Director's children.
    expect(acceptDirectorCommand(stop(12), 5)).toBe(false);
    expect(
      acceptDirectorCommand({ cmd: "director-active", parent: 12 }, 5),
    ).toBe(false);
    expect(
      acceptDirectorCommand(
        { cmd: "subagent-start", name: "X", parent: 12 },
        5,
      ),
    ).toBe(false);
  });

  it("accepts the Director session's own lifecycle lines", () => {
    expect(acceptDirectorCommand(stop(5), 5)).toBe(true);
    expect(
      acceptDirectorCommand({ cmd: "director-active", parent: 5 }, 5),
    ).toBe(true);
  });

  it("rejects parentless lifecycle lines while the Director pty is known", () => {
    // Sessions started before the parent-stamping hooks were installed still
    // emit bare commands; the Director itself always has fresh hooks.
    expect(acceptDirectorCommand(stop(undefined), 5)).toBe(false);
  });

  it("always passes explicit Director-authored commands", () => {
    expect(acceptDirectorCommand({ cmd: "spawn", task: "build" }, 5)).toBe(
      true,
    );
    expect(acceptDirectorCommand({ cmd: "message", text: "hi" }, 5)).toBe(true);
    expect(
      acceptDirectorCommand({ cmd: "status", agent: "QA", status: "done" }, 5),
    ).toBe(true);
  });

  it("accepts everything when no Director pty was recorded", () => {
    expect(acceptDirectorCommand(stop(12), null)).toBe(true);
  });
});
