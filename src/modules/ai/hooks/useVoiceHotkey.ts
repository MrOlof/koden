import { usePreferencesStore } from "@/modules/settings/preferences";
import { matchBinding, SHORTCUTS } from "@/modules/shortcuts/shortcuts";
import { useEffect, useRef } from "react";
import {
  chordPressAction,
  chordReleaseAction,
  isChordRelease,
} from "./voiceChord";

const DEFAULT_BINDINGS =
  SHORTCUTS.find((s) => s.id === "ai.voiceInput")?.defaultBindings ?? [];

/**
 * Hold-or-tap push-to-talk chord ("ai.voiceInput" in the shortcuts registry,
 * user-rebindable). useGlobalShortcuts is keydown-only, so this hook owns its
 * own keydown+keyup capture listeners:
 *
 * - press while idle      → start capture (onStart decides the mode — the
 *                           composer starts hotkey captures as MANUAL takes:
 *                           no silence auto-stop once speech registered)
 * - release after a hold  → stop + transcribe (push-to-talk, one take)
 * - quick tap             → the take keeps recording, Wispr Flow style — the
 *                           user may pause indefinitely; the next press stops
 *                           + transcribes + submits (also true when the live
 *                           capture belongs to the voice-session loop, which
 *                           then re-arms as designed)
 * - window blur           → stop (never keep the mic hot in the background)
 *
 * The always-on voice SESSION is the header mic button's job only — the
 * chord never toggles it.
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
}: {
  enabled: boolean;
  recording: boolean;
  transcribing: boolean;
  onStart: () => void;
  onStop: () => void;
}) {
  const latest = useRef({
    enabled,
    recording,
    transcribing,
    onStart,
    onStop,
  });
  latest.current = { enabled, recording, transcribing, onStart, onStop };
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
      const action = chordPressAction(s);
      if (action === "ignore") return;
      pressRef.current = {
        at: performance.now(),
        started: action === "start-take",
      };
      if (action === "stop-capture") s.onStop();
      else s.onStart();
    };

    const onKeyUp = (e: KeyboardEvent) => {
      const press = pressRef.current;
      if (!press || !isChordRelease(e, bindings)) return;
      pressRef.current = null;
      const action = chordReleaseAction({
        started: press.started,
        heldMs: performance.now() - press.at,
        recording: latest.current.recording,
      });
      if (action === "stop-capture") latest.current.onStop();
      // Tap release: nothing to do — the manual take keeps recording.
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
