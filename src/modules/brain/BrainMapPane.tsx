import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { brainGraph, type BrainGraph, type GraphNode } from "./lib/bindings";

// Viewport coordinate space (the radial graph is laid out centered on 0,0).
const W = 1600;
const H = 1000;
const R_HUB = 170; // project-hub ring radius

// Project palette — saturated mid-tones legible on both light and dark themes
// (from the design handoff). Assigned deterministically by project index.
const PALETTE = [
  "#2f6df6",
  "#1f9d57",
  "#e0322c",
  "#f2a017",
  "#6b3fd4",
  "#119a8e",
  "#ef6c1a",
  "#3b46c4",
  "#d6357f",
  "#5b6470",
];
const MEMORY_COLOR = "#f2b417";

type Pos = { x: number; y: number };

type ProjectMeta = {
  node: GraphNode;
  color: string;
  angle: number;
  childCount: number;
};

type Layout = {
  pos: Map<string, Pos>;
  byId: Map<string, GraphNode>;
  adj: Map<string, Set<string>>;
  projects: ProjectMeta[];
  maxR: number;
};

/** Stable [0,1) hash of a string — deterministic jitter so layout never flickers. */
function hash01(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return ((h >>> 0) % 100000) / 100000;
}

function computeLayout(graph: BrainGraph): Layout {
  const byId = new Map<string, GraphNode>();
  for (const n of graph.nodes) byId.set(n.id, n);

  const adj = new Map<string, Set<string>>();
  const link = (a: string, b: string) => {
    let sa = adj.get(a);
    if (!sa) {
      sa = new Set();
      adj.set(a, sa);
    }
    sa.add(b);
    let sb = adj.get(b);
    if (!sb) {
      sb = new Set();
      adj.set(b, sb);
    }
    sb.add(a);
  };
  for (const e of graph.edges) link(e.a, e.b);

  const pos = new Map<string, Pos>();
  pos.set("brain", { x: 0, y: 0 });

  const projectNodes = graph.nodes.filter((n) => n.kind === "project");
  const P = Math.max(1, projectNodes.length);
  const projects: ProjectMeta[] = [];
  let maxR = R_HUB;

  projectNodes.forEach((proj, i) => {
    const color = PALETTE[i % PALETTE.length];
    const angle = -Math.PI / 2 + (i * 2 * Math.PI) / P;
    pos.set(proj.id, { x: Math.cos(angle) * R_HUB, y: Math.sin(angle) * R_HUB });

    // This project's children: files (by degree desc) then memory notes.
    const files = graph.nodes
      .filter((n) => n.kind === "file" && n.project_id === proj.project_id)
      .sort((a, b) => b.degree - a.degree);
    const memory = graph.nodes.filter(
      (n) => n.kind === "memory" && n.project_id === proj.project_id,
    );
    const children = [...files, ...memory];

    // Fan children into concentric bands within an angular sector. Per-band count
    // scales with total so a 1-project / many-file brain doesn't make a thin spike.
    const sector = ((2 * Math.PI) / P) * (P === 1 ? 1.7 : 0.78);
    const perBand = Math.min(22, Math.max(6, Math.ceil(Math.sqrt(children.length) * 1.4)));
    children.forEach((node, j) => {
      const ring = Math.floor(j / perBand);
      const inRing = j % perBand;
      const ringTotal = Math.min(perBand, children.length - ring * perBand);
      const frac = ringTotal <= 1 ? 0.5 : inRing / (ringTotal - 1);
      const aoff = (frac - 0.5) * sector + (hash01(`${node.id}a`) - 0.5) * 0.05;
      const r = R_HUB + 70 + ring * 46 + (hash01(node.id) - 0.5) * 28;
      const a = angle + aoff;
      pos.set(node.id, { x: Math.cos(a) * r, y: Math.sin(a) * r });
      if (r > maxR) maxR = r;
    });

    projects.push({ node: proj, color, angle, childCount: children.length });
  });

  return { pos, byId, adj, projects, maxR };
}

