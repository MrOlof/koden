import type { AgentRunStatus } from "../store/chatStore";
import type { VoiceCaptureState } from "../hooks/useVoiceCapture";

/**
 * Pure seams for the headless-voice HUD (ADR-017): the floating pill is the
 * ONLY voice surface — the Librarian window never opens for voice (approvals
 * excepted). Type-only imports so tests run without the Tauri platform shim
 * (voiceChord.ts pattern).
 */

export type VoiceHudPhase = "hidden" | "listening" | "transcribing" | "working";

/**
 * Live phase from composer state. "done" and "error" are HUD-local overlays
 * (timers on voiceDoneSignal / voice.error), not live phases. "working" spans
 * the whole voice turn including awaiting-approval — the window auto-opens for
 * the approval, but the turn is still running.
 */
export function hudPhaseFor(i: {
  captureState: VoiceCaptureState;
  voiceTurnActive: boolean;
  agentStatus: AgentRunStatus;
}): VoiceHudPhase {
  if (i.captureState === "recording") return "listening";
  if (i.captureState === "transcribing") return "transcribing";
  if (
    i.voiceTurnActive &&
    i.agentStatus !== "idle" &&
    i.agentStatus !== "error"
  )
    return "working";
  return "hidden";
}

export const VOICE_REPLY_PREVIEW_MAX = 140;

type ReplyPart = { type: string; text?: string };

/**
 * Reply-toast gate for a completed VOICE turn: substantive assistant prose
 * (text parts, whitespace-collapsed, ~140 chars) gets ONE toast; action-only
 * turns (tools ran, no prose) return null — the activity/approval system
 * already narrates those.
 */
export function voiceReplyPreview(
  message: { role: string; parts: ReplyPart[] } | null | undefined,
  max: number = VOICE_REPLY_PREVIEW_MAX,
): string | null {
  if (message?.role !== "assistant") return null;
  let flat = "";
  for (const p of message.parts) {
    if (p.type !== "text" || typeof p.text !== "string") continue;
    flat += `${p.text} `;
  }
  flat = flat.replace(/\s+/g, " ").trim();
  if (!flat) return null;
  if (flat.length <= max) return flat;
  return `${flat.slice(0, Math.max(0, max - 1)).trimEnd()}…`;
}
