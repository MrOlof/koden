import { spaceEnv } from "./envSwitch";
import type { SpaceMeta } from "./store";
import { useSpaces } from "./useSpaces";

function fnv1a(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/**
 * Device-independent tmux key for a remote workspace: derived from the
 * REMOTE PATH, not the local Space id, so every device that connects to the
 * same host+path lands in the same live session and adoption merges their
 * tabs ("the spaces are the same despite device"). Hash first — the readable
 * path tail may be truncated, the identity never is. Only this function
 * computes the key; the Rust side just prefixes/sanitizes it verbatim.
 */
export function pathTmuxKey(path: string): string {
  const hash = fnv1a(path).toString(36);
  const tail = path.replace(/[^A-Za-z0-9_]+/g, "-").replace(/^-+|-+$/g, "");
  return `p${hash}${tail ? `-${tail}` : ""}`
    .slice(0, 40)
    .replace(/-+$/, "");
}

/**
 * The tmux key a terminal spawned in `space` should attach to, or undefined
 * for a plain shell. Only ssh Spaces that opted in get one; the Rust side
 * turns the key into the `koden-<key>` session name.
 */
export function tmuxKeyFor(space: SpaceMeta | undefined): string | undefined {
  if (space?.sshTmux !== true) return undefined;
  const env = spaceEnv(space);
  return env.kind === "ssh" ? pathTmuxKey(env.path) : undefined;
}

/** Read at spawn time, like the workspace env: the active Space decides. */
export function activeSpaceTmuxKey(): string | undefined {
  const { spaces, activeId } = useSpaces.getState();
  return tmuxKeyFor(spaces.find((s) => s.id === activeId));
}