/**
 * Brain Map — the interactive radial knowledge graph (design handoff). Brain core →
 * project hubs → file/memory nodes, with real containment/import/anchor edges from
 * `brain_graph`. Follows the app theme (Tailwind theme classes for structure;
 * per-project palette for hubs). Camera (pan/zoom/fly-to) is imperative via refs so
 * dragging never thrashes React; hover/select/focus drive re-renders.
 */
export function BrainMapPane() {
  const [graph, setGraph] = useState<BrainGraph | null>(null);
  const [hover, setHover] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [focusPid, setFocusPid] = useState<string | null>(null);

  const svgRef = useRef<SVGSVGElement>(null);
  const camG = useRef<SVGGElement>(null);
  const cam = useRef({ x: 0, y: 0, s: 0.85 });
  const drag = useRef<{ x: number; y: number; cx: number; cy: number; moved: boolean; bg: boolean } | null>(null);

  useEffect(() => {
    let alive = true;
    brainGraph(120)
      .then((g) => alive && setGraph(g))
      .catch(() => alive && setGraph({ nodes: [], edges: [] }));
    return () => {
      alive = false;
    };
  }, []);

  const layout = useMemo(() => (graph ? computeLayout(graph) : null), [graph]);

  const camTransform = useCallback(
    () => `translate(${cam.current.x} ${cam.current.y}) scale(${cam.current.s})`,
    [],
  );
  const applyCam = useCallback(
    (animate: boolean) => {
      const g = camG.current;
      if (!g) return;
      g.style.transition = animate ? "transform .7s cubic-bezier(.22,.61,.36,1)" : "none";
      g.setAttribute("transform", camTransform());
    },
    [camTransform],
  );
  const flyTo = useCallback(
    (x: number, y: number, s: number, animate: boolean) => {
      cam.current = { x: -s * x, y: -s * y, s };
      applyCam(animate);
    },
    [applyCam],
  );

  // Auto-fit the whole graph into view once it's laid out.
  useEffect(() => {
    if (!layout) return;
    const fit = Math.min(1.1, Math.max(0.3, (Math.min(W, H) / 2) * 0.92 / Math.max(1, layout.maxR)));
    cam.current = { x: 0, y: 0, s: fit };
    applyCam(false);
  }, [layout, applyCam]);

  const vbScale = useCallback(() => {
    const r = svgRef.current?.getBoundingClientRect();
    return r ? Math.min(r.width / W, r.height / H) : 1;
  }, []);
  const clientToWorld = useCallback(
    (cx: number, cy: number) => {
      const r = svgRef.current?.getBoundingClientRect();
      if (!r) return { x: 0, y: 0 };
      const sc = Math.min(r.width / W, r.height / H);
      const offX = r.left + (r.width - W * sc) / 2;
      const offY = r.top + (r.height - H * sc) / 2;
      const vbx = (cx - offX) / sc - W / 2;
      const vby = (cy - offY) / sc - H / 2;
      return { x: (vbx - cam.current.x) / cam.current.s, y: (vby - cam.current.y) / cam.current.s };
    },
    [],
  );

  const reset = useCallback(() => {
    if (!layout) return;
    const fit = Math.min(1.1, Math.max(0.3, (Math.min(W, H) / 2) * 0.92 / Math.max(1, layout.maxR)));
    flyTo(0, 0, fit, true);
    setFocusPid(null);
    setSelected(null);
    setHover(null);
  }, [layout, flyTo]);

  // Native wheel listener (passive:false so we can preventDefault the page scroll).
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const before = clientToWorld(e.clientX, e.clientY);
      const factor = Math.exp(-e.deltaY * 0.0014);
      const s2 = Math.max(0.25, Math.min(6, cam.current.s * factor));
      const r = svg.getBoundingClientRect();
      const sc = Math.min(r.width / W, r.height / H);
      const offX = r.left + (r.width - W * sc) / 2;
      const offY = r.top + (r.height - H * sc) / 2;
      const vbx = (e.clientX - offX) / sc - W / 2;
      const vby = (e.clientY - offY) / sc - H / 2;
      cam.current = { s: s2, x: vbx - before.x * s2, y: vby - before.y * s2 };
      applyCam(false);
    };
    svg.addEventListener("wheel", onWheel, { passive: false });
    return () => svg.removeEventListener("wheel", onWheel);
  }, [clientToWorld, applyCam]);

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const target = e.target as Element;
    drag.current = {
      x: e.clientX,
      y: e.clientY,
      cx: cam.current.x,
      cy: cam.current.y,
      moved: false,
      bg: target.getAttribute("data-bg") === "1",
    };
    applyCam(false);
    if (svgRef.current) svgRef.current.style.cursor = "grabbing";
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (Math.abs(dx) + Math.abs(dy) > 4) d.moved = true;
    const sc = vbScale();
    cam.current.x = d.cx + dx / sc;
    cam.current.y = d.cy + dy / sc;
    camG.current?.setAttribute("transform", camTransform());
  };
  const onPointerUp = () => {
    const d = drag.current;
    drag.current = null;
    if (svgRef.current) svgRef.current.style.cursor = "grab";
    if (d && !d.moved && d.bg) reset();
  };

  const focusProject = (pid: string, projNodeId: string) => {
    if (!layout) return;
    const p = layout.pos.get(projNodeId);
    if (p) flyTo(p.x * 1.1, p.y * 1.1, 1.0, true);
    setFocusPid(pid);
    setSelected(null);
  };
  const selectNode = (n: GraphNode) => {
    if (!layout) return;
    const p = layout.pos.get(n.id);
    if (p) flyTo(p.x, p.y, Math.max(1.6, cam.current.s), true);
    setSelected(n.id);
    if (n.project_id) setFocusPid(n.project_id);
  };

  if (!graph || !layout) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-sm text-muted-foreground">
        Loading the brain map…
      </div>
    );
  }
  if (graph.nodes.length === 0) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-sm text-muted-foreground">
        Nothing indexed yet. Add a project from the Brain pane (+ Add).
      </div>
    );
  }

  const { pos, byId, adj } = layout;
  const focusName = focusPid
    ? layout.projects.find((p) => p.node.project_id === focusPid)?.node.label
    : null;
  const selNode = selected ? byId.get(selected) : null;
  const neighbors = selected ? (adj.get(selected) ?? new Set<string>()) : null;

  const nodeOpacity = (n: GraphNode): number => {
    if (n.kind === "project" && focusPid && n.project_id !== focusPid) return 0.18;
    if (focusPid && n.project_id !== focusPid && n.kind !== "project") return 0.07;
    if (neighbors && selected) return n.id === selected || neighbors.has(n.id) ? 1 : 0.12;
    return 1;
  };
  const edgeOpacity = (a: string, b: string, kind: string): number => {
    if (neighbors && selected) return a === selected || b === selected ? 0.8 : 0.04;
    if (hover) return a === hover || b === hover ? 0.7 : 0.06;
    if (focusPid) {
      const an = byId.get(a);
      return an && an.project_id === focusPid ? 0.4 : 0.03;
    }
    return kind === "import" ? 0.22 : kind === "anchor" ? 0.3 : 0.14;
  };

  const projColorOf = (projectId: string) =>
    layout.projects.find((p) => p.node.project_id === projectId)?.color ?? PALETTE[9];

  return (
    <div className="relative h-full w-full overflow-hidden bg-background">
      {/* top stats */}
      <div className="pointer-events-none absolute top-2.5 right-3 z-10 flex items-center gap-2 font-mono text-[11px] text-muted-foreground">
        <span>{graph.nodes.length.toLocaleString()} nodes</span>
        <span className="opacity-40">/</span>
        <span>{graph.edges.length.toLocaleString()} links</span>
      </div>
      {/* hint / focus pill */}
      <div className="pointer-events-none absolute top-2.5 left-1/2 z-10 -translate-x-1/2">
        <span className="rounded-full border bg-background/70 px-3 py-1 font-mono text-[10.5px] text-muted-foreground backdrop-blur">
          {focusName
            ? `Viewing ${focusName} · click background to zoom out`
            : "Click a project to focus · scroll to zoom · drag to pan"}
        </span>
      </div>
      {/* legend */}
      <div className="absolute bottom-3 left-3 z-10 flex flex-col gap-1.5 rounded-lg border bg-background/80 px-3 py-2 backdrop-blur">
        <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">Layers</span>
        <Legend swatch="rounded-[3px] bg-foreground" label="Project" />
        <Legend swatch="rounded-full bg-muted-foreground" label="File / source" />
        <Legend swatch="rounded-full" label="Memory / context" style={{ background: MEMORY_COLOR }} />
      </div>

      <svg
        ref={svgRef}
        viewBox={`${-W / 2} ${-H / 2} ${W} ${H}`}
        preserveAspectRatio="xMidYMid meet"
        width="100%"
        height="100%"
        className="absolute inset-0 block cursor-grab touch-none"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerLeave={onPointerUp}
      >
        <title>Koden Brain Map</title>
        <defs>
          <radialGradient id="brainGlow">
            <stop offset="0%" stopColor={PALETTE[0]} stopOpacity={0.18} />
            <stop offset="100%" stopColor={PALETTE[0]} stopOpacity={0} />
          </radialGradient>
        </defs>
        <rect data-bg="1" x={-W} y={-H} width={W * 2} height={H * 2} fill="transparent" />
        <g ref={camG}>
          {/* edges */}
          <g>
            {graph.edges.map((e) => {
              const a = pos.get(e.a);
              const b = pos.get(e.b);
              if (!a || !b) return null;
              const an = byId.get(e.a);
              const hot =
                (!!hover && (e.a === hover || e.b === hover)) ||
                (!!selected && (e.a === selected || e.b === selected));
              const stroke = hot && an?.project_id ? projColorOf(an.project_id) : "currentColor";
              return (
                <line
                  key={`${e.a}|${e.b}|${e.kind}`}
                  x1={a.x}
                  y1={a.y}
                  x2={b.x}
                  y2={b.y}
                  className={hot ? undefined : "text-border"}
                  stroke={stroke}
                  strokeWidth={hot ? 1.4 : 0.7}
                  strokeOpacity={edgeOpacity(e.a, e.b, e.kind)}
                />
              );
            })}
          </g>
          {/* spines: brain → hubs */}
          <g className="text-border">
            {layout.projects.map((p) => {
              const h = pos.get(p.node.id);
              if (!h) return null;
              return (
                <line
                  key={`s${p.node.id}`}
                  x1={0}
                  y1={0}
                  x2={h.x}
                  y2={h.y}
                  stroke={focusPid === p.node.project_id ? p.color : "currentColor"}
                  strokeWidth={focusPid === p.node.project_id ? 1.3 : 0.9}
                  strokeOpacity={focusPid ? (focusPid === p.node.project_id ? 0.5 : 0.05) : 0.28}
                />
              );
            })}
          </g>
          {/* file + memory nodes */}
          <g>
            {graph.nodes.map((n) => {
              if (n.kind !== "file" && n.kind !== "memory") return null;
              const p = pos.get(n.id);
              if (!p) return null;
              const emph = n.id === hover || n.id === selected;
              const isMem = n.kind === "memory";
              const r = (isMem ? 4.3 : 3.4 + Math.min(n.degree, 8) * 0.35) * (emph ? 1.8 : 1);
              return (
                // biome-ignore lint/a11y/noStaticElementInteractions: pointer-driven graph node; keyboard access is via the Brain search list
                <circle
                  key={n.id}
                  cx={p.x}
                  cy={p.y}
                  r={r}
                  className={isMem ? undefined : "fill-muted-foreground"}
                  fill={isMem ? MEMORY_COLOR : undefined}
                  stroke={emph ? "currentColor" : "none"}
                  strokeWidth={emph ? 1.3 : 0}
                  opacity={nodeOpacity(n)}
                  style={{ cursor: "pointer" }}
                  onMouseEnter={() => setHover(n.id)}
                  onMouseLeave={() => setHover((h) => (h === n.id ? null : h))}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    selectNode(n);
                  }}
                />
              );
            })}
          </g>
          {/* project hubs */}
          <g>
            {layout.projects.map((p) => {
              const h = pos.get(p.node.id);
              if (!h) return null;
              const emph = p.node.id === hover || focusPid === p.node.project_id;
              const r = 17 * (emph ? 1.12 : 1);
              return (
                // biome-ignore lint/a11y/noStaticElementInteractions: pointer-driven project hub; keyboard access is via the Brain search list
                <g
                  key={p.node.id}
                  opacity={nodeOpacity(p.node)}
                  style={{ cursor: "pointer" }}
                  onMouseEnter={() => setHover(p.node.id)}
                  onMouseLeave={() => setHover((hv) => (hv === p.node.id ? null : hv))}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    focusProject(p.node.project_id, p.node.id);
                  }}
                >
                  <circle cx={h.x} cy={h.y} r={r + 5} fill="none" stroke={p.color} strokeWidth={1.3} strokeOpacity={emph ? 0.5 : 0.22} />
                  <circle cx={h.x} cy={h.y} r={r} fill={p.color} className="stroke-background" strokeWidth={2.4} />
                  <text
                    x={h.x}
                    y={h.y}
                    textAnchor="middle"
                    dominantBaseline="central"
                    className="pointer-events-none fill-white font-semibold"
                    fontSize={11}
                  >
                    {(p.node.label.slice(0, 2) || "?").replace(/^./, (c) => c.toUpperCase())}
                  </text>
                  <text
                    x={h.x}
                    y={h.y + r + 13}
                    textAnchor="middle"
                    className="pointer-events-none fill-foreground font-semibold"
                    fontSize={11}
                  >
                    {p.node.label}
                  </text>
                </g>
              );
            })}
          </g>
          {/* brain core */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: pointer-driven brain core (click = reset view) */}
          <g style={{ cursor: "pointer" }} onClick={(ev) => { ev.stopPropagation(); reset(); }}>
            <circle cx={0} cy={0} r={105} fill="url(#brainGlow)" className="motion-safe:[animation:koden-breathe_4s_ease-in-out_infinite]" style={{ transformOrigin: "center" }} />
            <circle cx={0} cy={0} r={42} fill="none" className="stroke-border" strokeWidth={1} strokeDasharray="2 5" />
            <circle cx={0} cy={0} r={27} className="fill-foreground" />
            <text x={0} y={0} textAnchor="middle" dominantBaseline="central" className="pointer-events-none fill-background font-bold" fontSize={24}>
              K
            </text>
            <text x={0} y={44} textAnchor="middle" className="pointer-events-none fill-muted-foreground font-mono" fontSize={8} letterSpacing={1}>
              BRAIN
            </text>
          </g>
        </g>
      </svg>

      {/* tooltip */}
      {hover ? <Tooltip id={hover} layout={layout} cam={cam.current} svgRef={svgRef} /> : null}

      {/* side panel */}
      {selNode ? (
        <SidePanel
          node={selNode}
          color={selNode.project_id ? projColorOf(selNode.project_id) : PALETTE[9]}
          projectName={layout.projects.find((p) => p.node.project_id === selNode.project_id)?.node.label ?? "—"}
          connections={adj.get(selNode.id)?.size ?? 0}
          onClose={reset}
        />
      ) : null}
    </div>
  );
}

