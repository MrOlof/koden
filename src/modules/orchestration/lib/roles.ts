import type { AgentConfig, AgentLimits, AgentRole } from "./types";

const NO_LIMITS: AgentLimits = { contextLimit: null, costLimit: null };

/**
 * Sensible per-role starting configuration. High-capability roles get capable
 * models and broad permissions; review/audit roles get cheaper models and a
 * read-mostly tool surface. These are defaults the director can override per
 * agent, not hard policy.
 */
// `model` is a Claude Code `--model` alias (opus / sonnet / haiku) so it
// actually drives the agent's terminal session; the user can override it per
// agent. High-capability roles default to opus, review/audit to cheaper tiers.
const ROLE_DEFAULTS: Record<
  AgentRole,
  { model: string; permissions: string[]; tools: string[]; blurb: string }
> = {
  director: {
    model: "opus",
    permissions: ["spawn", "route", "approve", "fs.read", "shell.run"],
    tools: ["read_file", "list_directory", "fs_grep", "run_command"],
    blurb: "Plans work, spawns and routes agents, approves merges. Does not code.",
  },
  architect: {
    model: "opus",
    permissions: ["fs.read", "net"],
    tools: ["read_file", "list_directory", "fs_grep", "fs_search"],
    blurb: "Owns system design and cross-cutting decisions.",
  },
  coder: {
    model: "opus",
    permissions: ["fs.read", "fs.write", "shell.run", "git.commit"],
    tools: [
      "read_file",
      "write_file",
      "list_directory",
      "fs_grep",
      "run_command",
      "shell_session_run",
    ],
    blurb: "High-capability implementer.",
  },
  reviewer: {
    model: "sonnet",
    permissions: ["fs.read"],
    tools: ["read_file", "list_directory", "fs_grep"],
    blurb: "Reviews diffs for correctness and quality.",
  },
  auditor: {
    model: "haiku",
    permissions: ["fs.read"],
    tools: ["read_file", "fs_grep"],
    blurb: "Cheap second-opinion review pass.",
  },
  qa: {
    model: "sonnet",
    permissions: ["fs.read", "shell.run"],
    tools: ["read_file", "run_command", "shell_session_run"],
    blurb: "Runs and verifies tests.",
  },
  devops: {
    model: "sonnet",
    permissions: ["fs.read", "shell.run", "net"],
    tools: ["read_file", "run_command", "shell_bg_spawn"],
    blurb: "Builds, deploys, infrastructure.",
  },
  worker: {
    model: "sonnet",
    permissions: ["fs.read", "fs.write", "shell.run"],
    tools: ["read_file", "write_file", "run_command"],
    blurb: "General-purpose task runner.",
  },
};

/** Claude Code `--model` aliases offered in the model picker. */
export const MODEL_ALIASES = ["opus", "sonnet", "haiku"] as const;

export function roleBlurb(role: AgentRole): string {
  return ROLE_DEFAULTS[role].blurb;
}

export function defaultConfigForRole(role: AgentRole): AgentConfig {
  const d = ROLE_DEFAULTS[role];
  return {
    model: d.model,
    limits: { ...NO_LIMITS },
    permissions: [...d.permissions],
    tools: [...d.tools],
  };
}
