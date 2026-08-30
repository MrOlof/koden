import {
  LOCAL_WORKSPACE,
  type WorkspaceEnv,
  workspaceEnvLabel,
  workspaceScopeKey,
} from "@/modules/workspace";
import type { SpaceMeta } from "./store";

export type EnvSwitchOptions = {
  /** ssh targets only: the tmux flag the resolved Space should carry. A
   * created Space starts with it, a reused one is updated to it. */
  sshTmux?: boolean;
};

export type EnvSwitch =
  | { action: "switch"; id: string; sshTmux?: boolean }
  | {
      action: "create";
      meta: { name: string; env: WorkspaceEnv; sshTmux?: boolean };
    };

/** Spaces files written before `env` existed are local. */
export function spaceEnv(space: SpaceMeta): WorkspaceEnv {
  return space.env ?? LOCAL_WORKSPACE;
}

export function envEquals(a: WorkspaceEnv, b: WorkspaceEnv): boolean {
  if (workspaceScopeKey(a) !== workspaceScopeKey(b)) return false;
  return a.kind !== "ssh" || b.kind !== "ssh" || a.path === b.path;
}

/** Local and WSL match on scope alone; an ssh target with a path only
 * matches a Space on that path, without one any Space on the host will do. */
export function spaceMatchesEnv(space: SpaceMeta, env: WorkspaceEnv): boolean {
  const own = spaceEnv(space);
  if (workspaceScopeKey(own) !== workspaceScopeKey(env)) return false;
  if (env.kind !== "ssh" || own.kind !== "ssh") return true;
  return env.path === "" || own.path === env.path;
}

export function spaceNameForEnv(env: WorkspaceEnv, localLabel: string): string {
  return env.kind === "ssh" ? env.host : workspaceEnvLabel(env, localLabel);
}

/**
 * Env is a property of the Space, so asking for another env means going to
 * a Space of that env: the active one if it already matches, else the most
 * recently used match, else a new one named after the env.
 */
export function resolveEnvSwitch(
  targetEnv: WorkspaceEnv,
  spaces: readonly SpaceMeta[],
  activeId: string | null,
  localLabel = "Local",
  options: EnvSwitchOptions = {},
): EnvSwitch {
  const tmux =
    targetEnv.kind === "ssh" && options.sshTmux !== undefined
      ? { sshTmux: options.sshTmux }
      : {};
  const active = spaces.find((s) => s.id === activeId);
  if (active && spaceMatchesEnv(active, targetEnv)) {
    return { action: "switch", id: active.id, ...tmux };
  }
  let best: SpaceMeta | null = null;
  for (const s of spaces) {
    if (!spaceMatchesEnv(s, targetEnv)) continue;
    if (!best || s.updatedAt > best.updatedAt) best = s;
  }
  if (best) return { action: "switch", id: best.id, ...tmux };
  return {
    action: "create",
    meta: {
      name: spaceNameForEnv(targetEnv, localLabel),
      env: targetEnv,
      ...tmux,
    },
  };
}