function Legend({ swatch, label, style }: { swatch: string; label: string; style?: React.CSSProperties }) {
  return (
    <div className="flex items-center gap-2 text-[11px] font-medium text-foreground/80">
      <span className={cn("inline-block size-2.5", swatch)} style={style} />
      {label}
    </div>
  );
}

function Tooltip({
  id,
  layout,
  cam,
  svgRef,
}: {
  id: string;
  layout: Layout;
  cam: { x: number; y: number; s: number };
  svgRef: React.RefObject<SVGSVGElement | null>;
}) {
  const n = layout.byId.get(id);
  const p = layout.pos.get(id);
  const r = svgRef.current?.getBoundingClientRect();
  if (!n || !p || !r) return null;
  const sc = Math.min(r.width / W, r.height / H);
  const offX = r.left + (r.width - W * sc) / 2;
  const offY = r.top + (r.height - H * sc) / 2;
  const vbx = cam.x + cam.s * p.x;
  const vby = cam.y + cam.s * p.y;
  const sx = offX + (vbx + W / 2) * sc;
  const sy = offY + (vby + H / 2) * sc;
  const kind = n.kind === "project" ? "project" : n.kind === "memory" ? "memory" : "file";
  return (
    <div
      className="pointer-events-none fixed z-30 -translate-x-1/2 -translate-y-full rounded-md bg-popover px-2.5 py-1.5 text-popover-foreground shadow-lg"
      style={{ left: sx, top: sy - 14 }}
    >
      <div className="max-w-[240px] truncate text-xs font-semibold">{n.label}</div>
      <div className="font-mono text-[9.5px] uppercase tracking-wide text-muted-foreground">{kind}</div>
    </div>
  );
}

