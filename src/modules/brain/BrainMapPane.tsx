import { useEffect, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { brainIndexStatus, type BrainStatusReport } from "./lib/bindings";

/**
 * Brain Map (scaffold). The interactive radial knowledge-graph (from the design
 * handoff — brain core → project hubs → file/symbol/memory/dependency bands) is
 * being built; it needs a backend graph snapshot command (files + AST symbols +
 * memory anchors + import edges) which doesn't exist yet. For now this shows the
 * real top-line index state so the entry point is live, not empty.
 */
export function BrainMapPane() {
  const [report, setReport] = useState<BrainStatusReport | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () => {
      brainIndexStatus()
        .then((r) => alive && setReport(r))
        .catch(() => {});
    };
    load();
    const id = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const projects = report?.projects ?? [];
  const files = projects.reduce((acc, p) => acc + p.files, 0);

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto flex max-w-2xl flex-col gap-4 p-6">
        <div>
          <h2 className="text-lg font-semibold">Brain Map</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            An interactive map of everything the brain knows — projects, files, symbols,
            memory, and how they connect. The radial graph is being built from your design
            handoff; below is the live index it will visualize.
          </p>
        </div>

        <div className="flex gap-3">
          <div className="flex-1 rounded-lg border p-3">
            <div className="text-2xl font-semibold tabular-nums">{projects.length}</div>
            <div className="text-xs text-muted-foreground">projects indexed</div>
          </div>
          <div className="flex-1 rounded-lg border p-3">
            <div className="text-2xl font-semibold tabular-nums">{files.toLocaleString()}</div>
            <div className="text-xs text-muted-foreground">files indexed</div>
          </div>
        </div>

        <div className="rounded-lg border">
          <div className="border-b px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            Projects
          </div>
          {projects.length === 0 ? (
            <div className="px-3 py-3 text-sm text-muted-foreground">
              No projects indexed yet. Add one from the Brain pane (+ Add).
            </div>
          ) : (
            <div className="flex flex-col">
              {projects.map((p) => (
                <div
                  key={p.project.id}
                  className="flex items-center gap-2 border-b px-3 py-2 text-sm last:border-b-0"
                >
                  <span className="truncate font-medium">{p.project.name}</span>
                  <span className="ml-auto text-xs text-muted-foreground tabular-nums">
                    {p.files.toLocaleString()} files
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </ScrollArea>
  );
}
