import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { brainGraph, type BrainGraph, type GraphEdge, type GraphNode } from "./lib/bindings";

// Viewport coordinate space (the radial tree is laid out centered on 0,0).
const W = 1600;
const H = 1000;
const R_HUB = 150; // project-hub ring radius
const RING = 62; // radial gap between directory-tree depths

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
type Kind = "project" | "folder" | "file" | "memory";

// A laid-out node (projects + synthetic folder nodes + files + memory).
type RNode = {
  id: string;
  kind: Kind;
  label: string;
  projectId: string;
  path?: string;
  depth: number;
  leaf: number; // subtree leaf count (drives angular share + folder size)
  color: string;
};

type Layout = {
  pos: Map<string, Pos>;
  nodes: RNode[];
  treeEdges: [string, string][];
  realEdges: GraphEdge[]; // imports + anchors (containment is the tree)
  byId: Map<string, RNode>;
  adj: Map<string, Set<string>>;
  colorByProject: Map<string, string>;
  maxR: number;
  projectCount: number;
};

// ── directory tree from file paths ──────────────────────────────────────────
type Tree = {
  id: string;
  kind: Kind;
  label: string;
  path?: string;
  children: Map<string, Tree>;
  leaf: number;
};

function buildTree(
  projId: string,
  projNodeId: string,
  projLabel: string,
  files: GraphNode[],
  memory: GraphNode[],
): Tree {
  const root: Tree = { id: projNodeId, kind: "project", label: projLabel, children: new Map(), leaf: 0 };
  for (const f of files) {
    const path = f.path ?? f.label;
    const segs = path.split(/[\\/]/).filter(Boolean);
    let cur = root;
    let prefix = "";
    segs.forEach((seg, i) => {
      if (i === segs.length - 1) {
        // file leaf — reuse the backend id so real import/anchor edges resolve
        if (!cur.children.has(f.id)) {
          cur.children.set(f.id, { id: f.id, kind: "file", label: seg, path, children: new Map(), leaf: 1 });
        }
      } else {
        prefix = prefix ? `${prefix}/${seg}` : seg;
        const fid = `fold:${projId}:${prefix}`;
        let next = cur.children.get(fid);
        if (!next) {
          next = { id: fid, kind: "folder", label: seg, path: prefix, children: new Map(), leaf: 0 };
          cur.children.set(fid, next);
        }
        cur = next;
      }
    });
  }
  for (const m of memory) {
    root.children.set(m.id, {
      id: m.id,
      kind: "memory",
      label: m.label,
      path: m.path ?? undefined,
      children: new Map(),
      leaf: 1,
    });
  }
  const computeLeaf = (t: Tree): number => {
    if (t.children.size === 0) {
      t.leaf = t.kind === "folder" ? 0 : 1;
      return t.leaf;
    }
    let s = 0;
    for (const c of t.children.values()) s += computeLeaf(c);
    t.leaf = Math.max(1, s);
    return t.leaf;
  };
  computeLeaf(root);
  // Collapse single-child folder chains: src → modules → brain becomes one
  // "src/modules/brain" node, killing the long thin branches a real (deep) tree
  // produces but a hand-balanced mock never has.
  const compress = (t: Tree) => {
    const merged = new Map<string, Tree>();
    for (let c of t.children.values()) {
      while (c.kind === "folder" && c.children.size === 1) {
        const only = c.children.values().next().value as Tree;
        if (only.kind !== "folder") break;
        only.label = `${c.label}/${only.label}`;
        c = only;
      }
      compress(c);
      merged.set(c.id, c);
    }
    t.children = merged;
  };
  compress(root);
  return root;
}

