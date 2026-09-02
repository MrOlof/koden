import type { SpaceMeta, SpaceState } from "@/modules/spaces/lib/store";
import type {
  Board,
  NoteDoc,
  TaskList,
} from "@/modules/workspace-docs/store/docsStore";

export const SYNC_WIRE_VERSION = 1;

export type SyncDomain = "docs" | "ws";

/** Index manifest stored at sync-<domain>. `gen` changes on every push; an
 * unchanged gen lets a poll stop after one round-trip. */
export type SyncIndex = {
  v: number;
  gen: number;
  of: number;
  bytes: number;
  hash: string;
  deviceId: string;
  at: number;
};

/** Part manifest stored at sync-<domain>-p<i>. Carries gen so a reader that
 * races a writer across files can detect the mix and retry. */
export type SyncPart = {
  v: number;
  gen: number;
  part: number;
  data: string;
};

export type DocsEnvelope = {
  v: number;
  notes: Record<string, NoteDoc>;
  boards: Record<string, Board>;
  tasks: Record<string, TaskList>;
};

/** Per-space layout timestamp; `koden-spaces.json` states carried none. */
export type StateMeta = Record<string, { at: number }>;

/** spaceId -> deletion time. Wins over LRU updatedAt bumps but not over a
 * recreate or a content edit after the delete. */
export type Tombstones = Record<string, number>;

export type WorkspaceEnvelope = {
  v: number;
  spaces: SpaceMeta[];
  states: Record<string, SpaceState>;
  stateMeta: StateMeta;
  tombstones: Tombstones;
};
