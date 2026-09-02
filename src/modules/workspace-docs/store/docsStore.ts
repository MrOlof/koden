import { LazyStore } from "@tauri-apps/plugin-store";
import { create } from "zustand";

export type NoteDoc = { content: string; updatedAt: number };
export type BoardCard = { id: string; text: string };
export type BoardColumn = { id: string; title: string; cardIds: string[] };
export type Board = {
  columns: BoardColumn[];
  cards: Record<string, BoardCard>;
  updatedAt: number;
};
export type TaskItem = {
  id: string;
  text: string;
  done: boolean;
  createdAt: number;
};
export type TaskList = { items: TaskItem[]; updatedAt: number };

const STORE_PATH = "koden-workspace-docs.json";
// Staggered backup mirror. The Tauri store plugin overwrites in place with a
// non-atomic `fs::write` (tauri-plugin-store store.rs), so a power cut mid-write
// truncates the primary file and silently loses every note/board/task. The
// backup is written on a slower cadence than the primary, so a single power cut
// is very unlikely to catch both mid-write — a torn primary is recovered from
// the backup on next boot (see `hydrateDocs`).
const BACKUP_PATH = "koden-workspace-docs.bak.json";
const NOTE_PREFIX = "note:";
const BOARD_PREFIX = "board:";
const TASKS_PREFIX = "tasks:";
const store = new LazyStore(STORE_PATH, { defaults: {}, autoSave: 600 });
const backup = new LazyStore(BACKUP_PATH, { defaults: {}, autoSave: 4000 });

function uid(prefix: string): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

function now(): number {
  return Date.now();
}

export function defaultBoard(): Board {
  const cols: BoardColumn[] = [
    { id: uid("col"), title: "To Do", cardIds: [] },
    { id: uid("col"), title: "In Progress", cardIds: [] },
    { id: uid("col"), title: "Done", cardIds: [] },
  ];
  return { columns: cols, cards: {}, updatedAt: now() };
}

export function defaultTaskList(): TaskList {
  return { items: [], updatedAt: now() };
}

type DocsState = {
  notes: Record<string, NoteDoc>;
  boards: Record<string, Board>;
  tasks: Record<string, TaskList>;
  hydrated: boolean;

  setNote: (docId: string, content: string) => void;
  applyRemoteNote: (docId: string, doc: NoteDoc) => void;
  applyRemoteBoard: (boardId: string, board: Board) => void;
  applyRemoteTasks: (listId: string, list: TaskList) => void;

  ensureBoard: (boardId: string) => void;
  addCard: (boardId: string, columnId: string, text: string) => void;
  editCard: (boardId: string, cardId: string, text: string) => void;
  removeCard: (boardId: string, columnId: string, cardId: string) => void;
  moveCard: (boardId: string, cardId: string, toColumnId: string) => void;
  renameColumn: (boardId: string, columnId: string, title: string) => void;

  ensureTaskList: (listId: string) => void;
  addTask: (listId: string, text: string) => void;
  toggleTask: (listId: string, taskId: string) => void;
  editTask: (listId: string, taskId: string, text: string) => void;
  removeTask: (listId: string, taskId: string) => void;
  moveTask: (listId: string, taskId: string, dir: -1 | 1) => void;
  clearCompleted: (listId: string) => void;
};

// Every persisted doc is written to BOTH the primary store and the staggered
// backup. Writes are best-effort and swallow errors (e.g. in non-Tauri test
// envs, or transient FS hiccups) so a save never throws into the UI.
function persist(hydrated: boolean, key: string, value: unknown): void {
  if (!hydrated) return;
  void store.set(key, value).catch(() => {});
  void backup.set(key, value).catch(() => {});
}

function persistNote(hydrated: boolean, docId: string, doc: NoteDoc): void {
  persist(hydrated, `${NOTE_PREFIX}${docId}`, doc);
}

function persistBoard(hydrated: boolean, boardId: string, board: Board): void {
  persist(hydrated, `${BOARD_PREFIX}${boardId}`, board);
}

