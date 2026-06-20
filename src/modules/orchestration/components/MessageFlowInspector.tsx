import { cn } from "@/lib/utils";
import { useMemo, useState } from "react";
import { formatRelativeTime } from "../lib/roleMeta";
import type { FlowEvent, FlowKind } from "../lib/types";
import { FLOW_KINDS } from "../lib/types";
import { useOrchestrationStore } from "../store/orchestrationStore";

const KIND_TONE: Record<FlowKind, string> = {
  message: "#94a3b8",
  delegation: "#60a5fa",
  handoff: "#a78bfa",
  decision: "#fbbf24",
  review: "#34d399",
  audit: "#f472b6",
  approval: "#22c55e",
};

export function MessageFlowInspector() {
  const agents = useOrchestrationStore((s) => s.agents);
  const flow = useOrchestrationStore((s) => s.flow);
  const [filter, setFilter] = useState<FlowKind | "all">("all");

  const nameOf = useMemo(
    () => (id: string | null) => (id ? (agents[id]?.name ?? "unknown") : "all"),
    [agents],
  );

  const shown = useMemo(() => {
    const list = filter === "all" ? flow : flow.filter((e) => e.kind === filter);
    return [...list].reverse();
  }, [flow, filter]);

  const nowMs = Date.now();

  return (
    <div className="flex h-full min-h-0 flex-col rounded-lg border border-border/60 bg-card/30">
      <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-border/50 px-3 py-2">
        <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>
          All
        </FilterChip>
        {FLOW_KINDS.map((k) => (
          <FilterChip key={k} active={filter === k} onClick={() => setFilter(k)}>
            {k}
          </FilterChip>
        ))}
      </div>
      {shown.length === 0 ? (
        <div className="flex flex-1 items-center justify-center p-8 text-center text-sm text-muted-foreground">
          No activity yet. Agent conversations, delegations, handoffs, reviews and
          approvals will appear here as a timeline.
        </div>
      ) : (
        <ol className="min-h-0 flex-1 overflow-y-auto p-3">
          {shown.map((e) => (
            <FlowRow key={e.id} event={e} nameOf={nameOf} nowMs={nowMs} />
          ))}
        </ol>
      )}
    </div>
  );
}

function FlowRow({
  event,
  nameOf,
  nowMs,
}: {
  event: FlowEvent;
  nameOf: (id: string | null) => string;
  nowMs: number;
}) {
  const tone = KIND_TONE[event.kind];
  return (
    <li className="relative flex gap-3 pb-4 pl-4">
      <span
        className="absolute left-0 top-1.5 size-2.5 rounded-full ring-2 ring-card"
        style={{ background: tone }}
      />
      <span className="absolute left-[4.5px] top-4 h-full w-px bg-border/60" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 text-xs">
          <span
            className="rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide"
            style={{ background: `${tone}22`, color: tone }}
          >
            {event.kind}
          </span>
          <span className="truncate font-medium text-foreground">
            {nameOf(event.fromId)}
            {event.toId ? (
              <span className="text-muted-foreground"> → {nameOf(event.toId)}</span>
            ) : null}
          </span>
          <span className="ml-auto shrink-0 text-[10px] text-muted-foreground tabular-nums">
            {formatRelativeTime(event.ts, nowMs)}
          </span>
        </div>
        <p className="mt-1 whitespace-pre-wrap break-words text-[13px] leading-snug text-foreground/90">
          {event.summary}
        </p>
        {event.detail ? (
          <p className="mt-1 whitespace-pre-wrap break-words text-xs text-muted-foreground">
            {event.detail}
          </p>
        ) : null}
      </div>
    </li>
  );
}

function FilterChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-full px-2 py-0.5 text-[11px] font-medium capitalize transition-colors",
        active
          ? "bg-foreground/[0.09] text-foreground"
          : "text-muted-foreground hover:bg-foreground/[0.05] hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}
