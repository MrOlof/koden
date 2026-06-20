import { cn } from "@/lib/utils";
import {
  Add01Icon,
  Cancel01Icon,
  HierarchySquare01Icon,
  Remove01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ROLE_META, STATUS_META } from "../lib/roleMeta";
import { isActiveStatus } from "../lib/topology";
import type { Agent } from "../lib/types";
import { useOrchestrationStore } from "../store/orchestrationStore";

type OpenTarget = { tabId: number; leafId: number };
type Placed = {
  agent: Agent;
  cx: number;
  cy: number;
  size: number;
  hub: boolean;
};
type Edge = {
  id: string;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  active: boolean;
};
type View = { tx: number; ty: number; scale: number };

const HUB_SIZE = 54;
const CHILD_SIZE = 42;
const MIN_SCALE = 0.2;
const MAX_SCALE = 2.4;

type Props = {
  /** Activate the terminal tab backing an agent, when it has one. */
  onActivateAgent?: (tabId: number, leafId: number) => void;
};

/**
 * Interactive constellation graph. Every cluster root (a terminal / the
 * Director) is a hub with its subagents on a clean ring around it, linked by
 * dashed edges; separate roots are separate, isolated clusters arranged in a
 * grid. The whole thing is laid out in a virtual canvas you can pan and zoom,
 * and it auto-fits (zooms out) as the agent count grows so nothing drifts off.
 */