function persistTaskList(
  hydrated: boolean,
  listId: string,
  list: TaskList,
): void {
  persist(hydrated, `${TASKS_PREFIX}${listId}`, list);
}

export const useDocsStore = create<DocsState>((set, get) => ({
  notes: {},
  boards: {},
  tasks: {},
  hydrated: false,

  setNote: (docId, content) =>
    set((s) => {
      const doc: NoteDoc = { content, updatedAt: now() };
      persistNote(s.hydrated, docId, doc);
      return { notes: { ...s.notes, [docId]: doc } };
    }),

  // Remote-sync appliers (ssh Spaces, remoteDocs.ts): replace the whole doc
  // with the other device's copy, KEEPING its updatedAt — last-writer-wins
  // needs the remote timestamp, not now(), or every apply would win forever.
  applyRemoteNote: (docId, doc) =>
    set((s) => {
      persistNote(s.hydrated, docId, doc);
      return { notes: { ...s.notes, [docId]: doc } };
    }),
  applyRemoteBoard: (boardId, board) =>
    set((s) => {
      persistBoard(s.hydrated, boardId, board);
      return { boards: { ...s.boards, [boardId]: board } };
    }),
  applyRemoteTasks: (listId, list) =>
    set((s) => {
      persistTaskList(s.hydrated, listId, list);
      return { tasks: { ...s.tasks, [listId]: list } };
    }),

  ensureBoard: (boardId) => {
    if (get().boards[boardId]) return;
    set((s) => {
      if (s.boards[boardId]) return s;
      const board = defaultBoard();
      persistBoard(s.hydrated, boardId, board);
      return { boards: { ...s.boards, [boardId]: board } };
    });
  },

  addCard: (boardId, columnId, text) =>
    set((s) => {
      const board = s.boards[boardId];
      const trimmed = text.trim();
      if (!board || !trimmed) return s;
      const card: BoardCard = { id: uid("card"), text: trimmed };
      const next: Board = {
        ...board,
        cards: { ...board.cards, [card.id]: card },
        columns: board.columns.map((c) =>
          c.id === columnId ? { ...c, cardIds: [...c.cardIds, card.id] } : c,
        ),
        updatedAt: now(),
      };
      persistBoard(s.hydrated, boardId, next);
      return { boards: { ...s.boards, [boardId]: next } };
    }),

  editCard: (boardId, cardId, text) =>
    set((s) => {
      const board = s.boards[boardId];
      const trimmed = text.trim();
      if (!board || !board.cards[cardId] || !trimmed) return s;
      const next: Board = {
        ...board,
        cards: {
          ...board.cards,
          [cardId]: { ...board.cards[cardId], text: trimmed },
        },
        updatedAt: now(),
      };
      persistBoard(s.hydrated, boardId, next);
      return { boards: { ...s.boards, [boardId]: next } };
    }),

  removeCard: (boardId, columnId, cardId) =>
    set((s) => {
      const board = s.boards[boardId];
      if (!board) return s;
      const cards = { ...board.cards };
      delete cards[cardId];
      const next: Board = {
        ...board,
        cards,
        columns: board.columns.map((c) =>
          c.id === columnId
            ? { ...c, cardIds: c.cardIds.filter((id) => id !== cardId) }
            : c,
        ),
        updatedAt: now(),
      };
      persistBoard(s.hydrated, boardId, next);
      return { boards: { ...s.boards, [boardId]: next } };
    }),

  moveCard: (boardId, cardId, toColumnId) =>
    set((s) => {
      const board = s.boards[boardId];
      if (!board) return s;
      const from = board.columns.find((c) => c.cardIds.includes(cardId));
      if (!from || from.id === toColumnId) return s;
      const next: Board = {
        ...board,
        columns: board.columns.map((c) => {
          if (c.id === from.id)
            return { ...c, cardIds: c.cardIds.filter((id) => id !== cardId) };
          if (c.id === toColumnId)
            return { ...c, cardIds: [...c.cardIds, cardId] };
          return c;
        }),
        updatedAt: now(),
      };
      persistBoard(s.hydrated, boardId, next);
      return { boards: { ...s.boards, [boardId]: next } };
    }),

  renameColumn: (boardId, columnId, title) =>
    set((s) => {
      const board = s.boards[boardId];
      const trimmed = title.trim();
      if (!board || !trimmed) return s;
      const next: Board = {
        ...board,
        columns: board.columns.map((c) =>
          c.id === columnId ? { ...c, title: trimmed } : c,
        ),
        updatedAt: now(),
      };
      persistBoard(s.hydrated, boardId, next);
      return { boards: { ...s.boards, [boardId]: next } };
    }),

  ensureTaskList: (listId) => {
    if (get().tasks[listId]) return;
    set((s) => {
      if (s.tasks[listId]) return s;
      const list = defaultTaskList();
      persistTaskList(s.hydrated, listId, list);
      return { tasks: { ...s.tasks, [listId]: list } };
    });
  },

  addTask: (listId, text) =>
    set((s) => {
      const list = s.tasks[listId] ?? defaultTaskList();
      const trimmed = text.trim();
      if (!trimmed) return s;
      const item: TaskItem = {
        id: uid("task"),
        text: trimmed,
        done: false,
        createdAt: now(),
      };
      const next: TaskList = {
        items: [...list.items, item],
        updatedAt: now(),
      };
      persistTaskList(s.hydrated, listId, next);
      return { tasks: { ...s.tasks, [listId]: next } };
    }),

  toggleTask: (listId, taskId) =>
    set((s) => {
      const list = s.tasks[listId];
      if (!list) return s;
      if (!list.items.some((t) => t.id === taskId)) return s;
      const next: TaskList = {
        items: list.items.map((t) =>
          t.id === taskId ? { ...t, done: !t.done } : t,
        ),
        updatedAt: now(),
      };
      persistTaskList(s.hydrated, listId, next);
      return { tasks: { ...s.tasks, [listId]: next } };
    }),

  editTask: (listId, taskId, text) =>
    set((s) => {
      const list = s.tasks[listId];
      const trimmed = text.trim();
      if (!list || !trimmed) return s;
      if (!list.items.some((t) => t.id === taskId)) return s;
      const next: TaskList = {
        items: list.items.map((t) =>
          t.id === taskId ? { ...t, text: trimmed } : t,
        ),
        updatedAt: now(),
      };
      persistTaskList(s.hydrated, listId, next);
      return { tasks: { ...s.tasks, [listId]: next } };
    }),

  removeTask: (listId, taskId) =>
    set((s) => {
      const list = s.tasks[listId];
      if (!list) return s;
      if (!list.items.some((t) => t.id === taskId)) return s;
      const next: TaskList = {
        items: list.items.filter((t) => t.id !== taskId),
        updatedAt: now(),
      };
      persistTaskList(s.hydrated, listId, next);
      return { tasks: { ...s.tasks, [listId]: next } };
    }),

  moveTask: (listId, taskId, dir) =>
    set((s) => {
      const list = s.tasks[listId];
      if (!list) return s;
      const idx = list.items.findIndex((t) => t.id === taskId);
      const swap = idx + dir;
      if (idx < 0 || swap < 0 || swap >= list.items.length) return s;
      const items = [...list.items];
      [items[idx], items[swap]] = [items[swap], items[idx]];
      const next: TaskList = { items, updatedAt: now() };
      persistTaskList(s.hydrated, listId, next);
      return { tasks: { ...s.tasks, [listId]: next } };
    }),

  clearCompleted: (listId) =>
    set((s) => {
      const list = s.tasks[listId];
      if (!list) return s;
      if (!list.items.some((t) => t.done)) return s;
      const next: TaskList = {
        items: list.items.filter((t) => !t.done),
        updatedAt: now(),
      };
      persistTaskList(s.hydrated, listId, next);
      return { tasks: { ...s.tasks, [listId]: next } };
    }),
}));

