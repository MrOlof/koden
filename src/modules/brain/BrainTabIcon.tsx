import { Brain02Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useBrainStatus } from "./lib/useBrainStatus";

/**
 * The Koden Brain tab icon, alive: a brain glyph with a status dot
 * (green = ready, amber = indexing, red = degraded, muted = connecting) and a
 * gentle breathe on the glyph + a pulsing dot WHILE the worker is indexing.
 * Both animations are `motion-safe` / disabled under `prefers-reduced-motion`.
 */
export function BrainTabIcon() {
  const status = useBrainStatus();
  const state = status?.state ?? null;
  const indexing = state === "warming";

  const dotColor =
    state === "ready"
      ? "bg-emerald-500"
      : state === "degraded"
        ? "bg-red-500"
        : state === "warming"
          ? "bg-amber-500"
          : "bg-muted-foreground/40";

  const title =
    state === "ready"
      ? "Brain: ready"
      : state === "warming"
        ? `Brain: indexing… ${status?.state === "warming" ? status.pct : 0}%`
        : state === "degraded"
          ? `Brain: degraded — ${status?.state === "degraded" ? status.reason : ""}`
          : "Brain: connecting…";

  return (
    <span className="relative inline-flex shrink-0" title={title}>
      <HugeiconsIcon
        icon={Brain02Icon}
        size={14}
        strokeWidth={2}
        className={cn("shrink-0", indexing && "koden-breathe")}
      />
      <span
        className={cn(
          "absolute -right-0.5 -bottom-0.5 size-1.5 rounded-full ring-1 ring-background",
          dotColor,
          indexing && "motion-safe:animate-pulse",
        )}
      />
    </span>
  );
}

/**
 * Top-bar entry point for the Brain — a ghost icon button (matching the bell /
 * command-palette buttons) carrying the same live status icon. Lives in the header
 * so the brain's state is always visible, not buried in a tab.
 */
export function BrainHeaderButton({ onClick }: { onClick: () => void }) {
  return (
    <Button
      size="icon-sm"
      variant="ghost"
      onClick={onClick}
      aria-label="Open Koden Brain"
      className="shrink-0 rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
    >
      <BrainTabIcon />
    </Button>
  );
}
