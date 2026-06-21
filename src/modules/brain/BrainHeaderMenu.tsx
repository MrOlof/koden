import {
  HierarchySquare01Icon,
  Search01Icon,
  Settings01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { BrainTabIcon } from "./BrainTabIcon";
import {
  brainBudgetStatus,
  brainIndexStatus,
  type BrainStatusReport,
} from "./lib/bindings";

const POLL_MS = 2000;

function stateLabel(report: BrainStatusReport | null): string {
  if (!report) return "Connecting…";
  switch (report.status.state) {
    case "warming":
      return `Indexing… ${report.status.pct}%`;
    case "ready":
      return "Ready";
    case "degraded":
      return "Degraded";
  }
}

type Props = {
  onOpenBrain: () => void;
  onOpenBrainMap: () => void;
  onOpenSettings: () => void;
};

/**
 * Top-bar Brain control center: a dropdown (like the notification bell) showing live
 * status — indexing %, file/project counts, and reflect spend ONLY when a paid
 * ceiling/spend exists (local models cost nothing, so the cost line stays hidden) —
 * plus entries for the Brain Map, the full Brain view, and Settings.
 */
export function BrainHeaderMenu({ onOpenBrain, onOpenBrainMap, onOpenSettings }: Props) {
  const [open, setOpen] = useState(false);
  const [report, setReport] = useState<BrainStatusReport | null>(null);
  const [budget, setBudget] = useState<[number, number] | null>(null);

  // Fetch + poll only while the menu is open (cheap, and avoids a background poll
  // when nobody's looking — the trigger's own live dot covers the always-on cue).
  useEffect(() => {
    if (!open) return;
    let alive = true;
    const load = () => {
      brainIndexStatus()
        .then((r) => alive && setReport(r))
        .catch(() => {});
      brainBudgetStatus()
        .then((b) => alive && setBudget(b))
        .catch(() => {});
    };
    load();
    const id = setInterval(load, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [open]);

  const files = report?.projects.reduce((acc, p) => acc + p.files, 0) ?? 0;
  const projectCount = report?.projects.length ?? 0;
  const [ceiling, spent] = budget ?? [0, 0];
  const showCost = ceiling > 0 || spent > 0;

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Koden Brain"
          className="shrink-0 rounded-md text-muted-foreground hover:bg-accent hover:text-foreground data-[state=open]:bg-accent data-[state=open]:text-foreground"
        >
          <BrainTabIcon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" sideOffset={6} className="w-64">
        <div className="px-2 py-1.5">
          <div className="flex items-center gap-2">
            <span className="text-sm font-semibold">Koden Brain</span>
            <span className="ml-auto text-[11px] text-muted-foreground">{stateLabel(report)}</span>
          </div>
          <div className="mt-1.5 flex flex-col gap-0.5 text-[11px] text-muted-foreground tabular-nums">
            <div>
              {files.toLocaleString()} files · {projectCount} project{projectCount === 1 ? "" : "s"}
            </div>
            {showCost ? (
              <div>
                Reflect spend ${spent.toFixed(4)} / ${ceiling.toFixed(2)}
              </div>
            ) : (
              <div>Reflect off · local search is free</div>
            )}
          </div>
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={onOpenBrain} className="gap-2">
          <HugeiconsIcon icon={Search01Icon} size={15} strokeWidth={1.75} />
          Brain Search
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onOpenBrainMap} className="gap-2">
          <HugeiconsIcon icon={HierarchySquare01Icon} size={15} strokeWidth={1.75} />
          Brain Map
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onOpenSettings} className="gap-2">
          <HugeiconsIcon icon={Settings01Icon} size={15} strokeWidth={1.75} />
          Settings
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
