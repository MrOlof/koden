const STORAGE_KEY = "koden.agentCommand";
// Defaults to the user's `cm` alias (PowerShell wrapper that cd's to the synced
// folder and starts Claude Code). Override in the Director's Launch command
// field for a plain `claude` or any other wrapper.
const DEFAULT_COMMAND = "cm";
// The real coding-agent binary. Used when a launch must forward extra flags
// (--model / --append-system-prompt / --agents): the default `cm` wrapper ends
// with `& $cmd.Source` and does NOT forward `@args`, so any appended flags are
// silently dropped. For those launches we bypass the wrapper and invoke the
// binary directly (the wrapper's only other job — cd'ing to the synced folder —
// is redundant because the agent PTY is already spawned in the workspace cwd).
const ARGS_FALLBACK_COMMAND = "claude";

/**
 * The base command used to launch a coding agent in a terminal. Defaults to
 * `claude`, but users with a wrapper alias (e.g. `cm` that cd's to the project
 * and starts Claude Code) can set their own. Persisted in localStorage like the
 * other shell-chrome settings. Model and prompt flags are appended to this.
 */
export function getAgentCommand(): string {
  try {
    return window.localStorage.getItem(STORAGE_KEY)?.trim() || DEFAULT_COMMAND;
  } catch {
    return DEFAULT_COMMAND;
  }
}

/**
 * Resolve the stored launch command to one that is SAFE to append flags to.
 *
 * The shipped default `cm` is a known arg-dropping wrapper (its PowerShell
 * profile body is `& $cmd.Source`, no `@args`), so launching an agent WITH
 * flags through it silently loses `--model` / `--append-system-prompt` /
 * `--agents`. When the command is the default, substitute the real `claude`
 * binary so the flags survive. A user-set CUSTOM command is respected verbatim
 * — they chose it deliberately, and forwarding args is then their wrapper's job.
 *
 * Pure (no DOM) so it's unit-testable; `getAgentCommandWithArgs` reads storage.
 */
export function agentCommandForArgs(stored: string): string {
  return stored === DEFAULT_COMMAND ? ARGS_FALLBACK_COMMAND : stored;
}

/**
 * Like {@link getAgentCommand}, but for launches that APPEND flags. Use this
 * (not `getAgentCommand`) wherever `--model` / `--append-system-prompt` /
 * `--agents` are added, so the args aren't dropped by the `cm` wrapper.
 */
export function getAgentCommandWithArgs(): string {
  return agentCommandForArgs(getAgentCommand());
}

export function setAgentCommand(cmd: string): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, cmd.trim() || DEFAULT_COMMAND);
  } catch {
    // storage may be unavailable
  }
}
