import type { SpaceMeta, SpaceState } from "@/modules/spaces/lib/store";
import type {
  SerializedNode,
  SerializedTab,
} from "@/modules/spaces/lib/serialize";

// Absolute paths (leaf cwd, space root, editor/markdown tab paths) are
// machine-specific. Each device configures its tree root (syncPathRoot pref);
// on push the root prefix becomes a wire token, on pull the token becomes the
// local root. Paths outside the root travel verbatim and simply may not exist
// on the other machine — boot already tolerates that (allSettled authorize,
// cold tabs).
export const WIRE_ROOT = "~SYNCROOT~";

function normSlashes(p: string): string {
  return p.replace(/\\/g, "/");
}

/** Trailing-slash-insensitive prefix swap in forward-slash form. */
function swapPrefix(path: string, from: string, to: string): string {
  const p = normSlashes(path);
  const f = normSlashes(from).replace(/\/+$/, "");
  if (f.length === 0) return p;
  if (p === f) return to;
  if (p.startsWith(`${f}/`)) return to + p.slice(f.length);
  return p;
}

export function toWirePath(path: string, localRoot: string): string {
  if (!localRoot) return normSlashes(path);
  return swapPrefix(path, localRoot, WIRE_ROOT);
}

export function fromWirePath(path: string, localRoot: string): string {
  if (!localRoot) return path;
  return swapPrefix(
    path,
    WIRE_ROOT,
    normSlashes(localRoot).replace(/\/+$/, ""),
  );
}

function mapNode(
  node: SerializedNode,
  map: (p: string) => string,
): SerializedNode {
  if (node.kind === "split") {
    return { ...node, children: node.children.map((c) => mapNode(c, map)) };
  }
  return node.cwd !== undefined ? { ...node, cwd: map(node.cwd) } : node;
}

function mapTab(tab: SerializedTab, map: (p: string) => string): SerializedTab {
  switch (tab.kind) {
    case "terminal":
      return { ...tab, tree: mapNode(tab.tree, map) };
    case "editor":
    case "markdown":
      return { ...tab, path: map(tab.path) };
    default:
      return tab;
  }
}

export function mapStatePaths(
  state: SpaceState,
  map: (p: string) => string,
): SpaceState {
  return { ...state, tabs: state.tabs.map((t) => mapTab(t, map)) };
}

/** ssh Spaces are untouched: their root and leaf cwds are paths on the REMOTE
 * host and are already device-independent. Callers must also skip their
 * states in mapStatePaths. */
export function mapSpacePaths(
  space: SpaceMeta,
  map: (p: string) => string,
): SpaceMeta {
  if (space.env.kind === "ssh") return space;
  return space.root != null ? { ...space, root: map(space.root) } : space;
}
