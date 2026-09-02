import { LazyStore } from "@tauri-apps/plugin-store";
import { mergeTombstoneMaps } from "./mergeWorkspace";
import type { SyncDomain, Tombstones } from "./types";

// Machine-local sync bookkeeping. Never synced itself: the device id is the
// first per-machine identity in the app, and tombstones pend here until a
// push lands them in the shared envelope.
const STORE_PATH = "koden-sync-meta.json";
const KEY_DEVICE = "deviceId";
const KEY_TOMBSTONES = "tombstones";
const genKey = (domain: SyncDomain) => `gen:${domain}`;

const store = new LazyStore(STORE_PATH, { defaults: {}, autoSave: 300 });

let deviceIdCache: string | null = null;

export async function getDeviceId(): Promise<string> {
  if (deviceIdCache) return deviceIdCache;
  const existing = await store.get<string>(KEY_DEVICE).catch(() => undefined);
  if (existing) {
    deviceIdCache = existing;
    return existing;
  }
  const minted = `dev-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
  deviceIdCache = minted;
  void store.set(KEY_DEVICE, minted).catch(() => {});
  return minted;
}

/** Last remote gen this device has fully merged, per domain. */
export async function getLastGen(domain: SyncDomain): Promise<number> {
  const v = await store.get<number>(genKey(domain)).catch(() => undefined);
  return typeof v === "number" ? v : 0;
}

export async function setLastGen(
  domain: SyncDomain,
  gen: number,
): Promise<void> {
  await store.set(genKey(domain), gen).catch(() => {});
}

export async function getLocalTombstones(): Promise<Tombstones> {
  const v = await store.get<Tombstones>(KEY_TOMBSTONES).catch(() => undefined);
  return v ?? {};
}

export async function recordTombstone(spaceId: string): Promise<void> {
  const all = await getLocalTombstones();
  all[spaceId] = Date.now();
  await store.set(KEY_TOMBSTONES, all).catch(() => {});
}

/** MERGES into the stored set (never replaces): a tombstone recorded between
 * a sync cycle's snapshot and this write must survive, or the delete silently
 * un-happens on the next merge. TTL-pruned to keep the file bounded. */
export async function setLocalTombstones(t: Tombstones): Promise<void> {
  const current = await getLocalTombstones();
  await store
    .set(KEY_TOMBSTONES, mergeTombstoneMaps(current, t))
    .catch(() => {});
}

const KEY_WS_SIG = "wsSig";
const KEY_WS_BOOT_FAILS = "wsBootFails";

/** Signature of the last successfully pushed ws envelope. Persisted so a
 * fresh boot doesn't re-push (and re-gen) a byte-identical envelope. */
export async function getWsSig(): Promise<string> {
  const v = await store.get<string>(KEY_WS_SIG).catch(() => undefined);
  return v ?? "";
}

export async function setWsSig(sig: string): Promise<void> {
  await store.set(KEY_WS_SIG, sig).catch(() => {});
}

/** Consecutive boot-pull failures; shrinks the next boot's wait budget so an
 * unreachable sync host doesn't stall every launch. */
export async function getBootFailCount(): Promise<number> {
  const v = await store.get<number>(KEY_WS_BOOT_FAILS).catch(() => undefined);
  return typeof v === "number" ? v : 0;
}

export async function setBootFailCount(n: number): Promise<void> {
  await store.set(KEY_WS_BOOT_FAILS, n).catch(() => {});
}
