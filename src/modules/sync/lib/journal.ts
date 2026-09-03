// Sync journal (ADR-025): every change a PEER makes to a tab this device
// already shows: at boot or live: is recorded here, so nothing the sync
// does to a layout is silent or unrecoverable. Ring-bounded; machine-local;
// never synced itself.
import { LazyStore } from "@tauri-apps/plugin-store";

export type JournalEntry = {
  at: number;
  spaceId: string;
  /** Tab identity (tabClocks.tabIdentity), stable across devices. */
  tabId: string;
  /** "title" for renames; "layout" for a boot-time pane-tree replacement;
   * "closed" when a peer's tombstone removed the tab at boot. */
  field: "title" | "layout" | "closed";
  before: string;
  after: string;
  /** Device that authored the winning value, when the envelope says. */
  fromDevice?: string;
  /** How the change landed: a live poll or the boot merge. */
  via: "live" | "boot";
};

const STORE_PATH = "koden-sync-journal.json";
const KEY = "entries";
const RING = 100;

const store = new LazyStore(STORE_PATH, { defaults: {}, autoSave: 300 });

export async function readJournal(): Promise<JournalEntry[]> {
  const v = await store.get<JournalEntry[]>(KEY).catch(() => undefined);
  return Array.isArray(v) ? v : [];
}

export async function appendJournal(
  entries: readonly JournalEntry[],
): Promise<void> {
  if (entries.length === 0) return;
  const all = [...(await readJournal()), ...entries];
  await store.set(KEY, all.slice(-RING)).catch(() => {});
}
