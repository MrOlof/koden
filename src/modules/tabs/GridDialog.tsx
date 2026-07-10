import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import {
  GridIcon,
  MinusSignIcon,
  PlusSignIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useCallback, useEffect, useState } from "react";

const RECENTS_KEY = "koden.grid.recentCmds";
const RECENTS_CAP = 5;
const HEAVY_THRESHOLD = 16;
const PRESETS: ReadonlyArray<readonly [number, number]> = [
  [2, 2],
  [3, 3],
  [4, 4],
];

function loadRecents(): string[] {
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((x): x is string => typeof x === "string").slice(0, RECENTS_CAP);
  } catch {
    return [];
  }
}

function pushRecent(list: string[], cmd: string): string[] {
  const trimmed = cmd.trim();
  if (!trimmed) return list;
  const deduped = [trimmed, ...list.filter((c) => c !== trimmed)];
  return deduped.slice(0, RECENTS_CAP);
}

function clampDim(n: number): number {
  if (!Number.isFinite(n)) return 1;
  return Math.min(8, Math.max(1, Math.floor(n)));
}

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (rows: number, cols: number, launchCmd: string) => void;
};

function Stepper({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (next: number) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <div className="flex items-center gap-1">
        <Button
          type="button"
          variant="outline"
          size="icon-sm"
          aria-label={`Decrease ${label}`}
          data-testid={`grid-dec-${label.toLowerCase()}`}
          disabled={value <= 1}
          onClick={() => onChange(clampDim(value - 1))}
        >
          <HugeiconsIcon icon={MinusSignIcon} size={14} strokeWidth={2} />
        </Button>
        <Input
          type="number"
          min={1}
          max={8}
          value={value}
          aria-label={label}
          onChange={(e) => onChange(clampDim(Number(e.target.value)))}
          className="h-8 w-14 rounded-xl text-center [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
        />
        <Button
          type="button"
          variant="outline"
          size="icon-sm"
          aria-label={`Increase ${label}`}
          data-testid={`grid-inc-${label.toLowerCase()}`}
          disabled={value >= 8}
          onClick={() => onChange(clampDim(value + 1))}
        >
          <HugeiconsIcon icon={PlusSignIcon} size={14} strokeWidth={2} />
        </Button>
      </div>
    </div>
  );
}

export function GridDialog({ open, onOpenChange, onConfirm }: Props) {
  const [rows, setRows] = useState(2);
  const [cols, setCols] = useState(2);
  const [cmd, setCmd] = useState("");
  const [recents, setRecents] = useState<string[]>([]);

  useEffect(() => {
    if (!open) return;
    setRows(2);
    setCols(2);
    setCmd("");
    setRecents(loadRecents());
  }, [open]);

  const count = rows * cols;
  const heavy = count > HEAVY_THRESHOLD;

  const submit = useCallback(() => {
    const launch = cmd.trim();
    if (launch) {
      const next = pushRecent(loadRecents(), launch);
      try {
        localStorage.setItem(RECENTS_KEY, JSON.stringify(next));
      } catch {
        // localStorage unavailable (private mode / quota): recents are a
        // convenience, so skip persistence rather than block the launch.
      }
    }
    onConfirm(clampDim(rows), clampDim(cols), launch);
    onOpenChange(false);
  }, [cmd, rows, cols, onConfirm, onOpenChange]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-1.75">
            <HugeiconsIcon icon={GridIcon} size={16} strokeWidth={1.75} />
            New grid
          </DialogTitle>
          <DialogDescription>
            Split a new tab into a grid of terminals and run one command in every
            pane.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-1.5">
            {PRESETS.map(([r, c]) => {
              const active = rows === r && cols === c;
              return (
                <Button
                  key={`${r}x${c}`}
                  type="button"
                  size="sm"
                  variant={active ? "secondary" : "outline"}
                  onClick={() => {
                    setRows(r);
                    setCols(c);
                  }}
                >
                  {r}×{c}
                </Button>
              );
            })}
          </div>

          <div className="flex items-end gap-3">
            <Stepper label="Rows" value={rows} onChange={setRows} />
            <span className="pb-2 text-muted-foreground">×</span>
            <Stepper label="Columns" value={cols} onChange={setCols} />
            <div className="flex-1" />
            <div className="pb-1 text-right">
              <div className="text-lg font-semibold tabular-nums">{count}</div>
              <div className="text-[11px] text-muted-foreground">panes</div>
            </div>
          </div>

          {heavy ? (
            <div className="text-xs text-muted-foreground">
              {count} panes — heavy; renders but may be slower.
            </div>
          ) : null}
        </div>

        <div className="flex flex-col gap-2">
          <span className="text-xs font-medium text-muted-foreground">
            Launch command
          </span>
          <Input
            value={cmd}
            autoFocus
            className="font-mono tracking-[0.01em]"
            onChange={(e) => setCmd(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                submit();
              }
            }}
            placeholder="e.g. cm — leave blank for a plain shell"
          />
          {recents.length > 0 ? (
            <div className="flex flex-wrap items-center gap-1.5">
              {recents.map((r) => (
                <button
                  key={r}
                  type="button"
                  onClick={() => setCmd(r)}
                  className={cn(
                    "rounded-full border border-border bg-background px-2.5 py-0.5 font-mono text-xs tracking-[0.01em] text-muted-foreground transition-colors",
                    "hover:bg-muted hover:text-foreground",
                    cmd.trim() === r && "border-ring text-foreground",
                  )}
                >
                  {r}
                </button>
              ))}
            </div>
          ) : null}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={submit} data-testid="grid-create">
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
