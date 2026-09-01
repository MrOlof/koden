import { LazyStore } from "@tauri-apps/plugin-store";
import type { WorkspaceEnv } from "@/modules/workspace";
import type { SerializedTab } from "./serialize";

export type SpaceWorktree = {
  repoRoot: string;
  branch: string;
};

export type SpaceMeta = {
  id: string;
  name: string;
  root: string | null;
  env: WorkspaceEnv;
  /** Opt-in accent, index into SPACE_COLORS. Undefined = theme primary. */
  color?: number;
  /** Set when the Space root is a git worktree Koden created. */
  worktree?: SpaceWorktree;
  /** ssh Spaces only: run each terminal inside a tmux session on the host
   * (named after the Space id) so it survives the client. */
  sshTmux?: boolean;
  createdAt: number;
  updatedAt: number;
  /** Bumped by content mutations only (create/rename/color/sshTmux), never by
   * setActive — updatedAt doubles as an LRU clock for the launcher, which
   * makes it unusable for cross-machine merge (ADR-023). */
  contentUpdatedAt?: number;
};

export type SpaceState = {
  tabs: SerializedTab[];
  activeTabIndex: number;
};

const STORE_PATH = "koden-spaces.json";
const KEY_SPACES = "spaces";
const KEY_ACTIVE = "activeId";
const STATE_PREFIX = "state:";
// Layout snapshots carried no timestamp; cross-machine merge needs one
// (ADR-023). Written beside each state. "stateMeta:x" does not match the
// "state:" prefix scan (the char after "state" is "M", not ":").
const STATE_META_PREFIX = "stateMeta:";
const stateKey = (id: string) => `${STATE_PREFIX}${id}`;
const stateMetaKey = (id: string) => `${STATE_META_PREFIX}${id}`;

const store = new LazyStore(STORE_PATH, { defaults: {}, autoSave: 500 });

export type SpaceStateMeta = { at: number };

export type LoadedSpaces = {
  spaces: SpaceMeta[];
  activeId: string | null;
  states: Map<string, SpaceState>;
  stateMeta: Map<string, SpaceStateMeta>;
};

export async function loadAll(): Promise<LoadedSpaces> {
  const entries = await store.entries();
  let spaces: SpaceMeta[] = [];
  let activeId: string | null = null;
  const states = new Map<string, SpaceState>();
  const stateMeta = new Map<string, SpaceStateMeta>();
  for (const [k, v] of entries) {
    if (k === KEY_SPACES) spaces = (v as SpaceMeta[]) ?? [];
    else if (k === KEY_ACTIVE) activeId = (v as string | null) ?? null;
    else if (k.startsWith(STATE_META_PREFIX)) {
      stateMeta.set(k.slice(STATE_META_PREFIX.length), v as SpaceStateMeta);
    } else if (k.startsWith(STATE_PREFIX)) {
      states.set(k.slice(STATE_PREFIX.length), v as SpaceState);
    }
  }
  return { spaces, activeId, states, stateMeta };
}

export async function saveSpacesList(spaces: SpaceMeta[]): Promise<void> {
  await store.set(KEY_SPACES, spaces);
}

export async function saveActiveId(id: string | null): Promise<void> {
  await store.set(KEY_ACTIVE, id);
}

export async function saveState(
  id: string,
  state: SpaceState,
  at: number = Date.now(),
): Promise<void> {
  await store.set(stateKey(id), state);
  await store.set(stateMetaKey(id), { at } satisfies SpaceStateMeta);
}

export async function deleteSpaceData(id: string): Promise<void> {
  await store.delete(stateKey(id));
  await store.delete(stateMetaKey(id));
}

export function newSpaceId(): string {
  return `sp-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}
