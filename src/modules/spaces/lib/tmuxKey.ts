import { spaceEnv } from "./envSwitch";
import type { SpaceMeta } from "./store";
import { useSpaces } from "./useSpaces";

/**
 * The tmux key a terminal spawned in `space` should attach to, or undefined
 * for a plain shell. Only ssh Spaces that opted in get one; the Rust side
 * turns the key into the `koden-<id>` session name.
 */
export function tmuxKeyFor(space: SpaceMeta | undefined): string | undefined {
  if (space?.sshTmux !== true) return undefined;
  return spaceEnv(space).kind === "ssh" ? space.id : undefined;
}

/** Read at spawn time, like the workspace env: the active Space decides. */
export function activeSpaceTmuxKey(): string | undefined {
  const { spaces, activeId } = useSpaces.getState();
  return tmuxKeyFor(spaces.find((s) => s.id === activeId));
}
