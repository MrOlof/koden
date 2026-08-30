import { LazyStore } from "@tauri-apps/plugin-store";
import type { Tab } from "@/modules/tabs/lib/useTabs";
import { isLeaf, type PaneNode } from "@/modules/terminal/lib/panes";
import { isSerializableTab } from "./serialize";

// Per-leaf scrollback snapshots for the cross-launch restore. Kept in its own
// store file so the layout file (koden-spaces.json) stays small and cheap to
// write; entries are keyed by space + a restart-stable leaf key that the
// layout file carries on each leaf node.

/** Hard cap per snapshot (in chars; ANSI text is ASCII-dominant). Older lines
 * are trimmed first, at a line boundary. */
export const SNAPSHOT_MAX_CHARS = 512 * 1024;
const STORE_PATH = "koden-scrollback.json";
const ENTRY_PREFIX = "snap:";
const KEY_RE = /^[A-Za-z0-9_-]{1,64}$/;

export type SnapshotEntry = { text: string; at: number };

// Runtime leaf ids are re-allocated on every boot; the restore key is the
// identity that survives. Seeded from the layout file on hydrate, minted on
// first serialize for new leaves, pruned as leaves disappear.
const leafKeys = new Map<number, string>();

export function mintRestoreKey(): string {
  return `rk-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export function isValidRestoreKey(k: unknown): k is string {
  return typeof k === "string" && KEY_RE.test(k);
}

export function leafRestoreKey(leafId: number): string {
  let k = leafKeys.get(leafId);
  if (!k) {
    k = mintRestoreKey();
    leafKeys.set(leafId, k);
  }
  return k;
}

export function peekLeafRestoreKey(leafId: number): string | undefined {
  return leafKeys.get(leafId);
}

export function seedLeafRestoreKey(leafId: number, key: string): void {
  if (isValidRestoreKey(key)) leafKeys.set(leafId, key);
}

export function pruneLeafRestoreKeys(live: ReadonlySet<number>): void {
  for (const id of [...leafKeys.keys()]) if (!live.has(id)) leafKeys.delete(id);
}

export function resetLeafRestoreKeys(): void {
  leafKeys.clear();
}

export function snapshotEntryKey(spaceId: string, leafKey: string): string {
  return `${ENTRY_PREFIX}${spaceId}/${leafKey}`;
}

/** Trim to the last `max` chars at a line boundary. A cut mid-line could split
 * an escape sequence, so with no boundary in the tail nothing is kept; the SGR
 * state active at the cut is lost either way, so the kept tail is prefixed
 * with a reset. */
export function capSnapshotText(
  text: string,
  max: number = SNAPSHOT_MAX_CHARS,
): string {
  if (text.length <= max) return text;
  const from = text.length - max;
  const crlf = text.indexOf("\r\n", from);
  const lf = crlf >= 0 ? -1 : text.indexOf("\n", from);
  const cut = crlf >= 0 ? crlf + 2 : lf >= 0 ? lf + 1 : -1;
  if (cut < 0 || cut >= text.length) return "";
  return `\x1b[0m${text.slice(cut)}`;
}

export type RestorableLeaf = { leafId: number; spaceId: string; key: string };

function walkLeaves(node: PaneNode, fn: (leaf: PaneNode) => void): void {
  if (isLeaf(node)) {
    fn(node);
    return;
  }
  for (const c of node.children) walkLeaves(c, fn);
}

/** Every terminal leaf id in the tab list (any tab kind that has a pane tree). */
export function terminalLeafIds(tabs: Tab[]): Set<number> {
  const out = new Set<number>();
  for (const t of tabs) {
    if (t.kind !== "terminal") continue;
    walkLeaves(t.paneTree, (leaf) => {
      if (isLeaf(leaf)) out.add(leaf.id);
    });
  }
  return out;
}

/** Leaves whose scrollback may be snapshotted: shell leaves of serializable
 * terminal tabs only. Private tabs fail the serializable gate (their buffers
 * never reach disk); blocks terminals are marker-structured, so a raw replay
 * has no meaning there; note/task panes have no xterm. */
export function restorableLeaves(
  tabs: Tab[],
  keyFor: (leafId: number) => string,
): RestorableLeaf[] {
  const out: RestorableLeaf[] = [];
  for (const t of tabs) {
    if (t.kind !== "terminal" || t.blocks || !isSerializableTab(t)) continue;
    walkLeaves(t.paneTree, (leaf) => {
      if (!isLeaf(leaf) || leaf.content !== undefined) return;
      out.push({ leafId: leaf.id, spaceId: t.spaceId, key: keyFor(leaf.id) });
    });
  }
  return out;
}

export type SavePlan = {
  /** entry key -> capped snapshot text */
  writes: Map<string, string>;
  /** entry keys that belong to a still-existing leaf (never gc these) */
  keep: Set<string>;
};

/** Pure save plan. `capture` returns null for a leaf that has not rendered
 * yet, whose on-disk entry is then kept as-is rather than overwritten. */
export function planScrollbackSave(
  tabs: Tab[],
  capture: (leafId: number) => string | null,
  keyFor: (leafId: number) => string,
  maxChars: number = SNAPSHOT_MAX_CHARS,
): SavePlan {
  const writes = new Map<string, string>();
  const keep = new Set<string>();
  for (const leaf of restorableLeaves(tabs, keyFor)) {
    const entry = snapshotEntryKey(leaf.spaceId, leaf.key);
    keep.add(entry);
    const text = capture(leaf.leafId);
    if (text === null) continue;
    const capped = capSnapshotText(text, maxChars);
    if (capped) writes.set(entry, capped);
  }
  return { writes, keep };
}

/** Entry keys on disk that no existing leaf owns (closed panes, deleted spaces). */
export function planGc(
  existing: Iterable<string>,
  keep: ReadonlySet<string>,
): string[] {
  const out: string[] = [];
  for (const k of existing) if (k.startsWith(ENTRY_PREFIX) && !keep.has(k)) out.push(k);
  return out;
}

// FNV-1a: cheap change detection so an unchanged buffer costs no IPC.
function hashText(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

let store: LazyStore | null = null;
function getStore(): LazyStore {
  store ??= new LazyStore(STORE_PATH, { defaults: {}, autoSave: false });
  return store;
}

const known = new Set<string>();
let knownSeeded = false;
const written = new Map<string, number>();
let queue: Promise<void> = Promise.resolve();

async function seedKnown(): Promise<void> {
  if (knownSeeded) return;
  knownSeeded = true;
  for (const k of await getStore().keys()) known.add(k);
}

async function writePlan(plan: SavePlan): Promise<void> {
  await seedKnown();
  const s = getStore();
  let dirty = false;
  for (const k of planGc(known, plan.keep)) {
    await s.delete(k);
    known.delete(k);
    written.delete(k);
    dirty = true;
  }
  for (const [k, text] of plan.writes) {
    const h = hashText(text);
    if (known.has(k) && written.get(k) === h) continue;
    await s.set(k, { text, at: Date.now() } satisfies SnapshotEntry);
    known.add(k);
    written.set(k, h);
    dirty = true;
  }
  if (dirty) await s.save();
}

/** Capture (synchronously, a copy at call time) and persist. Writes are
 * serialized so an interval flush and a close flush never interleave. */
export function saveScrollbackSnapshots(
  tabs: Tab[],
  capture: (leafId: number) => string | null,
): Promise<void> {
  const plan = planScrollbackSave(tabs, capture, leafRestoreKey);
  pruneLeafRestoreKeys(terminalLeafIds(tabs));
  queue = queue
    .then(() => writePlan(plan))
    .catch((e) => console.warn("[koden] scrollback save failed:", e));
  return queue;
}

/** One read of every snapshot on disk (boot). entry key -> text. */
export async function loadScrollbackSnapshots(): Promise<Map<string, string>> {
  const out = new Map<string, string>();
  const entries = await getStore().entries();
  knownSeeded = true;
  for (const [k, v] of entries) {
    if (!k.startsWith(ENTRY_PREFIX)) continue;
    known.add(k);
    const e = v as Partial<SnapshotEntry> | null;
    if (e && typeof e.text === "string" && e.text) out.set(k, e.text);
  }
  return out;
}

export async function clearScrollbackSnapshots(): Promise<void> {
  const s = getStore();
  await s.clear();
  await s.save();
  known.clear();
  written.clear();
  knownSeeded = true;
}