function computeLayout(graph: BrainGraph): Layout {
  const pos = new Map<string, Pos>();
  const nodes: RNode[] = [];
  const treeEdges: [string, string][] = [];
  const colorByProject = new Map<string, string>();
  let maxR = R_HUB;

  const projectNodes = graph.nodes.filter((n) => n.kind === "project");
  const P = Math.max(1, projectNodes.length);

  const place = (t: Tree, depth: number, angStart: number, angEnd: number, color: string, pid: string) => {
    const ang = (angStart + angEnd) / 2;
    const radius = R_HUB + depth * RING;
    pos.set(t.id, { x: Math.cos(ang) * radius, y: Math.sin(ang) * radius });
    if (radius > maxR) maxR = radius;
    nodes.push({ id: t.id, kind: t.kind, label: t.label, projectId: pid, path: t.path, depth, leaf: t.leaf, color });
    const kids = [...t.children.values()];
    if (!kids.length) return;
    const total = kids.reduce((s, k) => s + Math.sqrt(k.leaf), 0) || 1;
    let cursor = angStart;
    for (const k of kids) {
      const span = (angEnd - angStart) * (Math.sqrt(k.leaf) / total);
      treeEdges.push([t.id, k.id]);
      place(k, depth + 1, cursor, cursor + span, color, pid);
      cursor += span;
    }
  };

  projectNodes.forEach((proj, i) => {
    const color = PALETTE[i % PALETTE.length];
    colorByProject.set(proj.project_id, color);
    const a = -Math.PI / 2 + (i * 2 * Math.PI) / P;
    pos.set(proj.id, { x: Math.cos(a) * R_HUB, y: Math.sin(a) * R_HUB });
    nodes.push({ id: proj.id, kind: "project", label: proj.label, projectId: proj.project_id, depth: 0, leaf: 1, color });

    const files = graph.nodes.filter((n) => n.kind === "file" && n.project_id === proj.project_id);
    const memory = graph.nodes.filter((n) => n.kind === "memory" && n.project_id === proj.project_id);
    const root = buildTree(proj.project_id, proj.id, proj.label, files, memory);

    // The project's subtree fans across an angular sector centered on its direction.
    const sector = P === 1 ? Math.PI * 1.8 : ((2 * Math.PI) / P) * 0.86;
    const kids = [...root.children.values()];
    const total = kids.reduce((s, k) => s + Math.sqrt(k.leaf), 0) || 1;
    let cursor = a - sector / 2;
    for (const k of kids) {
      const span = sector * (Math.sqrt(k.leaf) / total);
      treeEdges.push([proj.id, k.id]);
      place(k, 1, cursor, cursor + span, color, proj.project_id);
      cursor += span;
    }
  });

  const byId = new Map(nodes.map((n) => [n.id, n]));
  const realEdges = graph.edges.filter((e) => e.kind !== "contains");
  const adj = new Map<string, Set<string>>();
  const link = (x: string, y: string) => {
    let sx = adj.get(x);
    if (!sx) {
      sx = new Set();
      adj.set(x, sx);
    }
    sx.add(y);
    let sy = adj.get(y);
    if (!sy) {
      sy = new Set();
      adj.set(y, sy);
    }
    sy.add(x);
  };
  for (const [x, y] of treeEdges) link(x, y);
  for (const e of realEdges) link(e.a, e.b);

  return { pos, nodes, treeEdges, realEdges, byId, adj, colorByProject, maxR, projectCount: projectNodes.length };
}

