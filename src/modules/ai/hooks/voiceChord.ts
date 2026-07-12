import type { KeyBinding } from "@/modules/shortcuts/shortcuts";

/**
 * Pure chord helpers for the voice hotkey (keydown/keyup semantics). Kept
 * free of runtime imports so tests run without the Tauri platform shim.
 *
 * Gesture model (Wispr Flow style): a TAP starts one continuous MANUAL take —
 * the user may pause indefinitely, nothing auto-stops it — and the next press
 * stops + transcribes + submits. A HOLD is classic push-to-talk: the release
 * stops the take. The always-on voice SESSION is the header mic's job only;
 * the chord never toggles it.
 */

/** A hold past this is push-to-talk; a quicker release is a tap. */
export const HOLD_TO_TALK_MS = 350;

/**
 * Release classification: a hold past the threshold is one push-to-talk take
 * (release stops + transcribes); a quick tap leaves its manual take recording
 * until the next press stops it.
 */
export function chordReleaseKind(
  heldMs: number,
  holdToTalkMs: number = HOLD_TO_TALK_MS,
): "tap" | "hold" {
  return heldMs >= holdToTalkMs ? "hold" : "tap";
}

export type ChordPressAction = "start-take" | "stop-capture" | "ignore";

/**
 * What a chord PRESS does given live capture state:
 * - idle          → start one MANUAL take (origin "hotkey", auto-submit)
 * - recording     → stop + transcribe + submit. This is both the second tap
 *                   of a manual take AND a tap during a session-loop capture
 *                   (the session re-arms after the turn, by design).
 * - transcribing / disabled → ignore (the chord is still swallowed).
 */
export function chordPressAction(s: {
  enabled: boolean;
  recording: boolean;
  transcribing: boolean;
}): ChordPressAction {
  if (!s.enabled || s.transcribing) return "ignore";
  return s.recording ? "stop-capture" : "start-take";
}

export type ChordReleaseAction = "stop-capture" | "none";

/**
 * What a chord RELEASE does. Only a press that STARTED a capture can act on
 * release; a hold's release ends its push-to-talk take (if still recording),
 * while a tap's release leaves the manual take running.
 */
export function chordReleaseAction(s: {
  /** This press began the capture (vs a press that stopped one). */
  started: boolean;
  heldMs: number;
  recording: boolean;
  holdToTalkMs?: number;
}): ChordReleaseAction {
  if (!s.started) return "none";
  const kind = chordReleaseKind(s.heldMs, s.holdToTalkMs ?? HOLD_TO_TALK_MS);
  if (kind === "hold") return s.recording ? "stop-capture" : "none";
  // Tap: the manual take keeps recording until the next press stops it.
  return "none";
}

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
