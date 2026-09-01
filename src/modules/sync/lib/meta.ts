import { LazyStore } from "@tauri-apps/plugin-store";
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

/** Replace local pending tombstones with the merged set after a push, so the
 * file doesn't grow forever; the shared envelope is the durable record. */
export async function setLocalTombstones(t: Tombstones): Promise<void> {
  await store.set(KEY_TOMBSTONES, t).catch(() => {});
}
