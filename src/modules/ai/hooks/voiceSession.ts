import type { AgentRunStatus } from "../store/chatStore";
import type { VoiceCaptureState } from "./useVoiceCapture";

/**
 * Pure decision seams for the always-on VOICE SESSION — the session-scoped
 * listen loop (header mic toggle ONLY; the hotkey tap is a one-take gesture,
 * see voiceChord.ts) — and the legacy hands-free re-arm. Type-only imports,
 * no runtime deps, so tests run without the Tauri platform shim
 * (voiceChord.ts pattern).
 *
 * The session and the hands-free pref are ORTHOGONAL (ADR-017 addendum):
 * the session governs only when the mic LISTENS; `handsFreeMode` governs only
 * terminal-submit approvals. Listen-always ≠ approve-always.
 */

export type RearmInput = {
  prevStatus: AgentRunStatus;
  status: AgentRunStatus;
  /** Voice session toggled on (composer state, never persisted). */
  sessionActive: boolean;
  /** Legacy lane: the hands-free approvals pref. */
  handsFreeArmed: boolean;
  miniOpen: boolean;
  /** Hard-stop latch (Esc / mic click) — pauses the LEGACY loop only. */
  suspended: boolean;
  captureState: VoiceCaptureState;
  supported: boolean;
  hasKey: boolean;
  /** Keyboard draft in the composer — stay out of the way. */
  hasDraft: boolean;
  /** Never arm the mic in a background window. */
  windowFocused: boolean;
};

/**
 * Post-turn re-arm, evaluated once per assistant-turn completion. The session
 * lane re-arms regardless of the hands-free pref (and ignores the suspend
 * latch — Esc during a session discards the take, not the loop); the legacy
 * lane preserves the prior armed + window-open + not-suspended behavior
 * exactly, for users who armed hands-free without the session toggle.
 */
export function shouldRearmVoice(i: RearmInput): boolean {
  if (i.status !== "idle") return false;
  if (i.prevStatus !== "thinking" && i.prevStatus !== "streaming") return false;
  const sessionLoop = i.sessionActive && i.miniOpen;
  const legacyLoop = i.handsFreeArmed && i.miniOpen && !i.suspended;
  if (!sessionLoop && !legacyLoop) return false;
  if (!i.supported || !i.hasKey) return false;
  if (i.captureState !== "idle") return false;
  if (i.hasDraft) return false;
  return i.windowFocused;
}

export type EscAction = "cancel-capture" | "end-session" | "none";

/**
 * Unified Esc tiering: Esc while a capture is live discards the TAKE; the
 * next Esc (or Esc while not capturing) ends the SESSION; with neither, Esc
 * falls through to the Librarian window's own close handler.
 */
export function escActionFor(i: {
  capturing: boolean;
  sessionActive: boolean;
}): EscAction {
  if (i.capturing) return "cancel-capture";
  if (i.sessionActive) return "end-session";
  return "none";
}