/**
 * Brain Map — radial directory-tree of the whole brain (design handoff). Project
 * hubs → folders → files branch outward (real containment = the tree); memory notes
 * are amber leaves; imports/anchors light up on hover. Follows the app theme.
 * Camera (pan/zoom/fly-to) is imperative via refs for smoothness; hover/select/focus
 * drive React re-renders.
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
    brainGraph(160)
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

  const fitScale = useCallback(
    () => Math.min(1.1, Math.max(0.25, ((Math.min(W, H) / 2) * 0.92) / Math.max(1, layout?.maxR ?? 1))),
    [layout],
  );
  useLayoutEffect(() => {
    if (!layout) return;
    cam.current = { x: 0, y: 0, s: fitScale() };
    applyCam(false);
  }, [layout, applyCam, fitScale]);

  const vbScale = useCallback(() => {
    const r = svgRef.current?.getBoundingClientRect();
    return r ? Math.min(r.width / W, r.height / H) : 1;
  }, []);
  const clientToWorld = useCallback((cx: number, cy: number) => {
    const r = svgRef.current?.getBoundingClientRect();
    if (!r) return { x: 0, y: 0 };
    const sc = Math.min(r.width / W, r.height / H);
    const offX = r.left + (r.width - W * sc) / 2;
    const offY = r.top + (r.height - H * sc) / 2;
    const vbx = (cx - offX) / sc - W / 2;
    const vby = (cy - offY) / sc - H / 2;
    return { x: (vbx - cam.current.x) / cam.current.s, y: (vby - cam.current.y) / cam.current.s };
  }, []);

  const reset = useCallback(() => {
    flyTo(0, 0, fitScale(), true);
    setFocusPid(null);
    setSelected(null);
    setHover(null);
  }, [flyTo, fitScale]);

  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const before = clientToWorld(e.clientX, e.clientY);
      const factor = Math.exp(-e.deltaY * 0.0014);
      const s2 = Math.max(0.2, Math.min(7, cam.current.s * factor));
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
    applyCam(false); // cancel any in-flight fly-to transition before dragging
    if (svgRef.current) svgRef.current.style.cursor = "grabbing";
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    // Capture only once a real DRAG starts (not on a click) — capturing on
    // pointerdown would steal the click from nodes and break focus/select.
    if (!d.moved && Math.abs(dx) + Math.abs(dy) > 4) {
      d.moved = true;
      try {
        (e.currentTarget as Element).setPointerCapture(e.pointerId);
      } catch {
        // pointer already gone — harmless.
      }
    }
    const sc = vbScale();
    cam.current.x = d.cx + dx / sc;
    cam.current.y = d.cy + dy / sc;
    camG.current?.setAttribute("transform", camTransform());
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = drag.current;
    drag.current = null;
    try {
      (e.currentTarget as Element).releasePointerCapture(e.pointerId);
    } catch {
      // already released — harmless.
    }
    if (svgRef.current) svgRef.current.style.cursor = "grab";
    if (d && !d.moved && d.bg) reset();
  };

  const focusProject = (pid: string, projNodeId: string) => {
    if (!layout) return;
    const p = layout.pos.get(projNodeId);
    if (p) flyTo(p.x * 1.15, p.y * 1.15, 1.0, true);
    setFocusPid(pid);
    setSelected(null);
  };
  const selectNode = (n: RNode) => {
    if (!layout) return;
    const p = layout.pos.get(n.id);
    if (p) flyTo(p.x, p.y, Math.max(1.7, cam.current.s), true);
    setSelected(n.id);
    if (n.projectId) setFocusPid(n.projectId);
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
    ? layout.nodes.find((n) => n.kind === "project" && n.projectId === focusPid)?.label
    : null;
  const selNode = selected ? byId.get(selected) : null;
  const active = selected ?? hover;
  const neighbors = selected ? (adj.get(selected) ?? new Set<string>()) : null;
  const projColorOf = (projectId: string) => layout.colorByProject.get(projectId) ?? PALETTE[9];

  const nodeOpacity = (n: RNode): number => {
    if (n.kind === "project" && focusPid && n.projectId !== focusPid) return 0.2;
    if (focusPid && n.projectId !== focusPid && n.kind !== "project") return 0.08;
    if (neighbors && selected) return n.id === selected || neighbors.has(n.id) ? 1 : 0.14;
    return 1;
  };

  return (
    <div className="relative h-full w-full overflow-hidden bg-background">
      <div className="pointer-events-none absolute top-2.5 right-3 z-10 flex items-center gap-2 font-mono text-[11px] text-muted-foreground">
        <span>{layout.nodes.length.toLocaleString()} nodes</span>
        <span className="opacity-40">/</span>
        <span>{layout.treeEdges.length.toLocaleString()} links</span>
      </div>
      <div className="pointer-events-none absolute top-2.5 left-1/2 z-10 -translate-x-1/2">
        <span className="rounded-full border bg-background/70 px-3 py-1 font-mono text-[10.5px] text-muted-foreground backdrop-blur">
          {focusName
            ? `Viewing ${focusName} · click background to zoom out`
            : "Click a project to focus · scroll to zoom · drag to pan"}
        </span>
      </div>
      <div className="absolute bottom-3 left-3 z-10 flex flex-col gap-1.5 rounded-lg border bg-background/80 px-3 py-2 backdrop-blur">
        <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">Layers</span>
        <Legend swatch="rounded-[3px] bg-foreground" label="Project" />
        <Legend swatch="rounded-[3px] bg-muted-foreground" label="Folder / module" />
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
          {/* spines: brain core → project hubs */}
          <g className="text-muted-foreground">
            {layout.nodes.map((n) => {
              if (n.kind !== "project") return null;
              const h = pos.get(n.id);
              if (!h) return null;
              const on = !focusPid || focusPid === n.projectId;
              return (
                <line
                  key={`spine${n.id}`}
                  x1={0}
                  y1={0}
                  x2={h.x}
                  y2={h.y}
                  stroke={focusPid === n.projectId ? n.color : "currentColor"}
                  strokeWidth={focusPid === n.projectId ? 1.6 : 1.1}
                  strokeOpacity={on ? 0.45 : 0.06}
                />
              );
            })}
          </g>
          {/* tree branches (project → folder → file/memory) */}
          <g className="text-muted-foreground">
            {layout.treeEdges.map(([aId, bId]) => {
              const a = pos.get(aId);
              const b = pos.get(bId);
              if (!a || !b) return null;
              const an = byId.get(aId);
              const dimmed = !!focusPid && an?.projectId !== focusPid;
              const hot =
                (!!active && (aId === active || bId === active)) ||
                (!!focusPid && an?.projectId === focusPid);
              return (
                <line
                  key={`t${aId}>${bId}`}
                  x1={a.x}
                  y1={a.y}
                  x2={b.x}
                  y2={b.y}
                  stroke={hot && an?.projectId ? projColorOf(an.projectId) : "currentColor"}
                  strokeWidth={hot ? 1.2 : 0.8}
                  strokeOpacity={dimmed ? 0.05 : hot ? 0.55 : 0.32}
                />
              );
            })}
          </g>
          {/* real import/anchor edges — only for the active (hovered/selected) node */}
          {active ? (
            <g>
              {layout.realEdges
                .filter((e) => e.a === active || e.b === active)
                .map((e) => {
                  const a = pos.get(e.a);
                  const b = pos.get(e.b);
                  if (!a || !b) return null;
                  return (
                    <line
                      key={`r${e.a}>${e.b}`}
                      x1={a.x}
                      y1={a.y}
                      x2={b.x}
                      y2={b.y}
                      stroke={e.kind === "anchor" ? MEMORY_COLOR : projColorOf(byId.get(e.a)?.projectId ?? "")}
                      strokeWidth={1.4}
                      strokeOpacity={0.85}
                      strokeDasharray={e.kind === "anchor" ? "3 3" : undefined}
                    />
                  );
                })}
            </g>
          ) : null}
          {/* leaf nodes: files (grey) + memory (amber) */}
          <g>
            {layout.nodes.map((n) => {
              if (n.kind !== "file" && n.kind !== "memory") return null;
              const p = pos.get(n.id);
              if (!p) return null;
              const emph = n.id === hover || n.id === selected;
              const isMem = n.kind === "memory";
              const r = (isMem ? 4.2 : 3.3) * (emph ? 1.9 : 1);
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
          {/* folder / module nodes: colored rounded squares with a letter badge */}
          <g>
            {layout.nodes.map((n) => {
              if (n.kind !== "folder") return null;
              const p = pos.get(n.id);
              if (!p) return null;
              const emph = n.id === hover || n.id === selected;
              const s = Math.min(10, 4.5 + Math.log2(n.leaf + 1) * 1.3) * (emph ? 1.25 : 1);
              return (
                // biome-ignore lint/a11y/noStaticElementInteractions: pointer-driven folder node; keyboard access is via the Brain search list
                <g
                  key={n.id}
                  opacity={nodeOpacity(n)}
                  style={{ cursor: "pointer" }}
                  onMouseEnter={() => setHover(n.id)}
                  onMouseLeave={() => setHover((h) => (h === n.id ? null : h))}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    selectNode(n);
                  }}
                >
                  <rect
                    x={p.x - s}
                    y={p.y - s}
                    width={s * 2}
                    height={s * 2}
                    rx={s * 0.45}
                    fill={n.color}
                    className="stroke-background"
                    strokeWidth={1.5}
                  />
                  <text
                    x={p.x}
                    y={p.y}
                    textAnchor="middle"
                    dominantBaseline="central"
                    className="pointer-events-none fill-white font-bold"
                    fontSize={s * 1.05}
                  >
                    {(n.label[0] ?? "?").toUpperCase()}
                  </text>
                </g>
              );
            })}
          </g>
          {/* project hubs */}
          <g>
            {layout.nodes.map((n) => {
              if (n.kind !== "project") return null;
              const h = pos.get(n.id);
              if (!h) return null;
              const emph = n.id === hover || focusPid === n.projectId;
              const r = 16 * (emph ? 1.12 : 1);
              return (
                // biome-ignore lint/a11y/noStaticElementInteractions: pointer-driven project hub; keyboard access is via the Brain search list
                <g
                  key={n.id}
                  opacity={nodeOpacity(n)}
                  style={{ cursor: "pointer" }}
                  onMouseEnter={() => setHover(n.id)}
                  onMouseLeave={() => setHover((hv) => (hv === n.id ? null : hv))}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    focusProject(n.projectId, n.id);
                  }}
                >
                  <circle cx={h.x} cy={h.y} r={r + 5} fill="none" stroke={n.color} strokeWidth={1.3} strokeOpacity={emph ? 0.5 : 0.22} />
                  <circle cx={h.x} cy={h.y} r={r} fill={n.color} className="stroke-background" strokeWidth={2.4} />
                  <text x={h.x} y={h.y} textAnchor="middle" dominantBaseline="central" className="pointer-events-none fill-white font-semibold" fontSize={10.5}>
                    {(n.label.slice(0, 2) || "?").replace(/^./, (c) => c.toUpperCase())}
                  </text>
                  <text x={h.x} y={h.y + r + 13} textAnchor="middle" className="pointer-events-none fill-foreground font-semibold" fontSize={11}>
                    {n.label}
                  </text>
                </g>
              );
            })}
          </g>
          {/* brain core */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: pointer-driven brain core (click = reset view) */}
          <g style={{ cursor: "pointer" }} onClick={(ev) => { ev.stopPropagation(); reset(); }}>
            <circle cx={0} cy={0} r={100} fill="url(#brainGlow)" className="motion-safe:[animation:koden-breathe_4s_ease-in-out_infinite]" style={{ transformOrigin: "center" }} />
            <circle cx={0} cy={0} r={40} fill="none" className="stroke-border" strokeWidth={1} strokeDasharray="2 5" />
            <circle cx={0} cy={0} r={26} className="fill-foreground" />
            <text x={0} y={0} textAnchor="middle" dominantBaseline="central" className="pointer-events-none fill-background font-bold" fontSize={23}>
              K
            </text>
            <text x={0} y={42} textAnchor="middle" className="pointer-events-none fill-muted-foreground font-mono" fontSize={8} letterSpacing={1}>
              BRAIN
            </text>
          </g>
        </g>
      </svg>

      {hover ? <Tooltip id={hover} layout={layout} cam={cam.current} svgRef={svgRef} /> : null}

      {selNode ? (
        <SidePanel
          node={selNode}
          color={projColorOf(selNode.projectId)}
          projectName={layout.nodes.find((n) => n.kind === "project" && n.projectId === selNode.projectId)?.label ?? "—"}
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
  const sx = offX + (cam.x + cam.s * p.x + W / 2) * sc;
  const sy = offY + (cam.y + cam.s * p.y + H / 2) * sc;
  const kind = n.kind === "project" ? "project" : n.kind === "memory" ? "memory" : n.kind === "folder" ? "folder" : "file";
  return (
    <div
      className="pointer-events-none fixed z-30 -translate-x-1/2 -translate-y-full rounded-md bg-popover px-2.5 py-1.5 text-popover-foreground shadow-lg"
      style={{ left: sx, top: sy - 14 }}
    >
      <div className="max-w-[260px] truncate text-xs font-semibold">{n.label}</div>
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
  node: RNode;
  color: string;
  projectName: string;
  connections: number;
  onClose: () => void;
}) {
  const kind =
    node.kind === "memory"
      ? "Memory node"
      : node.kind === "project"
        ? "Project"
        : node.kind === "folder"
          ? "Folder / module"
          : "File";
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
