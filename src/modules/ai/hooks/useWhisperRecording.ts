import { useCallback } from "react";
import { whisperTranscribe } from "../lib/whisperTranscribe";
import { useChatStore } from "../store/chatStore";
import { useVoiceCapture, type VoiceCaptureMeta } from "./useVoiceCapture";

/**
 * Whisper-backed voice recording — thin wrapper over the backend-agnostic
 * useVoiceCapture (D7) with the OpenAI whisper-1 transcriber injected. Keeps
 * the original return contract ({state, recording, transcribing, start, stop,
 * supported, hasKey}) and adds cancel/error/meta for the hands-free flow.
 */
export function useWhisperRecording({
  onResult,
}: {
  onResult: (text: string, meta: VoiceCaptureMeta) => void;
}) {
  const hasKey = useChatStore((s) => !!s.apiKeys.openai);
  const capture = useVoiceCapture({ transcribe: whisperTranscribe, onResult });
  const { start: startCapture } = capture;
  const start = useCallback(
    async (meta?: Partial<VoiceCaptureMeta>): Promise<boolean> => {
      // Never open the mic (or call OpenAI) without a key — the UI disables
      // the button; this guards the hotkey + continuous paths.
      if (!hasKey) return false;
      return startCapture(meta);
    },
    [hasKey, startCapture],
  );
  return { ...capture, start, hasKey };
}
