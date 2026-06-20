import {
  readText as tauriReadText,
  writeText as tauriWriteText,
} from "@tauri-apps/plugin-clipboard-manager";

/**
 * Clipboard access that works inside the Tauri WebView. WebView2 on Windows
 * blocks the web `navigator.clipboard.readText()` (and gates writeText behind a
 * gesture), so we go through the Tauri clipboard plugin first and fall back to
 * the web API only when the native bridge is unavailable (e.g. plain-browser
 * dev or tests).
 */
export async function clipboardWriteText(text: string): Promise<void> {
  try {
    await tauriWriteText(text);
    return;
  } catch {
    // fall through to the web API
  }
  try {
    await navigator.clipboard?.writeText(text);
  } catch {
    // best-effort
  }
}

export async function clipboardReadText(): Promise<string> {
  try {
    return (await tauriReadText()) ?? "";
  } catch {
    // fall through to the web API
  }
  try {
    return (await navigator.clipboard?.readText()) ?? "";
  } catch {
    return "";
  }
}
