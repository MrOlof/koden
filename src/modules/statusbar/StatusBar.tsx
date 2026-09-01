import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useUsageStore } from "@/modules/agents/store/usageStore";
import { useChatStore } from "@/modules/ai";
import { AgentStatusPill } from "@/modules/ai/components/AgentStatusPill";
import { AiStatusBarControls } from "@/modules/ai/components/AiStatusBarControls";
import { SyncStatusSegment } from "@/modules/sync";
import type { WorkspaceEnv } from "@/modules/workspace";
import { IncognitoIcon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { BrainActivitySegment } from "./BrainActivitySegment";
import { CwdBreadcrumb } from "./CwdBreadcrumb";
import { WorkspaceEnvSelector } from "./WorkspaceEnvSelector";

type Props = {
  cwd: string | null;
  filePath?: string | null;
  home: string | null;
  onCd: (path: string) => void;
  onWorkspaceChange: (env: WorkspaceEnv) => void;
  onOpenMini: () => void;
  /** Only rendered when the AI panel is open and a key is loaded. */
  hasComposer: boolean;
  privateActive: boolean;
};

export function StatusBar({
  cwd,
  filePath,
  home,
  onCd,
  onWorkspaceChange,
  onOpenMini,
  hasComposer,
  privateActive,
}: Props) {
  const panelOpen = useChatStore((s) => s.panelOpen);
  const pauseActive = useUsageStore((s) => s.pauseActive);
  const resetEpochMs = useUsageStore((s) => s.latest?.resetEpochMs ?? null);
  const resetLabel =
    resetEpochMs !== null
      ? new Date(resetEpochMs).toLocaleTimeString([], {
          hour: "2-digit",
          minute: "2-digit",
        })
      : null;

  return (
    <footer className="flex h-8 shrink-0 items-center justify-between gap-3 border-t border-border/60 bg-card/60 px-3 font-mono text-[11px] tracking-[0.01em]">
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <WorkspaceEnvSelector onSelect={onWorkspaceChange} />
        <CwdBreadcrumb cwd={cwd} filePath={filePath} home={home} onCd={onCd} />
        {privateActive ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="flex shrink-0 cursor-default items-center gap-1 rounded-full bg-amber-500/15 px-2 py-0.5 text-[10.5px] font-medium text-amber-700 dark:text-amber-400">
                <HugeiconsIcon icon={IncognitoIcon} size={11} strokeWidth={2} />
                <span>Private: hidden from AI</span>
              </span>
            </TooltipTrigger>
            <TooltipContent
              side="top"
              className="max-w-64 text-[11px] leading-relaxed"
            >
              AI can't see this terminal's output. Use it for secrets, SSH, or
              anything you don't want sent to the model.
            </TooltipContent>
          </Tooltip>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        {/* ADR-020: ambient Librarian segment — always on, never a popup. */}
        <BrainActivitySegment />
        {/* ADR-023: cross-machine sync state; hidden while the pref is off. */}
        <SyncStatusSegment />
        {pauseActive ? (
          <span
            title={
              resetLabel
                ? `Usage limit reached. New agents resume around ${resetLabel}.`
                : "Usage limit reached. New agents resume when usage drops."
            }
            className="flex shrink-0 cursor-default items-center gap-1 rounded-full bg-foreground/[0.06] px-2 py-0.5 text-[10.5px] font-medium text-muted-foreground"
          >
            <span
              aria-hidden
              className="size-1.5 rounded-full bg-[color:var(--terminal-ansi-yellow)]"
            />
            <span>Usage guard: paused</span>
          </span>
        ) : null}
        <AgentStatusPill onClick={onOpenMini} />
        {/* The launcher moved to the header ("Koden" button); the status bar only
            shows the inline controls while the panel is open. */}
        {panelOpen && hasComposer ? <AiStatusBarControls /> : null}
      </div>
    </footer>
  );
}
