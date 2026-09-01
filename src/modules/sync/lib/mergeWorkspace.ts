import type { SpaceMeta, SpaceState } from "@/modules/spaces/lib/store";
import type { StateMeta, Tombstones, WorkspaceEnvelope } from "./types";

const TOMBSTONE_TTL_MS = 90 * 24 * 3600_000;

/** Union with max-clock per id, dropping entries past the TTL. Shared by the
 * envelope merge and the meta store's merge-write, so a delete recorded mid
 * sync cycle never loses to a stale snapshot. */
export function mergeTombstoneMaps(
  a: Tombstones,
  b: Tombstones,
  now: number = Date.now(),
): Tombstones {
  const out: Tombstones = {};
  for (const src of [a, b]) {
    for (const [id, at] of Object.entries(src)) {
      if (now - at > TOMBSTONE_TTL_MS) continue;
      out[id] = Math.max(out[id] ?? 0, at);
    }
  }
  return out;
}

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
  /** absorbed duplicate id -> surviving id (identity fold). Callers remap
   * references (activeId) through this before consulting removedSpaces. */
  idRemap: Record<string, string>;
  pushNeeded: boolean;
};

function normIdentityPath(p: string): string {
  let n = p.replace(/\\/g, "/").replace(/\/+$/, "");
  // Windows paths are case-insensitive; devices report drive/case variants.
  if (/^[A-Za-z]:\//.test(n)) n = n.toLowerCase();
  return n;
}

/**
 * Machine-independent identity of a space, or null when it has none worth
 * folding on. Spaces created independently per device (each "Open folder as
 * Space" of the same tree, each connect to the same ssh host+path) get
 * different random ids; without this, the first sync unions them into
 * side-by-side duplicates. Local/wsl identity only exists once the path-root
 * rewrite has made roots comparable; null roots never fold.
 */
export function spaceIdentityKey(s: SpaceMeta): string | null {
  if (s.worktree !== undefined) return null;
  const env = s.env ?? { kind: "local" };
  if (env.kind === "ssh")
    return `ssh:${env.host}:${normIdentityPath(env.path)}`;
  if (s.root == null) return null;
  const root = normIdentityPath(s.root);
  return env.kind === "wsl" ? `wsl:${env.distro}:${root}` : `local:${root}`;
}

/** Worktree Spaces are machine-local (absolute checkout paths under the repo);
 * they never travel. Applied on both push filtering and adoption. */
export function isSyncableSpace(s: SpaceMeta): boolean {
  return s.worktree === undefined;
}

export function mergeWorkspace(
  local: WorkspaceLocal,
  remote: WorkspaceEnvelope,
): WorkspaceMergeResult {
  const tombstones = mergeTombstoneMaps(
    local.tombstones,
    remote.tombstones ?? {},
  );

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

  // Identity fold: collapse per-device duplicates of the same real space.
  // Survivor = oldest createdAt (tiebreak: smaller id) so every machine picks
  // the same one; content = clock winner across the group; the best-stamped
  // layout follows the survivor.
  const idRemap: Record<string, string> = {};
  const byIdentity = new Map<string, SpaceMeta[]>();
  for (const s of spaces) {
    const key = spaceIdentityKey(s);
    if (!key) continue;
    const arr = byIdentity.get(key);
    if (arr) arr.push(s);
    else byIdentity.set(key, [s]);
  }
  const foldedById = new Map<string, SpaceMeta>();
  const dropIds = new Set<string>();
  for (const group of byIdentity.values()) {
    if (group.length < 2) continue;
    const survivor = [...group].sort(
      (a, b) => a.createdAt - b.createdAt || (a.id < b.id ? -1 : 1),
    )[0];
    const content = group.reduce((w, s) => pickSpace(w, s));
    const merged: SpaceMeta = {
      ...content,
      id: survivor.id,
      createdAt: survivor.createdAt,
      updatedAt: Math.max(...group.map((s) => s.updatedAt ?? 0)),
      ...(group.some((s) => s.contentUpdatedAt !== undefined)
        ? { contentUpdatedAt: Math.max(...group.map(contentClock)) }
        : {}),
    };
    foldedById.set(survivor.id, merged);
    changedSpaces.push(survivor.id);

    let bestId: string | null = null;
    let bestAt = -1;
    for (const s of group) {
      if (!states.has(s.id)) continue;
      const at = stateMeta[s.id]?.at ?? 0;
      if (
        bestId === null ||
        at > bestAt ||
        (at === bestAt && s.id === survivor.id)
      ) {
        bestId = s.id;
        bestAt = at;
      }
    }
    if (bestId !== null && bestId !== survivor.id) {
      const bestState = states.get(bestId);
      if (bestState) {
        states.set(survivor.id, bestState);
        stateMeta[survivor.id] = { at: bestAt };
        changedStates.push(survivor.id);
      }
    }
    for (const s of group) {
      if (s.id === survivor.id) continue;
      idRemap[s.id] = survivor.id;
      dropIds.add(s.id);
      states.delete(s.id);
      delete stateMeta[s.id];
      if (localById.has(s.id)) removedSpaces.push(s.id);
    }
  }
  // The merged space takes the position of the group's first-listed member,
  // so a fold never visually reorders the user's list.
  const foldedSpaces: SpaceMeta[] = [];
  const emitted = new Set<string>();
  for (const s of spaces) {
    const survivorId = idRemap[s.id] ?? (foldedById.has(s.id) ? s.id : null);
    if (survivorId === null) {
      foldedSpaces.push(s);
      continue;
    }
    if (emitted.has(survivorId)) continue;
    emitted.add(survivorId);
    const merged = foldedById.get(survivorId);
    if (merged) foldedSpaces.push(merged);
  }

  const pushNeeded =
    dropIds.size > 0 ||
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
    spaces: foldedSpaces,
    states,
    stateMeta,
    tombstones,
    changedSpaces: [...new Set(changedSpaces)],
    changedStates: [...new Set(changedStates)],
    removedSpaces: [...new Set(removedSpaces)],
    idRemap,
    pushNeeded,
  };
}
