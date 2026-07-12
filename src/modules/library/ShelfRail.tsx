import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { ArrowRight01Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useRef, useState } from "react";
import {
  fmtAgo,
  fmtDay,
  type PageRef,
  pageFromNote,
  type Shelf,
} from "./lib/useLibrary";

const SHELF_CHANGES_SHOWN = 5;

type Props = {
  shelves: Shelf[] | null;
  error: string | null;
  selected: PageRef | null;
  onOpen: (page: PageRef) => void;
};

export function ShelfRail({ shelves, error, selected, onOpen }: Props) {
  const [openIds, setOpenIds] = useState<Set<string>>(new Set());
  const seeded = useRef(false);

  // First load: open the first shelf that actually holds notes.
  useEffect(() => {
    if (seeded.current || !shelves) return;
    seeded.current = true;
    const first = shelves.find((s) => s.notes.length > 0) ?? shelves[0];
    if (first) setOpenIds(new Set([first.project.id]));
  }, [shelves]);

  const toggle = (id: string) => {
    setOpenIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <ScrollArea className="min-h-0 flex-1">
      {shelves === null ? (
        <div className="px-3 py-2 text-[11px] text-muted-foreground">
          Opening the library…
        </div>
      ) : error ? (
        <div className="px-3 py-2 text-[11px] text-destructive">{error}</div>
      ) : shelves.length === 0 ? (
        <div className="px-3 py-2 text-[11px] text-muted-foreground">
          No projects registered. The Brain shelves what it indexes.
        </div>
      ) : (
        shelves.map((shelf) => (
          <ShelfSection
            key={shelf.project.id}
            shelf={shelf}
            open={openIds.has(shelf.project.id)}
            onToggle={() => toggle(shelf.project.id)}
            selected={selected}
            onOpen={onOpen}
          />
        ))
      )}
    </ScrollArea>
  );
}

function ShelfSection({
  shelf,
  open,
  onToggle,
  selected,
  onOpen,
}: {
  shelf: Shelf;
  open: boolean;
  onToggle: () => void;
  selected: PageRef | null;
  onOpen: (page: PageRef) => void;
}) {
  const { project, notes, changes, lastActivityMs } = shelf;
  return (
    <section className="border-b border-border/60">
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-1.5 px-2.5 py-2 text-left hover:bg-accent/50"
        title={project.root}
      >
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          size={12}
          strokeWidth={2}
          className={cn(
            "shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90",
          )}
        />
        <span className="truncate font-mono text-xs font-medium">
          {project.name}
        </span>
        <span className="ml-auto shrink-0 text-[10px] text-muted-foreground tabular-nums">
          {notes.length} {notes.length === 1 ? "note" : "notes"}
          {lastActivityMs ? ` · ${fmtDay(lastActivityMs)}` : ""}
        </span>
      </button>
      {open ? (
        <div className="pb-2">
          {notes.length === 0 ? (
            <div className="px-3 py-1 text-[11px] text-muted-foreground">
              Nothing shelved here yet.
            </div>
          ) : (
            notes.map((n) => {
              const isSelected =
                selected?.project.id === project.id && selected.path === n.path;
              return (
                <button
                  key={`${n.id}:${n.path}`}
                  type="button"
                  onClick={() => onOpen(pageFromNote(shelf, n))}
                  className={cn(
                    "flex w-full items-center gap-1.5 py-1 pr-2.5 pl-7 text-left text-xs",
                    isSelected
                      ? "bg-accent text-foreground"
                      : "text-foreground/80 hover:bg-accent/50",
                  )}
                  title={n.path}
                >
                  <span className="truncate">{n.title}</span>
                  {n.note_type ? (
                    <span className="ml-auto shrink-0 rounded bg-muted px-1 py-px font-mono text-[9px] uppercase text-muted-foreground">
                      {n.note_type}
                    </span>
                  ) : null}
                </button>
              );
            })
          )}
          {changes.length > 0 ? (
            <>
              <div className="mt-1.5 mb-0.5 pr-2.5 pl-7 font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                Recent changes
              </div>
              {changes.slice(0, SHELF_CHANGES_SHOWN).map((ch) => (
                <div
                  key={`${ch.project}:${ch.signature}`}
                  className={cn(
                    "flex items-center gap-1.5 py-1 pr-2.5 pl-7 text-[11px]",
                    ch.status === "reverted" && "opacity-60",
                  )}
                  title={ch.detail}
                >
                  <span className="shrink-0 rounded bg-muted px-1 py-px font-mono text-[9px] uppercase text-muted-foreground">
                    {ch.action}
                  </span>
                  <span className="truncate text-foreground/80">
                    {ch.title}
                  </span>
                  <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">
                    {ch.status === "reverted"
                      ? "reverted"
                      : fmtAgo(ch.applied_ms)}
                  </span>
                </div>
              ))}
            </>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
