// Remote image paste (M2.9): terminal paste only transmits text — for
// images, Claude Code reads the OS clipboard of the machine IT runs on,
// which for an ssh tab is the headless host. So Koden reads the image from
// the LOCAL clipboard, ships it to `~/.koden/paste/` on the host, and
// inserts the remote path into the terminal — Claude Code attaches paths.

import { currentWorkspaceEnv } from "@/modules/workspace";
import { invoke } from "@tauri-apps/api/core";
import { readImage } from "@tauri-apps/plugin-clipboard-manager";
import { toast } from "sonner";

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

/** The Ctrl+V path: the terminal's key handler intercepts paste before any
 * browser paste event exists (WebView2 blocks web clipboard reads anyway),
 * so the image comes from the Tauri clipboard plugin as raw RGBA and gets
 * PNG-encoded through a canvas before the trip to the host. `write` is the
 * pane's pty writer, injected to keep this module out of the terminal-lib
 * import cycle. */
export async function pasteClipboardImageToRemote(
  write: (data: string) => void,
): Promise<void> {
  const env = currentWorkspaceEnv();
  if (env.kind !== "ssh") return;
  let rgba: Uint8Array;
  let width: number;
  let height: number;
  try {
    const img = await readImage();
    const size = await img.size();
    rgba = new Uint8Array(await img.rgba());
    width = size.width;
    height = size.height;
  } catch {
    toast.error("No image on the clipboard.");
    return;
  }
  if (!width || !height || rgba.length !== width * height * 4) {
    toast.error("Couldn't read the clipboard image.");
    return;
  }
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    toast.error("Couldn't encode the clipboard image.");
    return;
  }
  ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba), width, height), 0, 0);
  const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
  if (!blob) {
    toast.error("Couldn't encode the clipboard image.");
    return;
  }
  const uploading = toast.loading("Sending image to the host…");
  try {
    const path = await uploadPastedImage(env.host, blob, "png");
    toast.dismiss(uploading);
    write(`'${path}' `);
  } catch (e) {
    toast.dismiss(uploading);
    toast.error(`Image paste failed: ${String(e)}`);
  }
}
