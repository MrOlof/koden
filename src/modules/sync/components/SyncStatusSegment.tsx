import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { syncNow } from "../lib/engine";
import { useSyncStore } from "../lib/syncStore";

function ago(at: number): string {
  const s = Math.floor((Date.now() - at) / 1000);
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

/**
 * Statusbar sync segment (ADR-023): hidden entirely while the pref is off,
 * otherwise a dot + "sync" with the last-synced time in the tooltip. Click =
 * sync now. Same visual idiom as BrainActivitySegment.
 */
export function SyncStatusSegment() {
  const enabled = usePreferencesStore((s) => s.syncEnabled);
  const host = usePreferencesStore((s) => s.syncHost);
  const status = useSyncStore((s) => s.status);
  const lastSyncAt = useSyncStore((s) => s.lastSyncAt);
  const lastError = useSyncStore((s) => s.lastError);

  if (!enabled || status === "disabled") return null;

  const stateLabel =
    status === "syncing"
      ? "syncing…"
      : status === "offline"
        ? `offline${lastError ? `: ${lastError}` : ""}`
        : status === "error"
          ? (lastError ?? "error")
          : lastSyncAt
            ? `synced ${ago(lastSyncAt)}`
            : "waiting for first sync";
  const title = `Workspace sync — ${host} — ${stateLabel} · click to sync now`;

  return (
    <button
      type="button"
      title={title}
      onClick={() => syncNow()}
      className="flex shrink-0 cursor-pointer items-center gap-1.5 px-1 font-mono text-[10.5px] text-muted-foreground"
    >
      <span
        aria-hidden
        className={cn(
          "size-1.5 rounded-full",
          status === "syncing"
            ? "animate-pulse bg-primary"
            : status === "offline" || status === "error"
              ? "bg-[color:var(--terminal-ansi-yellow)]"
              : "bg-primary/60",
        )}
      />
      <span>sync</span>
    </button>
  );
}
