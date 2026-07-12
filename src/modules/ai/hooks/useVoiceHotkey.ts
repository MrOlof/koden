import { usePreferencesStore } from "@/modules/settings/preferences";
import { matchBinding, SHORTCUTS } from "@/modules/shortcuts/shortcuts";
import { useEffect, useRef } from "react";
import { chordReleaseKind, HOLD_TO_TALK_MS, isChordRelease } from "./voiceChord";

const DEFAULT_BINDINGS =
  SHORTCUTS.find((s) => s.id === "ai.voiceInput")?.defaultBindings ?? [];

/**
 * Hold-or-toggle push-to-talk chord ("ai.voiceInput" in the shortcuts
 * registry, user-rebindable). useGlobalShortcuts is keydown-only, so this
 * hook owns its own keydown+keyup capture listeners:
 *
 * - press while idle      → start capture
 * - release after a hold  → stop + transcribe (push-to-talk, one take)
 * - quick tap             → onTap (voice session ON); stays listening and the
 *                           next press stops + transcribes, as before
 * - window blur           → stop (never keep the mic hot in the background)
 *
 * The chord is always swallowed (capture + preventDefault) so a held key
 * never leaks into the focused terminal — Ctrl+M variants would land as CR
 * in a pty.
 */
export function useVoiceHotkey({
  enabled,
  recording,
  transcribing,
  onStart,
  onStop,
  onTap,
}: {
  enabled: boolean;
  recording: boolean;
  transcribing: boolean;
  onStart: () => void;
  onStop: () => void;
  /** Quick release of a press that STARTED a capture (tap, not hold). */
  onTap?: () => void;
}) {
  const latest = useRef({
    enabled,
    recording,
    transcribing,
    onStart,
    onStop,
    onTap,
  });
  latest.current = { enabled, recording, transcribing, onStart, onStop, onTap };
  const userShortcuts = usePreferencesStore((s) => s.shortcuts);
  // null = chord not held; started = this press began the capture.
  const pressRef = useRef<{ at: number; started: boolean } | null>(null);

  useEffect(() => {
    const bindings = userShortcuts["ai.voiceInput"] ?? DEFAULT_BINDINGS;
    if (bindings.length === 0) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (!bindings.some((b) => matchBinding(e, b))) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.repeat || pressRef.current) return;
      const s = latest.current;
      if (!s.enabled || s.transcribing) return;
      if (s.recording) {
        // Second press while listening = toggle off (stop + transcribe).
        pressRef.current = { at: performance.now(), started: false };
        s.onStop();
        return;
      }
      pressRef.current = { at: performance.now(), started: true };
      s.onStart();
    };

    const onKeyUp = (e: KeyboardEvent) => {
      const press = pressRef.current;
      if (!press || !isChordRelease(e, bindings)) return;
      pressRef.current = null;
      if (!press.started) return;
      const held = performance.now() - press.at;
      if (chordReleaseKind(held, HOLD_TO_TALK_MS) === "hold") {
        if (latest.current.recording) latest.current.onStop();
        return;
      }
      latest.current.onTap?.();
    };

    const onBlur = () => {
      pressRef.current = null;
      if (latest.current.recording) latest.current.onStop();
    };

    window.addEventListener("keydown", onKeyDown, { capture: true });
    window.addEventListener("keyup", onKeyUp, { capture: true });
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown, { capture: true });
      window.removeEventListener("keyup", onKeyUp, { capture: true });
      window.removeEventListener("blur", onBlur);
    };
  }, [userShortcuts]);
}
