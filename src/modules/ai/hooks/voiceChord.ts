import type { KeyBinding } from "@/modules/shortcuts/shortcuts";

/**
 * Pure chord helpers for the voice hotkey (keyup semantics). Kept free of
 * runtime imports so tests run without the Tauri platform shim.
 */

/** Tap = toggle listening; a hold past this stops + transcribes on release. */
export const HOLD_TO_TALK_MS = 350;

/**
 * Chord release: the bound key going up, or any bound modifier going up.
 * Modifiers count because macOS suppresses letter keyups while ⌘ is held —
 * releasing ⌘ is often the only release event we ever see.
 */
export function isChordRelease(
  e: Pick<KeyboardEvent, "key">,
  bindings: KeyBinding[],
): boolean {
  const k = e.key.toLowerCase();
  return bindings.some(
    (b) =>
      k === b.key.toLowerCase() ||
      (!!b.ctrl && k === "control") ||
      (!!b.meta && k === "meta") ||
      (!!b.shift && k === "shift") ||
      (!!b.alt && k === "alt"),
  );
}
