import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Backend-agnostic voice capture (the D7 refactor of useWhisperRecording):
 * owns getUserMedia, MIME negotiation, the idle → recording → transcribing
 * state machine, stream teardown, and silence auto-stop via a WebAudio
 * AnalyserNode. The transcriber is injected — see useWhisperRecording for the
 * default OpenAI whisper-1 backend; a local STT can slot in without touching
 * this file.
 */

export type VoiceCaptureState = "idle" | "recording" | "transcribing";

export type VoiceOrigin = "mic" | "hotkey" | "auto";

export type VoiceCaptureMeta = {
  /** What started the capture: mic button, PTT hotkey, or the hands-free loop. */
  origin: VoiceOrigin;
  /** Hands-free capture: the transcript is submitted, not just inserted. */
  autoSubmit: boolean;
};

export type VoiceCaptureErrorKind = "permission" | "transcribe" | "no-speech";

export type VoiceCaptureError = {
  kind: VoiceCaptureErrorKind;
  message: string;
};

export const DEFAULT_VOICE_META: VoiceCaptureMeta = {
  origin: "mic",
  autoSubmit: false,
};

// Silence auto-stop tuning. RMS of Float32 time-domain samples: a quiet room
// idles around 0.001–0.005, speech runs 0.02+.
export const SILENCE_MS_DEFAULT = 1400;
export const NO_SPEECH_MS_DEFAULT = 8000;
export const SILENCE_RMS_THRESHOLD = 0.01;
const RMS_POLL_MS = 100;

const MIME_CANDIDATES = [
  "audio/webm;codecs=opus",
  "audio/webm",
  "audio/ogg;codecs=opus",
  "audio/mp4",
];

/** First MediaRecorder-supported candidate. Injectable for tests. */
export function pickMime(
  isTypeSupported?: (mime: string) => boolean,
): string | undefined {
  const supported =
    isTypeSupported ??
    (typeof MediaRecorder !== "undefined"
      ? (m: string) => MediaRecorder.isTypeSupported(m)
      : null);
  if (!supported) return undefined;
  return MIME_CANDIDATES.find((m) => supported(m));
}

export type SilenceVerdict = "continue" | "stop" | "cancel";

export type SilenceDetector = {
  sample: (rms: number, nowMs: number) => SilenceVerdict;
};

/**
 * Pure state machine behind silence auto-stop (injectable clock for tests).
 * "stop"   → speech was heard, then silenceMs of quiet: stop + transcribe.
 * "cancel" → nothing above threshold for noSpeechMs: discard the take, never
 *            upload — the mic must not stay hot unattended.
 */
export function createSilenceDetector({
  silenceMs = SILENCE_MS_DEFAULT,
  noSpeechMs = NO_SPEECH_MS_DEFAULT,
  threshold = SILENCE_RMS_THRESHOLD,
}: {
  silenceMs?: number;
  noSpeechMs?: number;
  threshold?: number;
} = {}): SilenceDetector {
  let firstAt: number | null = null;
  let lastSpeechAt: number | null = null;
  return {
    sample(rms, now) {
      if (firstAt === null) firstAt = now;
      if (rms >= threshold) {
        lastSpeechAt = now;
        return "continue";
      }
      if (lastSpeechAt === null) {
        return now - firstAt >= noSpeechMs ? "cancel" : "continue";
      }
      return now - lastSpeechAt >= silenceMs ? "stop" : "continue";
    },
  };
}

export function rmsOf(samples: Float32Array): number {
  if (samples.length === 0) return 0;
  let sum = 0;
  for (let i = 0; i < samples.length; i++) sum += samples[i] * samples[i];
  return Math.sqrt(sum / samples.length);
}

function captureError(e: unknown): VoiceCaptureError {
  const name = e instanceof DOMException ? e.name : "";
  if (name === "NotFoundError" || name === "OverconstrainedError") {
    return { kind: "permission", message: "No microphone found." };
  }
  return {
    kind: "permission",
    message:
      "Microphone blocked — allow mic access for Koden in system settings.",
  };
}

