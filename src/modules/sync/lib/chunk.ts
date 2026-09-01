import { SYNC_WIRE_VERSION, type SyncIndex, type SyncPart } from "./types";

// The transport is ssh_write_space_manifest, capped at 16 KB of JSON per
// manifest. Envelopes are base64'd (stable chunk sizes regardless of what
// JSON-escaping the payload would need) and sliced under the cap with room
// for the part wrapper.
export const PART_DATA_CHARS = 10_800;

/** FNV-1a over the base64 string; cheap integrity check for reassembly. */
export function fnv1a(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(36);
}

export function encodeEnvelope(value: unknown): string {
  const json = JSON.stringify(value);
  const bytes = new TextEncoder().encode(json);
  let bin = "";
  // 8 KB stride keeps the btoa argument bounded (String.fromCharCode limit).
  for (let i = 0; i < bytes.length; i += 8192) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 8192));
  }
  return btoa(bin);
}

export function decodeEnvelope<T>(b64: string): T {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return JSON.parse(new TextDecoder().decode(bytes)) as T;
}

export type ChunkedWrite = { index: SyncIndex; parts: SyncPart[] };

export function chunkEnvelope(
  value: unknown,
  gen: number,
  deviceId: string,
): ChunkedWrite {
  const b64 = encodeEnvelope(value);
  const parts: SyncPart[] = [];
  for (let i = 0; i * PART_DATA_CHARS < b64.length; i++) {
    parts.push({
      v: SYNC_WIRE_VERSION,
      gen,
      part: i,
      data: b64.slice(i * PART_DATA_CHARS, (i + 1) * PART_DATA_CHARS),
    });
  }
  if (parts.length === 0) {
    parts.push({ v: SYNC_WIRE_VERSION, gen, part: 0, data: "" });
  }
  return {
    index: {
      v: SYNC_WIRE_VERSION,
      gen,
      of: parts.length,
      bytes: b64.length,
      hash: fnv1a(b64),
      deviceId,
      at: Date.now(),
    },
    parts,
  };
}

/** Reassemble pulled parts against their index. Returns null on any
 * inconsistency (gen mix from racing a writer, wrong count, hash mismatch)
 * so the caller can retry the pull once and otherwise fail soft. */
export function joinParts<T>(index: SyncIndex, parts: SyncPart[]): T | null {
  if (parts.length !== index.of) return null;
  const ordered = [...parts].sort((a, b) => a.part - b.part);
  let b64 = "";
  for (let i = 0; i < ordered.length; i++) {
    const p = ordered[i];
    if (p.part !== i || p.gen !== index.gen) return null;
    b64 += p.data;
  }
  if (b64.length !== index.bytes || fnv1a(b64) !== index.hash) return null;
  try {
    return decodeEnvelope<T>(b64);
  } catch {
    return null;
  }
}
