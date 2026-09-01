import { invoke } from "@tauri-apps/api/core";
import { chunkEnvelope, joinParts } from "./chunk";
import type { SyncDomain, SyncIndex, SyncPart } from "./types";

// Sync rides the existing space-manifest commands (ADR-023): atomic tmp+mv
// writes, stdin-piped (no shell quoting on any OS), 16 KB per manifest, key
// charset [A-Za-z0-9_-]. Reserved key namespace: sync-<domain>[-p<i>] —
// cannot collide with tmux-derived Space keys (those are p<fnv36>…).
const indexKey = (domain: SyncDomain) => `sync-${domain}`;
const partKey = (domain: SyncDomain, i: number) => `sync-${domain}-p${i}`;

// Mirrors Rust is_safe_ssh_host so a bad pref fails here with a clear message
// instead of deep in the command layer.
export function isValidSyncHost(host: string): boolean {
  if (host.length === 0 || host.length > 255) return false;
  if (host.startsWith("-")) return false;
  return (
    /^[A-Za-z0-9._@:-]+$/.test(host) && (host.match(/@/g) ?? []).length <= 1
  );
}

function readManifest(host: string, key: string): Promise<string> {
  return invoke<string>("ssh_read_space_manifest", { host, spaceKey: key });
}

function writeManifest(host: string, key: string, json: string): Promise<void> {
  return invoke<void>("ssh_write_space_manifest", {
    host,
    spaceKey: key,
    json,
  });
}

export type PullResult<T> =
  | { status: "unchanged" }
  | { status: "absent" }
  | { status: "ok"; gen: number; envelope: T }
  | { status: "error"; message: string };

function parseJson<T>(raw: string): T | null {
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

async function pullOnce<T>(
  host: string,
  domain: SyncDomain,
  lastGen: number,
): Promise<PullResult<T> | { status: "torn" }> {
  const rawIdx = await readManifest(host, indexKey(domain));
  if (rawIdx.trim() === "") return { status: "absent" };
  const index = parseJson<SyncIndex>(rawIdx);
  if (!index || typeof index.gen !== "number") return { status: "torn" };
  if (index.gen === lastGen) return { status: "unchanged" };
  const parts: SyncPart[] = [];
  for (let i = 0; i < index.of; i++) {
    const raw = await readManifest(host, partKey(domain, i));
    const part = parseJson<SyncPart>(raw);
    if (!part) return { status: "torn" };
    parts.push(part);
  }
  const envelope = joinParts<T>(index, parts);
  if (envelope === null) return { status: "torn" };
  return { status: "ok", gen: index.gen, envelope };
}

/** Pull a domain envelope. A torn read (racing a concurrent writer across
 * manifest files) retries once; a second tear reports as error and the next
 * scheduled pull tries again. */
export async function pullDomain<T>(
  host: string,
  domain: SyncDomain,
  lastGen: number,
): Promise<PullResult<T>> {
  try {
    let r = await pullOnce<T>(host, domain, lastGen);
    if (r.status === "torn") r = await pullOnce<T>(host, domain, lastGen);
    if (r.status === "torn") {
      return { status: "error", message: "torn read after retry" };
    }
    return r;
  } catch (e) {
    return {
      status: "error",
      message: e instanceof Error ? e.message : String(e),
    };
  }
}

/** Read just the remote gen (one round-trip); 0 when absent, null on error. */
export async function peekGen(
  host: string,
  domain: SyncDomain,
): Promise<number | null> {
  try {
    const raw = await readManifest(host, indexKey(domain));
    if (raw.trim() === "") return 0;
    const index = parseJson<SyncIndex>(raw);
    return index && typeof index.gen === "number" ? index.gen : null;
  } catch {
    return null;
  }
}

/** Write parts first, index last: a reader either sees the old index (old
 * parts still join under the old gen only if unchanged — otherwise its gen
 * check tears and it retries) or the new index over new parts. */
export async function pushDomain(
  host: string,
  domain: SyncDomain,
  value: unknown,
  deviceId: string,
): Promise<number> {
  const gen = Date.now();
  const { index, parts } = chunkEnvelope(value, gen, deviceId);
  for (const p of parts) {
    await writeManifest(host, partKey(domain, p.part), JSON.stringify(p));
  }
  await writeManifest(host, indexKey(domain), JSON.stringify(index));
  return gen;
}