export function useVoiceCapture({
  transcribe,
  onResult,
  silenceMs = SILENCE_MS_DEFAULT,
  noSpeechMs = NO_SPEECH_MS_DEFAULT,
}: {
  transcribe: (blob: Blob) => Promise<string>;
  onResult: (text: string, meta: VoiceCaptureMeta) => void;
  silenceMs?: number;
  noSpeechMs?: number;
}) {
  const [state, setState] = useState<VoiceCaptureState>("idle");
  const [meta, setMeta] = useState<VoiceCaptureMeta | null>(null);
  const [error, setError] = useState<VoiceCaptureError | null>(null);

  const stateRef = useRef(state);
  stateRef.current = state;

  const recRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const streamRef = useRef<MediaStream | null>(null);
  const audioRef = useRef<{ ctx: AudioContext; timer: number } | null>(null);
  const cancelledRef = useRef(false);
  // Per-attempt epoch: state stays "idle" until getUserMedia resolves, so two
  // rapid start() calls both pass the idle guard and both await the mic. The
  // superseded attempt must stop ITS OWN stream — otherwise its live track
  // leaks (OS mic-in-use indicator stuck on) and its recorder keeps feeding
  // the shared chunks. cancelledRef alone can't express this: a new start()
  // resets it, reviving the older pending attempt.
  const attemptRef = useRef(0);

  // rec.onstop closes over the render that called start(); route the
  // callbacks through a ref so results always use the freshest closures
  // (composer value, submit, busy state).
  const latest = useRef({ transcribe, onResult });
  latest.current = { transcribe, onResult };

  const supported =
    typeof navigator !== "undefined" &&
    !!navigator.mediaDevices?.getUserMedia &&
    typeof MediaRecorder !== "undefined";

  const teardownAudio = useCallback(() => {
    const a = audioRef.current;
    if (a) {
      window.clearInterval(a.timer);
      void a.ctx.close().catch(() => undefined);
      audioRef.current = null;
    }
    for (const t of streamRef.current?.getTracks() ?? []) t.stop();
    streamRef.current = null;
  }, []);

  /** Stop + transcribe the take. */
  const stop = useCallback(() => {
    const rec = recRef.current;
    if (rec && rec.state !== "inactive") rec.stop();
  }, []);

  /**
   * Discard the take — nothing is uploaded. During transcribing the pending
   * result is suppressed instead (a new start is blocked until it settles).
   */
  const cancel = useCallback(() => {
    cancelledRef.current = true;
    const rec = recRef.current;
    if (rec && rec.state !== "inactive") rec.stop();
  }, []);

  const clearError = useCallback(() => setError(null), []);

  const start = useCallback(
    async (metaIn?: Partial<VoiceCaptureMeta>): Promise<boolean> => {
      if (!supported || stateRef.current !== "idle") return false;
      setError(null);
      cancelledRef.current = false;
      const attempt = ++attemptRef.current;
      const captureMeta: VoiceCaptureMeta = {
        ...DEFAULT_VOICE_META,
        ...metaIn,
      };
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: true,
        });
        if (cancelledRef.current || attempt !== attemptRef.current) {
          // cancel() raced the permission prompt, or a newer start()
          // superseded this attempt while the mic prompt was open.
          for (const t of stream.getTracks()) t.stop();
          return false;
        }
        streamRef.current = stream;
        const mimeType = pickMime();
        const rec = new MediaRecorder(
          stream,
          mimeType ? { mimeType } : undefined,
        );
        chunksRef.current = [];
        rec.ondataavailable = (e) => {
          if (e.data.size > 0) chunksRef.current.push(e.data);
        };
        rec.onstop = async () => {
          const blob = new Blob(chunksRef.current, {
            type: rec.mimeType || "audio/webm",
          });
          chunksRef.current = [];
          teardownAudio();
          if (cancelledRef.current || blob.size === 0) {
            setMeta(null);
            setState("idle");
            return;
          }
          setState("transcribing");
          try {
            const text = (await latest.current.transcribe(blob)).trim();
            if (!cancelledRef.current) {
              if (text) latest.current.onResult(text, captureMeta);
              else {
                setError({
                  kind: "no-speech",
                  message: "Nothing heard — try again.",
                });
              }
            }
          } catch (e) {
            console.error("voice.transcribe", e);
            setError({
              kind: "transcribe",
              message: e instanceof Error ? e.message : "Transcription failed.",
            });
          } finally {
            setMeta(null);
            setState("idle");
          }
        };
        recRef.current = rec;

        // Silence auto-stop: RMS below threshold for silenceMs ends the take;
        // noSpeechMs with no speech at all discards it.
        try {
          const ctx = new AudioContext();
          void ctx.resume().catch(() => undefined);
          const source = ctx.createMediaStreamSource(stream);
          const analyser = ctx.createAnalyser();
          analyser.fftSize = 1024;
          source.connect(analyser);
          const buf = new Float32Array(analyser.fftSize);
          const detector = createSilenceDetector({ silenceMs, noSpeechMs });
          const timer = window.setInterval(() => {
            analyser.getFloatTimeDomainData(buf);
            const verdict = detector.sample(rmsOf(buf), performance.now());
            if (verdict === "continue") return;
            if (verdict === "cancel") {
              cancelledRef.current = true;
              setError({ kind: "no-speech", message: "No speech detected." });
            }
            stop();
          }, RMS_POLL_MS);
          audioRef.current = { ctx, timer };
        } catch (e) {
          // No analyser (unlikely): capture still works, stop stays manual.
          console.error("voice.analyser", e);
        }

        rec.start();
        setMeta(captureMeta);
        setState("recording");
        return true;
      } catch (e) {
        console.error("voice.getUserMedia", e);
        teardownAudio();
        setMeta(null);
        setState("idle");
        setError(captureError(e));
        return false;
      }
    },
    [supported, silenceMs, noSpeechMs, stop, teardownAudio],
  );

  useEffect(() => {
    return () => {
      cancelledRef.current = true;
      const rec = recRef.current;
      if (rec && rec.state !== "inactive") rec.stop();
      teardownAudio();
    };
  }, [teardownAudio]);

  return {
    state,
    recording: state === "recording",
    transcribing: state === "transcribing",
    supported,
    /** Meta of the capture in flight (null when idle). */
    meta,
    error,
    start,
    stop,
    cancel,
    clearError,
  };
}
