import { Spinner } from "@/components/ui/spinner";
import { usePresence } from "@/lib/usePresence";
import { cn } from "@/lib/utils";
import { useShortcutLabel } from "@/modules/shortcuts/lib/useShortcutLabel";
import { Alert02Icon, Cancel01Icon, Tick02Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useRef, useState } from "react";
import { useComposer } from "../lib/composer";
import { hudPhaseFor, type VoiceHudPhase } from "../lib/voiceHud";
import { useChatStore } from "../store/chatStore";

/**
 * The headless-voice surface (ADR-017): a floating pill, bottom-center, that
 * narrates the whole voice flow — listening (RMS waveform), transcribing,
 * working, a brief done flash, transient errors. Mounted once at App-shell
 * level; renders nothing when voice is idle. pointer-events stay OFF except
 * on its buttons, so it never blocks whatever lives underneath.
 */

const DONE_FLASH_MS = 1500;
const ERROR_FADE_MS = 4000;

type HudDisplay = Exclude<VoiceHudPhase, "hidden"> | "done" | "error";

export function VoiceHud() {
  const c = useComposer();
  const agentStatus = useChatStore((s) => s.agentMeta.status);
  const shortcut = useShortcutLabel("ai.voiceInput");

  const livePhase = hudPhaseFor({
    captureState: c.voice.state,
    voiceTurnActive: c.voiceTurnActive,
    agentStatus,
  });

  // Done flash: bumps once per cleanly settled voice turn. A session re-arm
  // can start listening in the same beat — the live phase wins then.
  const [doneFlash, setDoneFlash] = useState(false);
  const doneSignal = c.voiceDoneSignal;
  useEffect(() => {
    if (doneSignal === 0) return;
    setDoneFlash(true);
    const t = window.setTimeout(() => setDoneFlash(false), DONE_FLASH_MS);
    return () => window.clearTimeout(t);
  }, [doneSignal]);

  // Errors fade here sooner than the composer clears them (~6s) — the pill is
  // ambient chrome, not a log.
  const error = c.voice.error;
  const [errorShown, setErrorShown] = useState(false);
  useEffect(() => {
    if (!error) {
      setErrorShown(false);
      return;
    }
    setErrorShown(true);
    const t = window.setTimeout(() => setErrorShown(false), ERROR_FADE_MS);
    return () => window.clearTimeout(t);
  }, [error]);

  const display: HudDisplay | "hidden" =
    livePhase !== "hidden"
      ? livePhase
      : error && errorShown
        ? "error"
        : doneFlash
          ? "done"
          : "hidden";

  const presence = usePresence(display !== "hidden", 180);
  // Keep the last visible content through the exit animation.
  const lastRef = useRef<HudDisplay>("listening");
  if (display !== "hidden") lastRef.current = display;
  const lastErrorRef = useRef("");
  if (error) lastErrorRef.current = error.message;
  if (!presence.mounted) return null;
  const shown: HudDisplay = display === "hidden" ? lastRef.current : display;

  const manualTake = c.voice.recording && c.voice.meta?.mode === "manual";
  const errorDestructive = error?.kind !== "no-speech";
  const dismissable =
    shown === "listening" || shown === "transcribing" || shown === "working";

  const label =
    shown === "listening"
      ? "Listening"
      : shown === "transcribing"
        ? "Transcribing…"
        : shown === "working"
          ? "Librarian is working…"
          : shown === "done"
            ? "Done"
            : error?.message || lastErrorRef.current;

  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-12 z-50 flex justify-center">
      <div
        role="status"
        aria-live="polite"
        data-state={presence.state}
        className={cn(
          "flex h-8 items-center gap-2.5 rounded-full border border-border/70 bg-card/95 pl-3.5 backdrop-blur",
          dismissable ? "pr-1.5" : "pr-3.5",
          "shadow-[0_1px_0_0_rgba(255,255,255,0.04)_inset,0_12px_32px_-12px_rgba(0,0,0,0.55),0_4px_12px_-6px_rgba(0,0,0,0.35)]",
          "duration-200 ease-out",
          "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:slide-in-from-bottom-2",
          "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:slide-out-to-bottom-2",
        )}
      >
        {shown === "listening" ? (
          <Waveform levelRef={c.voice.levelRef} />
        ) : shown === "transcribing" || shown === "working" ? (
          <Spinner className="size-3 text-muted-foreground" />
        ) : shown === "done" ? (
          <HugeiconsIcon
            icon={Tick02Icon}
            size={13}
            strokeWidth={2}
            className="text-primary"
          />
        ) : (
          <HugeiconsIcon
            icon={Alert02Icon}
            size={12}
            strokeWidth={1.75}
            className={
              errorDestructive ? "text-destructive" : "text-muted-foreground"
            }
          />
        )}

        <span
          className={cn(
            "max-w-72 truncate font-mono text-[11px]",
            shown === "error"
              ? errorDestructive
                ? "text-destructive"
                : "text-muted-foreground"
              : shown === "listening"
                ? "text-foreground"
                : "text-muted-foreground",
          )}
        >
          {label}
        </span>

        {shown === "listening" && manualTake && shortcut ? (
          <span className="font-mono text-[10px] text-muted-foreground/70">
            tap {shortcut} to send
          </span>
        ) : null}

        {dismissable ? (
          <button
            type="button"
            onClick={c.voiceHudDismiss}
            aria-label={shown === "working" ? "Stop" : "Discard take"}
            title={shown === "working" ? "Stop the run" : "Discard (Esc)"}
            className={cn(
              "pointer-events-auto flex size-5 items-center justify-center rounded-full",
              "text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
            )}
          >
            <HugeiconsIcon icon={Cancel01Icon} size={10} strokeWidth={2} />
          </button>
        ) : null}
      </div>
    </div>
  );
}

/**
 * 4-bar mini waveform driven by the live capture RMS. Polls the ref at 10Hz
 * (the analyser writes at the same rate) — re-renders only this leaf, never
 * the app.
 */
const BAR_SPREAD = [0.5, 0.85, 1, 0.68];

function Waveform({ levelRef }: { levelRef: React.RefObject<number> }) {
  const [level, setLevel] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => {
      setLevel(levelRef.current ?? 0);
    }, 100);
    return () => window.clearInterval(t);
  }, [levelRef]);
  // Quiet rooms idle around 0.001-0.005 RMS, speech runs 0.02-0.1+.
  const norm = Math.min(1, level / 0.09);
  return (
    <span aria-hidden className="flex h-3.5 items-center gap-[3px]">
      {BAR_SPREAD.map((m) => (
        <span
          key={m}
          className="w-[2.5px] rounded-full bg-primary transition-[height] duration-150 ease-out"
          style={{ height: `${Math.round(3 + m * norm * 11)}px` }}
        />
      ))}
    </span>
  );
}
