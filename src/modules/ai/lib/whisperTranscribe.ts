import { useChatStore } from "../store/chatStore";

export const VOICE_NEEDS_KEY_MESSAGE =
  "Voice needs an OpenAI key (Settings → Models) — local STT is on the roadmap";

/**
 * Default voice backend: OpenAI whisper-1 with the user's BYOK key (OS
 * keyring → chatStore). Injected into useVoiceCapture so a local STT can
 * replace it without touching the capture machinery.
 *
 * NOTE: the audio leaves the machine — capture stays strictly push-to-talk,
 * and callers must never invoke this without a key (we throw, not silently
 * skip, as defense in depth).
 */
export async function whisperTranscribe(blob: Blob): Promise<string> {
  const apiKey = useChatStore.getState().apiKeys.openai;
  if (!apiKey) throw new Error(VOICE_NEEDS_KEY_MESSAGE);
  const [{ createOpenAI }, { experimental_transcribe: transcribe }] =
    await Promise.all([import("@ai-sdk/openai"), import("ai")]);
  const openai = createOpenAI({ apiKey });
  const audio = new Uint8Array(await blob.arrayBuffer());
  const { text } = await transcribe({
    model: openai.transcription("whisper-1"),
    audio,
  });
  return text;
}
