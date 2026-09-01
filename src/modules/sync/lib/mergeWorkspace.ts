import type { SpaceMeta, SpaceState } from "@/modules/spaces/lib/store";
import type { StateMeta, Tombstones, WorkspaceEnvelope } from "./types";

// Space content merges on contentUpdatedAt because updatedAt is deliberately
// an LRU clock (setActive bumps it for the launcher's Continue list) — a mere
// visit must never beat a rename from another machine.
function contentClock(s: SpaceMeta): number {
  return s.contentUpdatedAt ?? 0;
}

/** Tombstone wins unless the space was recreated after the delete or its
 * content was edited after the delete. LRU updatedAt cannot resurrect. */
function isDeleted(s: SpaceMeta, tombstones: Tombstones): boolean {
  const deletedAt = tombstones[s.id];
  if (deletedAt === undefined) return false;
  if (s.createdAt > deletedAt) return false;
  return contentClock(s) <= deletedAt;
}

function pickSpace(local: SpaceMeta, remote: SpaceMeta): SpaceMeta {
  const lc = contentClock(local);
  const rc = contentClock(remote);
  if (rc !== lc) return rc > lc ? remote : local;
  if ((remote.updatedAt ?? 0) > (local.updatedAt ?? 0)) return remote;
  return local;
}

export type WorkspaceLocal = {
  spaces: SpaceMeta[];
  states: Map<string, SpaceState>;
  stateMeta: StateMeta;
  tombstones: Tombstones;
};

export type WorkspaceMergeResult = {
  spaces: SpaceMeta[];
  states: Map<string, SpaceState>;
  stateMeta: StateMeta;
  tombstones: Tombstones;
  /** space ids whose meta or layout changed vs the local input — what the
   * caller must persist locally. */
  changedSpaces: string[];
  changedStates: string[];
  removedSpaces: string[];
  pushNeeded: boolean;
};

/** Worktree Spaces are machine-local (absolute checkout paths under the repo);
 * they never travel. Applied on both push filtering and adoption. */
export function isSyncableSpace(s: SpaceMeta): boolean {
  return s.worktree === undefined;
}

export function mergeWorkspace(
  local: WorkspaceLocal,
  remote: WorkspaceEnvelope,
): WorkspaceMergeResult {
  const tombstones: Tombstones = { ...local.tombstones };
  for (const [id, at] of Object.entries(remote.tombstones ?? {})) {
    tombstones[id] = Math.max(tombstones[id] ?? 0, at);
  }

  const localById = new Map(local.spaces.map((s) => [s.id, s]));
  const remoteById = new Map(
    (remote.spaces ?? []).filter(isSyncableSpace).map((s) => [s.id, s]),
  );

  const changedSpaces: string[] = [];
  const removedSpaces: string[] = [];
  const spaces: SpaceMeta[] = [];

  // Local order is preserved; unseen remote spaces append in remote order.
  for (const l of local.spaces) {
    const winner = (() => {
      if (!isSyncableSpace(l)) return l;
      const r = remoteById.get(l.id);
      return r ? pickSpace(l, r) : l;
    })();
    if (isSyncableSpace(l) && isDeleted(winner, tombstones)) {
      removedSpaces.push(l.id);
      continue;
    }
    if (winner !== l) changedSpaces.push(l.id);
    spaces.push(winner);
  }
  for (const r of remote.spaces ?? []) {
    if (!isSyncableSpace(r)) continue;
    if (localById.has(r.id)) continue;
    if (isDeleted(r, tombstones)) continue;
    spaces.push(r);
    changedSpaces.push(r.id);
  }

  // Layout snapshots: per-space LWW on stateMeta.at. A side that has a stamp
  // beats a side that lacks one; both unstamped keeps local (pre-sync data).
  const keptIds = new Set(spaces.map((s) => s.id));
  const states = new Map(local.states);
  const stateMeta: StateMeta = { ...local.stateMeta };
  const changedStates: string[] = [];
  for (const [id, remoteState] of Object.entries(remote.states ?? {})) {
    if (!keptIds.has(id)) continue;
    const localSpace = localById.get(id);
    if (localSpace && !isSyncableSpace(localSpace)) continue;
    const lAt = local.stateMeta[id]?.at ?? 0;
    const rAt = remote.stateMeta?.[id]?.at ?? 0;
    if (!states.has(id) || rAt > lAt) {
      states.set(id, remoteState);
      stateMeta[id] = { at: rAt };
      changedStates.push(id);
    }
  }
  for (const id of removedSpaces) {
    states.delete(id);
    delete stateMeta[id];
  }

  const pushNeeded =
    local.spaces.some((l) => {
      if (!isSyncableSpace(l)) return false;
      if (removedSpaces.includes(l.id)) return true;
      const r = remoteById.get(l.id);
      return !r || contentClock(l) > contentClock(r);
    }) ||
    Object.entries(local.stateMeta).some(([id, m]) => {
      if (!keptIds.has(id)) return false;
      return (m?.at ?? 0) > (remote.stateMeta?.[id]?.at ?? 0);
    }) ||
    Object.entries(local.tombstones).some(
      ([id, at]) => at > (remote.tombstones?.[id] ?? 0),
    );

  return {
    spaces,
    states,
    stateMeta,
    tombstones,
    changedSpaces,
    changedStates,
    removedSpaces,
    pushNeeded,
  };
}