function SidePanel({
  node,
  color,
  projectName,
  connections,
  onClose,
}: {
  node: GraphNode;
  color: string;
  projectName: string;
  connections: number;
  onClose: () => void;
}) {
  const kind = node.kind === "memory" ? "Memory node" : node.kind === "project" ? "Project" : "File";
  return (
    <aside className="absolute top-14 right-3 bottom-3 z-20 flex w-72 flex-col overflow-hidden rounded-2xl border bg-background/95 shadow-2xl backdrop-blur">
      <div className="flex items-center justify-between border-b px-4 py-3">
        <span className="font-mono text-[9.5px] uppercase tracking-wider text-muted-foreground">{kind}</span>
        <button
          type="button"
          onClick={onClose}
          className="flex size-6 items-center justify-center rounded-md bg-muted text-muted-foreground hover:bg-accent hover:text-foreground"
          aria-label="Close"
        >
          ✕
        </button>
      </div>
      <div className="flex flex-col gap-3 px-4 py-4 text-sm">
        <div className="font-semibold break-words leading-tight">{node.label}</div>
        {node.path ? <div className="font-mono text-[11px] break-all text-muted-foreground">{node.path}</div> : null}
        <Field label="Belongs to">
          <span className="inline-flex items-center gap-1.5">
            <span className="inline-block size-2.5 rounded-[3px]" style={{ background: color }} />
            <span className="font-medium">{projectName}</span>
          </span>
        </Field>
        <Field label="Connections">
          <span className="font-medium">{connections} linked</span>
        </Field>
      </div>
      <div className="mt-auto border-t px-4 py-3">
        <button
          type="button"
          onClick={onClose}
          className="w-full rounded-lg border bg-background px-3 py-2 text-xs font-semibold hover:bg-accent"
        >
          ← Back to brain
        </button>
      </div>
    </aside>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="font-mono text-[9.5px] uppercase tracking-wide text-muted-foreground">{label}</span>
      <span className="text-[13px]">{children}</span>
    </div>
  );
}
