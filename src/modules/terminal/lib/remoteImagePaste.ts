// Remote image paste (M2.9): terminal paste only transmits text — for
// images, Claude Code reads the OS clipboard of the machine IT runs on,
// which for an ssh tab is the headless host. So Koden reads the image from
// the LOCAL clipboard, ships it to `~/.koden/paste/` on the host, and
// inserts the remote path into the terminal — Claude Code attaches paths.

import { invoke } from "@tauri-apps/api/core";

const EXT_BY_MIME: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
};

export function imageFromClipboard(
  e: ClipboardEvent,
): { blob: Blob; ext: string } | null {
  for (const item of e.clipboardData?.items ?? []) {
    const ext = EXT_BY_MIME[item.type];
    if (!ext) continue;
    const blob = item.getAsFile();
    if (blob) return { blob, ext };
  }
  return null;
}

/** Uploads to the host and returns the remote absolute path. Two steps: the
 * raw bytes are staged instantly (sync command, no I/O), then shipped from
 * the blocking pool — no Tauri thread ever blocks on the network. */
export async function uploadPastedImage(
  host: string,
  blob: Blob,
  ext: string,
): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const id = await invoke<number>("ssh_paste_stage", bytes);
  return invoke<string>("ssh_paste_send", { host, id, ext });
}