/**
 * Force both the primary store and the backup mirror to flush to disk now.
 * Wired to window blur / hide / unload so at most a sub-second of typing is at
 * risk on a crash, instead of everything since the last 600ms autosave tick.
 */
export async function flushDocs(): Promise<void> {
  await Promise.allSettled([store.save(), backup.save()]);
}

let guardInstalled = false;
/** Register the crash-safety flush listeners exactly once. */
export function installDocsCrashGuard(): void {
  if (guardInstalled || typeof window === "undefined") return;
  guardInstalled = true;
  const flush = () => {
    void flushDocs();
  };
  const onVisibility = () => {
    if (document.visibilityState === "hidden") flush();
  };
  window.addEventListener("blur", flush);
  window.addEventListener("beforeunload", flush);
  document.addEventListener("visibilitychange", onVisibility);
}

function ingest(
  entries: [string, unknown][],
  notes: Record<string, NoteDoc>,
  boards: Record<string, Board>,
  tasks: Record<string, TaskList>,
): void {
  for (const [k, v] of entries) {
    if (k.startsWith(NOTE_PREFIX)) notes[k.slice(NOTE_PREFIX.length)] = v as NoteDoc;
    else if (k.startsWith(BOARD_PREFIX))
      boards[k.slice(BOARD_PREFIX.length)] = v as Board;
    else if (k.startsWith(TASKS_PREFIX))
      tasks[k.slice(TASKS_PREFIX.length)] = v as TaskList;
  }
}