export function AgentTopologyView({ onActivateAgent }: Props) {
  const agents = useOrchestrationStore((s) => s.agents);

  const wrapRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View>({ tx: 0, ty: 0, scale: 1 });
  // Programmatic moves (fit / focus / zoom buttons) glide; drag + wheel stay
  // immediate. Once the user takes control we stop auto-fitting so the view
  // never snaps out from under them as agents spawn and the layout reflows.
  const [smooth, setSmooth] = useState(false);
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const interacted = useRef(false);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) setBox({ w: r.width, h: r.height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const { nodes, edges, bbox } = useMemo(
    () => layout(Object.values(agents)),
    [agents],
  );
  const fitView = useCallback(() => {
    if (box.w === 0 || box.h === 0 || nodes.length === 0) return;
    const pad = 56;
    const scale = clamp(
      Math.min(
        (box.w - pad) / Math.max(1, bbox.w),
        (box.h - pad) / Math.max(1, bbox.h),
      ),
      MIN_SCALE,
      1.6,
    );
    setSmooth(true);
    setView({
      scale,
      tx: box.w / 2 - bbox.cx * scale,
      ty: box.h / 2 - bbox.cy * scale,
    });
  }, [box.w, box.h, nodes.length, bbox]);

  // Focus + lock onto one node (double-click): center it and follow it as the
  // constellation reflows. Taking focus counts as user interaction, so auto-fit
  // stops yanking the view.
  const focusNode = useCallback(
    (fx: number, fy: number, id: string) => {
      if (box.w === 0 || box.h === 0) return;
      interacted.current = true;
      setFocusedId(id);
      setSmooth(true);
      setView((v) => {
        const scale = clamp(Math.max(v.scale, 1.3), MIN_SCALE, MAX_SCALE);
        return { scale, tx: box.w / 2 - fx * scale, ty: box.h / 2 - fy * scale };
      });
    },
    [box.w, box.h],
  );

  // Auto-fit on first measure and whenever the node COUNT changes (so spinning
  // up more agents zooms out to keep them all in view). Manual pan/zoom persists
  // between count changes; the Fit button re-centers on demand.
  const lastFitKey = useRef("");
  useLayoutEffect(() => {
    const key = `${box.w}x${box.h}:${nodes.length}`;
    if (box.w === 0 || nodes.length === 0 || key === lastFitKey.current) return;
    lastFitKey.current = key;
    // Once the user has panned / zoomed / focused, never auto-fit again — that
    // was the "snaps around" complaint as agents spawned.
    if (interacted.current) return;
    fitView();
  }, [box.w, box.h, nodes.length, fitView]);

  // While locked, keep the focused node centered as it moves with each reflow.
  // If it disappears, drop the lock.
  useEffect(() => {
    if (!focusedId || box.w === 0) return;
    const n = nodes.find((p) => p.agent.id === focusedId);
    if (!n) {
      setFocusedId(null);
      return;
    }
    setSmooth(true);
    setView((v) => ({
      scale: v.scale,
      tx: box.w / 2 - n.cx * v.scale,
      ty: box.h / 2 - n.cy * v.scale,
    }));
  }, [focusedId, nodes, box.w, box.h]);

  // Wheel zoom toward the cursor.
  const onWheel = useCallback(
    (e: React.WheelEvent) => {
      const rect = wrapRef.current?.getBoundingClientRect();
      if (!rect) return;
      const px = e.clientX - rect.left;
      const py = e.clientY - rect.top;
      interacted.current = true;
      setSmooth(false);
      setView((v) => {
        const next = clamp(
          v.scale * Math.exp(-e.deltaY * 0.0015),
          MIN_SCALE,
          MAX_SCALE,
        );
        // Keep the world point under the cursor fixed.
        const wx = (px - v.tx) / v.scale;
        const wy = (py - v.ty) / v.scale;
        return { scale: next, tx: px - wx * next, ty: py - wy * next };
      });
    },
    [],
  );

  // Drag to pan (from empty canvas; nodes stop propagation so clicks still work).
  const drag = useRef<{ x: number; y: number; tx: number; ty: number } | null>(
    null,
  );
  const onPointerDown = (e: React.PointerEvent) => {
    // Grabbing empty canvas to pan = manual control; release the lock so it
    // doesn't fight the drag, and stop animating so the pan tracks 1:1.
    interacted.current = true;
    setSmooth(false);
    setFocusedId(null);
    drag.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    setView((v) => ({
      ...v,
      tx: d.tx + (e.clientX - d.x),
      ty: d.ty + (e.clientY - d.y),
    }));
  };
  const endDrag = () => {
    drag.current = null;
  };

  const zoomBy = (factor: number) => {
    interacted.current = true;
    setSmooth(true);
    setView((v) => {
      const next = clamp(v.scale * factor, MIN_SCALE, MAX_SCALE);
      // Zoom about the panel center.
      const wx = (box.w / 2 - v.tx) / v.scale;
      const wy = (box.h / 2 - v.ty) / v.scale;
      return { scale: next, tx: box.w / 2 - wx * next, ty: box.h / 2 - wy * next };
    });
  };

  const resolveTarget = (agent: Agent): OpenTarget | null => {
    if (agent.tabId !== null && agent.leafId !== null)
      return { tabId: agent.tabId, leafId: agent.leafId };
    if (agent.parentId) {
      const parent = agents[agent.parentId];
      if (parent?.tabId != null && parent.leafId != null)
        return { tabId: parent.tabId, leafId: parent.leafId };
    }
    return null;
  };

  const isEmpty = Object.keys(agents).length === 0;

  // Subtle one-shot fade-in on mount (reduced-motion collapses --dur-base to
  // ~0 globally, so this is automatically respected).
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    const id = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(id);
  }, []);

  return (
    <div
      ref={wrapRef}
      onWheel={onWheel}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      className="relative h-full min-h-0 w-full touch-none overflow-hidden rounded-lg border border-border/60 bg-card/30"
      // Faint graph-paper dots that pan with the content (tied to view.tx/ty).
      style={{
        backgroundImage:
          "radial-gradient(circle, color-mix(in oklab, var(--border) 60%, transparent) 1px, transparent 1px)",
        backgroundSize: "24px 24px",
        backgroundPosition: `${view.tx}px ${view.ty}px`,
      }}
    >
      {isEmpty ? (
        <div className="flex h-full items-center justify-center p-8 text-center text-sm text-muted-foreground">
          No agents yet. Open a terminal or start the Director — running agents
          and the subagents they spawn appear here.
        </div>
      ) : null}

      <div
        className="absolute left-0 top-0 origin-top-left"
        style={{
          transform: `translate(${view.tx}px,${view.ty}px) scale(${view.scale})`,
          opacity: mounted ? 1 : 0,
          transition: smooth
            ? "transform 320ms var(--ease-premium), opacity var(--dur-base) var(--ease-premium)"
            : "opacity var(--dur-base) var(--ease-premium)",
        }}
      >
        {bbox.w > 0 ? (
          <svg
            className="pointer-events-none absolute"
            style={{ left: bbox.x, top: bbox.y, overflow: "visible" }}
            width={bbox.w}
            height={bbox.h}
            aria-hidden
          >
            <title>Agent topology</title>
            {edges.map((ed) => (
              <TopologyEdge key={ed.id} edge={ed} ox={bbox.x} oy={bbox.y} />
            ))}
          </svg>
        ) : null}

        {nodes.map((n) => (
          <TopologyNode
            key={n.agent.id}
            agent={n.agent}
            cx={n.cx}
            cy={n.cy}
            size={n.size}
            hub={n.hub}
            target={resolveTarget(n.agent)}
            onActivateAgent={onActivateAgent}
            focused={n.agent.id === focusedId}
            onFocus={focusNode}
          />
        ))}
      </div>

      {focusedId ? (
        <div className="absolute left-2 top-2 flex items-center gap-1 rounded-md border border-border/60 bg-card/85 px-2 py-1 text-[11px] text-muted-foreground shadow-sm">
          <span className="max-w-[140px] truncate">
            Locked: {agents[focusedId]?.name ?? "node"}
          </span>
          <button
            type="button"
            aria-label="Clear focus"
            title="Clear focus"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => setFocusedId(null)}
            className="rounded p-0.5 hover:bg-accent hover:text-foreground"
          >
            <HugeiconsIcon icon={Cancel01Icon} size={12} strokeWidth={2} />
          </button>
        </div>
      ) : null}

      {!isEmpty ? (
        <div className="absolute bottom-2 right-2 flex flex-col gap-1">
          <ZoomBtn label="Zoom in" icon={Add01Icon} onClick={() => zoomBy(1.25)} />
          <ZoomBtn
            label="Zoom out"
            icon={Remove01Icon}
            onClick={() => zoomBy(0.8)}
          />
          <ZoomBtn
            label="Fit"
            icon={HierarchySquare01Icon}
            onClick={() => {
              setFocusedId(null);
              fitView();
            }}
          />
        </div>
      ) : null}
    </div>
  );
}

