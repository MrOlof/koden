import { describe, expect, it } from "vitest";
import { agentCommandForArgs } from "./agentCommand";

// The default launch command `cm` is a PowerShell wrapper whose body is
// `& $cmd.Source` — it does NOT forward `@args`, so launching an agent with
// flags through it silently drops --model / --append-system-prompt / --agents.
// `agentCommandForArgs` is the guard that swaps the arg-dropping default for the
// real `claude` binary while leaving any user-chosen custom command alone.
describe("agentCommandForArgs", () => {
  it("swaps the arg-dropping `cm` default for the real `claude` binary", () => {
    expect(agentCommandForArgs("cm")).toBe("claude");
  });

  it("respects a user-set custom command verbatim", () => {
    expect(agentCommandForArgs("claude")).toBe("claude");
    expect(agentCommandForArgs("my-wrapper")).toBe("my-wrapper");
    // A custom wrapper that itself forwards args is the user's responsibility.
    expect(agentCommandForArgs("glm")).toBe("glm");
  });

  it("keeps appended flags in the assembled launch command", () => {
    // Mirrors how App.tsx assembles a worker launch: base + appended flags.
    const base = agentCommandForArgs("cm");
    const parts = [base, "--model", "sonnet", "--append-system-prompt", "PROMPT"];
    const command = parts.join(" ");
    // The base must be the real binary (so the flags aren't dropped) and every
    // appended flag must survive into the final command string.
    expect(command).toBe("claude --model sonnet --append-system-prompt PROMPT");
    expect(command).toContain("--model");
    expect(command).toContain("--append-system-prompt");
    expect(command.startsWith("cm ")).toBe(false);
  });

  it("keeps --agents in a Director-style command", () => {
    const command = `${agentCommandForArgs(
      "cm",
    )} --model opus --append-system-prompt P --agents J @args`;
    expect(command).toContain("--agents");
    expect(command).toContain("@args");
    expect(command.startsWith("claude ")).toBe(true);
  });
});