// Shared in-flight promise (not a boolean): a second caller during boot
// hydration must WAIT for the real data, not return against an empty store —
// the Librarian's workspace_* tools hydrate on demand and would otherwise
// report "no tasks" while the boot load is still in flight.
let hydratePromise: Promise<{ recovered: boolean }> | null = null;
/**
 * Loads notes/boards/tasks from disk. If the primary file is corrupt (a torn
 * `fs::write` from a power cut throws on parse), falls back to the staggered
 * backup and heals the primary from it. A cleanly-empty primary is respected
 * (fresh install or user deleted everything) and never overwritten by a stale
 * backup. Returns `{ recovered }` so the caller can surface a recovery notice.
 */
export function hydrateDocs(): Promise<{ recovered: boolean }> {
  if (useDocsStore.getState().hydrated) {
    return Promise.resolve({ recovered: false });
  }
  if (hydratePromise) return hydratePromise;
  hydratePromise = hydrateDocsInner();
  return hydratePromise;
}

async function hydrateDocsInner(): Promise<{ recovered: boolean }> {
  let recovered = false;
  try {
    let entries: [string, unknown][] = [];
    let primaryThrew = false;
    try {
      entries = await store.entries();
    } catch {
      primaryThrew = true;
    }

    // Only treat the backup as a recovery source when the primary genuinely
    // failed to parse. A clean empty primary is a valid state we must keep.
    if (primaryThrew) {
      try {
        const backupEntries = await backup.entries();
        if (backupEntries.length > 0) {
          entries = backupEntries;
          recovered = true;
          // Heal the corrupt primary from the backup so the next boot is clean.
          for (const [k, v] of backupEntries) {
            void store.set(k, v).catch(() => {});
          }
          void store.save().catch(() => {});
        }
      } catch {
        // Both files unreadable: start empty rather than crash.
      }
    }

    const notes: Record<string, NoteDoc> = {};
    const boards: Record<string, Board> = {};
    const tasks: Record<string, TaskList> = {};
    ingest(entries, notes, boards, tasks);

    useDocsStore.setState((s) => ({
      notes: { ...notes, ...s.notes },
      boards: { ...boards, ...s.boards },
      tasks: { ...tasks, ...s.tasks },
      hydrated: true,
    }));
  } catch {
    useDocsStore.setState({ hydrated: true });
  } finally {
    installDocsCrashGuard();
  }
  return { recovered };
}