function ZoomBtn({
  label,
  icon,
  onClick,
}: {
  label: string;
  icon: Parameters<typeof HugeiconsIcon>[0]["icon"];
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={onClick}
      className="flex size-6 items-center justify-center rounded-md border border-border/60 bg-card/85 text-muted-foreground shadow-sm transition-colors hover:bg-accent hover:text-foreground"
    >
      <HugeiconsIcon icon={icon} size={13} strokeWidth={2} />
    </button>
  );
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/**
 * Forest layout in a fixed virtual canvas (pan/zoom handles fitting). Each root
 * (terminal / Director, i.e. an agent with no parent in the set) is a hub with
 * its children on a clean ring around it; clusters are packed into a grid so
 * each one stays a distinct, isolated constellation.
 */
function layout(list: Agent[]): {
  nodes: Placed[];
  edges: Edge[];
  bbox: { x: number; y: number; w: number; h: number; cx: number; cy: number };
} {
  const empty = {
    nodes: [],
    edges: [],
    bbox: { x: 0, y: 0, w: 0, h: 0, cx: 0, cy: 0 },
  };
  if (list.length === 0) return empty;

  const ids = new Set(list.map((a) => a.id));
  const childrenOf = new Map<string, Agent[]>();
  const roots: Agent[] = [];
  for (const a of list) {
    const parent = a.parentId && ids.has(a.parentId) ? a.parentId : null;
    if (parent) {
      const arr = childrenOf.get(parent);
      if (arr) arr.push(a);
      else childrenOf.set(parent, [a]);
    } else {
      roots.push(a);
    }
  }

  // Footprint reserved below each chip for its label. One name line now (the
  // uppercase status line was dropped — status reads from the rim + pip), so
  // this is smaller than when two lines were stacked. Keep in sync with the
  // node's gap-1 + single 11px text line in TopologyNode.
  const label = 16;
  // Ring radius for a hub with k children: clears the hub + its label and keeps
  // children from overlapping each other, with generous breathing room.
  const ringFor = (k: number) =>
    k === 0
      ? 0
      : Math.max(
          HUB_SIZE / 2 + CHILD_SIZE / 2 + 44,
          (CHILD_SIZE + 22) / (2 * Math.sin(Math.PI / Math.max(2, k))),
        );

  // One cluster per root, sized to its own halo (childless roots are small),
  // so clusters pack by their true width instead of fat uniform cells.
  const clusters = roots.map((root) => {
    const kids = childrenOf.get(root.id) ?? [];
    const ring = ringFor(kids.length);
    const extent =
      (kids.length === 0 ? HUB_SIZE / 2 : ring + CHILD_SIZE / 2) + label;
    return { root, kids, ring, extent };
  });

  const gap = 48;
  const cols = Math.max(1, Math.ceil(Math.sqrt(clusters.length)));

  const nodes: Placed[] = [];
  const edges: Edge[] = [];
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const note = (x: number, y: number) => {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
  };

  // Flow clusters into rows of up to `cols`, advancing by each cluster's actual
  // width so neighbours sit a fixed gap apart, not a max-cluster gap apart.
  let y = 0;
  for (let i = 0; i < clusters.length; i += cols) {
    const row = clusters.slice(i, i + cols);
    const rowH = Math.max(...row.map((c) => 2 * c.extent));
    let x = 0;
    for (const c of row) {
      const hcx = x + c.extent;
      const hcy = y + rowH / 2;
      nodes.push({ agent: c.root, cx: hcx, cy: hcy, size: HUB_SIZE, hub: true });
      note(hcx - HUB_SIZE / 2, hcy - HUB_SIZE / 2);
      note(hcx + HUB_SIZE / 2, hcy + HUB_SIZE / 2 + label);

      const k = c.kids.length;
      c.kids.forEach((kid, ki) => {
        const ang = -Math.PI / 2 + (ki * 2 * Math.PI) / k;
        const kcx = hcx + c.ring * Math.cos(ang);
        const kcy = hcy + c.ring * Math.sin(ang);
        nodes.push({
          agent: kid,
          cx: kcx,
          cy: kcy,
          size: CHILD_SIZE,
          hub: false,
        });
        // Drawn start=hub, end=child so the koden-flow dashes march hub→child.
        edges.push({
          id: `e-${kid.id}`,
          x1: hcx,
          y1: hcy,
          x2: kcx,
          y2: kcy,
          active: isActiveStatus(kid.status),
        });
        note(kcx - CHILD_SIZE / 2, kcy - CHILD_SIZE / 2);
        note(kcx + CHILD_SIZE / 2, kcy + CHILD_SIZE / 2 + label);
      });
      x += 2 * c.extent + gap;
    }
    y += rowH + gap;
  }

  if (!Number.isFinite(minX)) return empty;
  // bbox is the ACTUAL content extent (not the grid), so fit zooms to the nodes
  // instead of shrinking everything to show empty space.
  return {
    nodes,
    edges,
    bbox: {
      x: minX,
      y: minY,
      w: maxX - minX,
      h: maxY - minY,
      cx: (minX + maxX) / 2,
      cy: (minY + maxY) / 2,
    },
  };
}

/**
 * One ownership edge: a gentle quadratic curve from hub → child. Idle edges are
 * a static theme-colored stroke; active child edges get the marching koden-flow
 * dashes (hub→child, since the path is drawn start=hub). Reproduces the per-edge
 * bbox offset so the path lives in the SVG's local coordinate space.
 */
function TopologyEdge({
  edge,
  ox,
  oy,
}: {
  edge: Edge;
  ox: number;
  oy: number;
}) {
  const x1 = edge.x1 - ox;
  const y1 = edge.y1 - oy;
  const x2 = edge.x2 - ox;
  const y2 = edge.y2 - oy;
  // Control point: midpoint nudged perpendicular to the line so the curve bows
  // gently. The sign is consistent per edge, giving each spoke a soft arc.
  const mx = (x1 + x2) / 2;
  const my = (y1 + y2) / 2;
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len = Math.hypot(dx, dy) || 1;
  const bow = Math.min(18, len * 0.16);
  const cx = mx + (-dy / len) * bow;
  const cy = my + (dx / len) * bow;
  const d = `M${x1} ${y1} Q${cx} ${cy} ${x2} ${y2}`;

  return (
    <path
      d={d}
      fill="none"
      stroke="color-mix(in oklab, var(--muted-foreground) 45%, var(--border))"
      strokeWidth={1.5}
      strokeLinecap="round"
      vectorEffect="non-scaling-stroke"
      className={edge.active ? "koden-flow" : undefined}
      style={
        edge.active
          ? ({
              "--koden-dash": "12px",
              strokeDasharray: "6 6",
            } as React.CSSProperties)
          : undefined
      }
    />
  );
}

function TopologyNode({
  agent,
  cx,
  cy,
  size,
  hub,
  target,
  onActivateAgent,
  focused,
  onFocus,
}: {
  agent: Agent;
  cx: number;
  cy: number;
  size: number;
  hub: boolean;
  target: OpenTarget | null;
  onActivateAgent?: (tabId: number, leafId: number) => void;
  focused: boolean;
  onFocus: (cx: number, cy: number, id: string) => void;
}) {
  const role = ROLE_META[agent.role];
  const status = STATUS_META[agent.status];
  const canOpen = target !== null && !!onActivateAgent;
  const active = isActiveStatus(agent.status);
  const done = agent.status === "done";
  const isDirector = agent.role === "director";
  const icon = Math.round(size * (hub ? 0.4 : 0.42));

  // Monochrome card chip: the status color lives in a thin 1px rim, not a
  // saturated fill. The Director is the hub anchor — a filled primary chip so
  // the hierarchy reads at a glance; everyone else is card-with-rim.
  const rim = `color-mix(in oklab, ${status.dot} 40%, var(--border))`;
  const chipBg = isDirector ? "var(--primary)" : "var(--card)";
  const iconColor = isDirector
    ? "var(--primary-foreground)"
    : "var(--foreground)";
  // A single soft glow only while the chip is doing something; idle/done flat.
  // A locked/focused node gets a clear ring so you can see what's pinned.
  const glow = focused
    ? `0 0 0 2px var(--ring), 0 0 26px -4px ${status.dot}`
    : active
      ? `0 0 0 1px var(--card), 0 0 20px -6px ${status.dot}`
      : "0 0 0 1px var(--card)";

  return (
    <button
      type="button"
      title={
        agent.task
          ? `${agent.name} — ${status.label} — ${agent.task} · double-click to focus`
          : `${agent.name} — ${status.label} · double-click to focus`
      }
      onPointerDown={(e) => e.stopPropagation()}
      onClick={() => {
        if (canOpen && target) onActivateAgent?.(target.tabId, target.leafId);
      }}
      onDoubleClick={(e) => {
        e.stopPropagation();
        onFocus(cx, cy, agent.id);
      }}
      style={{
        left: cx,
        top: cy,
        width: size,
        transform: "translate(-50%, -50%)",
      }}
      className={cn(
        "group absolute flex flex-col items-center gap-1 transition-opacity",
        canOpen ? "cursor-pointer" : "cursor-default",
        done && "opacity-55 hover:opacity-90",
      )}
    >
      <span
        className={cn(
          "relative flex items-center justify-center rounded-full transition-transform",
          canOpen && "hover:scale-110",
        )}
        style={{
          width: size,
          height: size,
          background: chipBg,
          border: `1px solid ${isDirector ? "var(--primary)" : rim}`,
          color: iconColor,
          boxShadow: glow,
        }}
      >
        <HugeiconsIcon icon={role.icon} size={icon} strokeWidth={2} />
        {/* Small status pip, like a presence dot. */}
        <span
          className={cn(
            "absolute bottom-0 right-0 size-2.5 rounded-full border-2 border-[var(--card)]",
            status.pulse && "koden-pulse",
          )}
          style={{ background: status.dot }}
        />
      </span>
      {/* Status reads from rim + pip; the label color conveys hue and the name
          lifts to foreground on hover. */}
      <span
        className="max-w-[110px] truncate text-[11px] font-medium leading-tight text-muted-foreground transition-colors group-hover:text-foreground"
        style={{ maxWidth: Math.max(size + 40, 80) }}
      >
        {agent.name}
      </span>
    </button>
  );
}
