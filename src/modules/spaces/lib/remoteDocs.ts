// Host-side docs manifest for ssh+tmux Spaces (M2.5 F2 extension, 2026-09-02):
// notes / tasks / boards are part of the workspace, so they live on the host
// beside the tab manifest (`~/.koden/spaces/<tmuxKey>-docs.json`) and every
// device renders the same set. Whole-doc last-writer-wins on `updatedAt` —
// docs are low-contention; terminals are the high-stakes path and stay in
// tmux. Pure planning logic here; the App wires it to the store.

export type RemoteDocKind = "notes" | "tasks" | "board";

export type RemoteDoc = {
  kind: RemoteDocKind;
  /** docId / listId / boardId — shared verbatim across devices. */
  id: string;
  /** Tab title as shown. */
  title: string;
  /** NoteDoc / TaskList / Board object, passed through untouched. */
  payload: unknown;
  updatedAt: number;
};

const KINDS: readonly RemoteDocKind[] = ["notes", "tasks", "board"];

export function buildDocsManifest(docs: readonly RemoteDoc[]): string {
  return JSON.stringify({ v: 1, docs, updatedAt: Date.now() });
}

/** Tolerant parse: absent / torn / foreign json → null (meaning "no remote
 * truth yet" — callers must NOT treat that as "everything was deleted"). */
export function parseDocsManifest(json: string): RemoteDoc[] | null {
  try {
    const m = JSON.parse(json) as { docs?: unknown[] };
    if (!Array.isArray(m.docs)) return null;
    const out: RemoteDoc[] = [];
    for (const d of m.docs) {
      const doc = d as Partial<RemoteDoc>;
      if (
        typeof doc.id === "string" &&
        doc.id &&
        typeof doc.title === "string" &&
        KINDS.includes(doc.kind as RemoteDocKind) &&
        typeof doc.updatedAt === "number"
      ) {
        out.push({
          kind: doc.kind as RemoteDocKind,
          id: doc.id,
          title: doc.title,
          payload: doc.payload,
          updatedAt: doc.updatedAt,
        });
      }
    }
    return out;
  } catch {
    return null;
  }
}

export type LocalDocTab = { kind: RemoteDocKind; id: string };

export type DocsPlan = {
  /** Doc tabs to create locally (background, no focus steal). */
  create: RemoteDoc[];
  /** Docs whose remote payload is newer than the local copy. */
  apply: RemoteDoc[];
  /** Local tab ids (doc ids) to close: present before, gone from remote now. */
  close: LocalDocTab[];
};

/**
 * Diff remote truth against local state.
 *
 * `seenBefore` is the set of doc ids this device has already observed in the
 * remote manifest ("kind:id"). Close ONLY docs that were seen remotely and
 * vanished — a locally-created tab the other side never knew about must
 * survive until our own push publishes it (otherwise first-connect races
 * would eat fresh work).
 */
export function planDocsApply(
  remote: readonly RemoteDoc[],
  localTabs: readonly LocalDocTab[],
  localUpdatedAt: (kind: RemoteDocKind, id: string) => number | undefined,
  seenBefore: ReadonlySet<string>,
): DocsPlan {
  const remoteIds = new Set(remote.map((d) => `${d.kind}:${d.id}`));
  const localIds = new Set(localTabs.map((t) => `${t.kind}:${t.id}`));
  const create: RemoteDoc[] = [];
  const apply: RemoteDoc[] = [];
  for (const d of remote) {
    if (!localIds.has(`${d.kind}:${d.id}`)) create.push(d);
    const localAt = localUpdatedAt(d.kind, d.id);
    if (d.payload !== undefined && (localAt === undefined || d.updatedAt > localAt)) {
      apply.push(d);
    }
  }
  const close = localTabs.filter(
    (t) => seenBefore.has(`${t.kind}:${t.id}`) && !remoteIds.has(`${t.kind}:${t.id}`),
  );
  return { create, apply, close };
}

/** The "kind:id" keys of a remote snapshot, for the caller's seenBefore set. */
export function docKeys(remote: readonly RemoteDoc[]): Set<string> {
  return new Set(remote.map((d) => `${d.kind}:${d.id}`));
}
