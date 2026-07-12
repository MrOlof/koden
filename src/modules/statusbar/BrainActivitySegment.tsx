import { cn } from "@/lib/utils";
import { useBrainActivityStore, useBrainStatus } from "@/modules/brain";
import { useEffect, useState } from "react";

const FLASH_MS = 2500;

function ago(at: number): string {
  const s = Math.floor((Date.now() - at) / 1000);
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

const VERB: Record<string, string> = {
  reflected: "reflect",
  applied: "applied",
  reverted: "revert",
  registered: "registered",
};

/**
 * Ambient brain segment (ADR-020): a small mono dot+label in the status bar.
 * Muted when idle, pulses while the index warms, flashes primary briefly after
 * a Librarian activity event. ALWAYS on — ambient chrome, not gated by the
 * memoryNotifications pref. Hover = the last activity summary + index state.
 * Svart tokens only (primary / muted-foreground / --terminal-ansi-yellow).
 */
export function BrainActivitySegment() {
  const status = useBrainStatus();
  const last = useBrainActivityStore((s) => s.last);
  const [flash, setFlash] = useState(false);

  useEffect(() => {
    if (!last) return;
    setFlash(true);
    const id = window.setTimeout(() => setFlash(false), FLASH_MS);
    return () => window.clearTimeout(id);
  }, [last]);

  const warming = status?.state === "warming";
  const degraded = status?.state === "degraded";

  const stateLabel = !status
    ? "connecting…"
    : status.state === "warming"
      ? `indexing… ${status.pct}%`
      : status.state === "degraded"
        ? `degraded: ${status.reason}`
        : "ready";
  const lastLabel = last
    ? `${VERB[last.event.kind] ?? last.event.kind} ${ago(last.at)}` +
      (last.event.spent_usd != null
        ? ` · $${last.event.spent_usd.toFixed(4)}`
        : "") +
      (last.event.count > 0 && last.event.kind !== "registered"
        ? ` · ${last.event.count} ${
            last.event.kind === "reflected" ? "proposal(s)" : "applied"
          }`
        : "")
    : null;
  const title = `Koden Brain — ${stateLabel}${lastLabel ? ` · ${lastLabel}` : ""}`;

  return (
    <span
      title={title}
      className="flex shrink-0 cursor-default items-center gap-1.5 px-1 font-mono text-[10.5px] text-muted-foreground"
    >
      <span
        aria-hidden
        className={cn(
          "size-1.5 rounded-full",
          flash
            ? "bg-primary"
            : warming
              ? "animate-pulse bg-[color:var(--terminal-ansi-yellow)]"
              : degraded
                ? "bg-[color:var(--terminal-ansi-yellow)]"
                : "bg-muted-foreground/40",
        )}
      />
      <span>brain</span>
    </span>
  );
}
