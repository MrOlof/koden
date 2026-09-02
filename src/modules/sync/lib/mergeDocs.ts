import type {
  Board,
  NoteDoc,
  TaskList,
} from "@/modules/workspace-docs/store/docsStore";
import type { DocsEnvelope } from "./types";

type Stamped = { updatedAt: number };

/** Per-entry last-writer-wins. Docs entries never get deleted by the store,
 * so union + newer-wins is complete; concurrent edits of the SAME entry on
 * two offline machines resolve to the newer clock (ADR-023 assumption). */
function mergeMap<T extends Stamped>(
  local: Record<string, T>,
  remote: Record<string, T>,
): { merged: Record<string, T>; changedLocal: string[] } {
  const merged: Record<string, T> = { ...local };
  const changedLocal: string[] = [];
  for (const [id, r] of Object.entries(remote)) {
    const l = local[id];
    if (!l || (r.updatedAt ?? 0) > (l.updatedAt ?? 0)) {
      merged[id] = r;
      changedLocal.push(id);
    }
  }
  return { merged, changedLocal };
}

export type DocsMergeResult = {
  notes: Record<string, NoteDoc>;
  boards: Record<string, Board>;
  tasks: Record<string, TaskList>;
  /** ids adopted from remote, per kind — the entries the store must persist. */
  adopted: { notes: string[]; boards: string[]; tasks: string[] };
  /** true when local has anything remote lacks or beats — a push is due. */
  pushNeeded: boolean;
};

function localBeats<T extends Stamped>(
  local: Record<string, T>,
  remote: Record<string, T>,
): boolean {
  for (const [id, l] of Object.entries(local)) {
    const r = remote[id];
    if (!r || (l.updatedAt ?? 0) > (r.updatedAt ?? 0)) return true;
  }
  return false;
}

export function mergeDocs(
  local: Pick<DocsEnvelope, "notes" | "boards" | "tasks">,
  remote: Pick<DocsEnvelope, "notes" | "boards" | "tasks">,
): DocsMergeResult {
  const notes = mergeMap(local.notes, remote.notes);
  const boards = mergeMap(local.boards, remote.boards);
  const tasks = mergeMap(local.tasks, remote.tasks);
  return {
    notes: notes.merged,
    boards: boards.merged,
    tasks: tasks.merged,
    adopted: {
      notes: notes.changedLocal,
      boards: boards.changedLocal,
      tasks: tasks.changedLocal,
    },
    pushNeeded:
      localBeats(local.notes, remote.notes) ||
      localBeats(local.boards, remote.boards) ||
      localBeats(local.tasks, remote.tasks),
  };
}
