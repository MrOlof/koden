import { listenFsChanged } from "@/modules/explorer/lib/watch";
import { useOrchestrationStore } from "@/modules/orchestration/store/orchestrationStore";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type BrainGraph,
  brainGraph,
  brainProposals,
  type GraphNode,
} from "./lib/bindings";

// ── Koden Observatory ────────────────────────────────────────────────────────
// A canvas radial map of the whole brain (design handoff "Koden Observatory",
// adapted to MULTI-HUB: the BRAIN core at center, every indexed project as its
// own sub-hub with its OWN recency rings + module wedges). Files are placed by
// (module = top-level dir → angle) and (recency band → ring), colored + glowed
// by how recently they changed. Live: agent drones (orchestration roster) orbit
// the brain's hottest files; blast-radius arcs follow the AST import graph; risk
// halos come from the review-inbox proposals; the bottom timeline streams real
// fs:changed events and the changed files light up. Dark by design — this view is
// intentionally not theme-following.

const BG_TOP = "#0a0e18";
const BG_MID = "#06070d";
const BG_BOT = "#04050a";
const HUB_PALETTE = [
  "#4d8dff",
  "#2fe08a",
  "#22d3ee",
  "#f5c518",
  "#ff8a3d",
  "#c084fc",
  "#ff5fa8",
  "#2dd4bf",
  "#8b8cff",
  "#9fb2c9",
];

// Recency bands → local ring radius (distance from a project's sub-hub) + color.
// band 0 active(<1h) · 1 today(<24h) · 2 week(<7d) · 3 stale/none.
const BANDS = [
  { label: "ACTIVE NOW", color: "#2fe08a", r: 52 },
  { label: "CHANGED TODAY", color: "#4d8dff", r: 100 },
  { label: "RELATED · WEEK", color: "#5b6373", r: 150 },
  { label: "STALE", color: "#3a4150", r: 200 },
];
const HOUR = 3600_000;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

function bandOf(mtime: number, now: number): number {
  if (!mtime || mtime <= 0) return 3;
  const age = now - mtime;
  if (age < HOUR) return 0;
  if (age < DAY) return 1;
  if (age < WEEK) return 2;
  return 3;
}

function topDirOf(path: string): string {
  const segs = path.split(/[\\/]/).filter(Boolean);
  return segs.length > 1 ? segs[0] : "·root";
}
function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}
function fmtAge(mtime: number, now: number): string {
  if (!mtime || mtime <= 0) return "no recent change";
  const d = now - mtime;
  if (d < 60_000) return "just now";
  const m = Math.floor(d / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

type FNode = {
  id: string;
  name: string;
  path: string;
  projectId: string;
  color: string; // owning hub color
  band: number;
  mtime: number;
  x: number;
  y: number;
  r: number;
  isMemory: boolean;
};
type Hub = {
  projectId: string;
  name: string;
  color: string;
  ang: number; // spoke direction from the brain
  hx: number;
  hy: number;
  modules: { name: string; ang: number; count: number }[];
  fileCount: number;
};
type OLayout = {
  hubs: Hub[];
  files: FNode[];
  byId: Map<string, FNode>;
  hubById: Map<string, Hub>;
  adj: Map<string, string[]>; // import/anchor edges → blast radius
  hotGlobal: string[]; // freshest file ids across the brain → drone targets
  now: number;
  edits24h: number;
  maxR: number;
};

// Distance of each project sub-hub from the brain core; grows a touch with the
// project count so a busy workspace fans out instead of overlapping.
function hubDist(p: number): number {
  return Math.min(560, 320 + p * 10);
}

function buildObservatory(graph: BrainGraph, now: number): OLayout {
  const projects = graph.nodes.filter((n) => n.kind === "project");
  const P = Math.max(1, projects.length);
  const HD = hubDist(P);
  const hubs: Hub[] = [];
  const files: FNode[] = [];
  const byId = new Map<string, FNode>();
  const hubById = new Map<string, Hub>();
  let maxR = HD;
  let edits24h = 0;

  projects.forEach((proj, i) => {
    const color = HUB_PALETTE[i % HUB_PALETTE.length];
    const ang = -Math.PI / 2 + (i * 2 * Math.PI) / P;
    const hx = Math.cos(ang) * HD;
    const hy = Math.sin(ang) * HD;

    const projFiles = graph.nodes.filter(
      (n) =>
        (n.kind === "file" || n.kind === "memory") &&
        n.project_id === proj.project_id,
    );
    // Group by top-level dir (module). Memory nodes share one "memory" module.
    const byModule = new Map<string, GraphNode[]>();
    for (const f of projFiles) {
      const key = f.kind === "memory" ? "memory" : topDirOf(f.path ?? f.label);
      const list = byModule.get(key);
      if (list) list.push(f);
      else byModule.set(key, [f]);
    }
    const modEntries = [...byModule.entries()].sort(
      (a, b) => b[1].length - a[1].length,
    );

    // The project's files fan into a cone pointing radially OUTWARD from the
    // brain (centered on `ang`). Each module gets an angular slice of the cone;
    // recency band sets the distance from the sub-hub.
    const sector = Math.min(1.5, ((2 * Math.PI) / P) * 0.82);
    const modules: Hub["modules"] = [];
    const totalFiles = projFiles.length || 1;
    let cursor = ang - sector / 2;
    for (const [modName, list] of modEntries) {
      const span = sector * (list.length / totalFiles);
      const mid = cursor + span / 2;
      modules.push({ name: modName, ang: mid, count: list.length });
      // Deterministic spread inside the module slice so dots don't stack.
      list.forEach((f, k) => {
        const path = f.path ?? f.label;
        const isMemory = f.kind === "memory";
        const mtime = f.mtime ?? 0;
        if (mtime > 0 && now - mtime < DAY) edits24h++;
        const band = isMemory ? 1 : bandOf(mtime, now);
        const jitter = list.length > 1 ? k / (list.length - 1) - 0.5 : 0;
        const fang = mid + jitter * span * 0.82;
        const rad = BANDS[band].r + ((k % 3) - 1) * 9;
        const fx = hx + Math.cos(fang) * rad;
        const fy = hy + Math.sin(fang) * rad;
        const dist = Math.hypot(fx, fy);
        if (dist > maxR) maxR = dist;
        const node: FNode = {
          id: f.id,
          name: baseName(path),
          path,
          projectId: proj.project_id,
          color,
          band,
          mtime,
          x: fx,
          y: fy,
          r: isMemory ? 3.4 : band === 0 ? 4 : band === 1 ? 3.2 : 2.6,
          isMemory,
        };
        files.push(node);
        byId.set(node.id, node);
      });
      cursor += span;
    }

    const hub: Hub = {
      projectId: proj.project_id,
      name: proj.label,
      color,
      ang,
      hx,
      hy,
      modules,
      fileCount: projFiles.filter((f) => f.kind === "file").length,
    };
    hubs.push(hub);
    hubById.set(proj.project_id, hub);
  });

  // Import/anchor edges (not the tree "contains") → undirected blast-radius graph.
  const adj = new Map<string, string[]>();
  const link = (a: string, b: string) => {
    const cur = adj.get(a);
    if (cur) cur.push(b);
    else adj.set(a, [b]);
  };
  for (const e of graph.edges) {
    if (e.kind === "contains") continue;
    if (byId.has(e.a) && byId.has(e.b)) {
      link(e.a, e.b);
      link(e.b, e.a);
    }
  }

  // Freshest files across the brain → where the live agent drones hover.
  const hotGlobal = [...byId.values()]
    .filter((f) => !f.isMemory && f.mtime > 0)
    .sort((a, b) => b.mtime - a.mtime)
    .slice(0, 12)
    .map((f) => f.id);

  return { hubs, files, byId, hubById, adj, hotGlobal, now, edits24h, maxR };
}

type Pick =
  | { type: "file"; id: string }
  | { type: "hub"; id: string }
  | { type: "agent"; id: string }
  | { type: "brain" }
  | null;

// A live agent drone (orbits the brain's hottest files; re-targets periodically).
type Drone = {
  id: string;
  name: string;
  color: string;
  x: number;
  y: number;
  targetId: string | null;
  trail: { x: number; y: number }[];
  hopAt: number;
  sx: number; // last screen pos (for picking)
  sy: number;
};

// Orchestration status → drone/agent color.
const STATUS_COLOR: Record<string, string> = {
  working: "#4d8dff",
  spawning: "#22d3ee",
  waiting: "#f5c518",
  ready: "#2fe08a",
  done: "#2fe08a",
  error: "#ff5a5f",
};
const droneColor = (status: string) => STATUS_COLOR[status] ?? "#9fb2c9";

export function BrainMapPane() {
  const [graph, setGraph] = useState<BrainGraph | null>(null);
  const [hover, setHover] = useState<Pick>(null);
  const [sel, setSel] = useState<Pick>(null);
  const [focusPid, setFocusPid] = useState<string | null>(null);
  const [blast, setBlast] = useState(false);
  const [risk, setRisk] = useState<{ files: Set<string>; count: number }>({
    files: new Set(),
    count: 0,
  });
  const [ticks, setTicks] = useState<
    { t: number; color: string; id: number }[]
  >([]);

  // Live agents from the orchestration roster → drones (Object identity is stable
  // per render; we reconcile into dronesRef so positions persist across renders).
  const agentMap = useOrchestrationStore((s) => s.agents);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const cam = useRef({ x: 0, y: 0, s: 0.6 });
  const view = useRef({ w: 0, h: 0, dpr: 1 });
  const drag = useRef<{
    x: number;
    y: number;
    cx: number;
    cy: number;
    moved: boolean;
  } | null>(null);
  const raf = useRef<number | null>(null);
  const t0 = useRef(0);
  const hoverRef = useRef<Pick>(null);
  const selRef = useRef<Pick>(null);
  const focusRef = useRef<string | null>(null);
  const blastRef = useRef(false);
  const riskRef = useRef<Set<string>>(new Set());
  const dronesRef = useRef<Drone[]>([]);
  const liveMtimeRef = useRef<Map<string, number>>(new Map());
  const tickSeq = useRef(0);
  const [tip, setTip] = useState<{
    x: number;
    y: number;
    title: string;
    sub: string;
  } | null>(null);

  useEffect(() => {
    let alive = true;
    brainGraph(200)
      .then((g) => alive && setGraph(g))
      .catch(() => alive && setGraph({ nodes: [], edges: [] }));
    return () => {
      alive = false;
    };
  }, []);

  // Layout is stamped with a load-time `now` (stable across redraws so bands
  // don't drift every frame). Recomputed only when the graph changes.
  const layout = useMemo(
    () => (graph ? buildObservatory(graph, Date.now()) : null),
    [graph],
  );

  // Keep refs in sync for the rAF draw loop (which can't read React state).
  hoverRef.current = hover;
  selRef.current = sel;
  focusRef.current = focusPid;
  blastRef.current = blast;
  riskRef.current = risk.files;

  const agents = useMemo(() => Object.values(agentMap), [agentMap]);

  // Risk = the review inbox (pending proposals). The badge is the count; we also
  // best-effort-halo any indexed node a proposal targets (mostly memory notes).
  useEffect(() => {
    if (!layout) return;
    let alive = true;
    brainProposals(null)
      .then((props) => {
        if (!alive) return;
        const pending = props.filter((p) => p.status === "pending");
        const files = new Set<string>();
        for (const p of pending) {
          if (!p.target_id) continue;
          const t = p.target_id.replace(/\\/g, "/").toLowerCase();
          for (const f of layout.files) {
            if (
              f.id === p.target_id ||
              f.path.replace(/\\/g, "/").toLowerCase().endsWith(t)
            ) {
              files.add(f.id);
              break;
            }
          }
        }
        setRisk({ files, count: pending.length });
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [layout]);

  // Reconcile drones from the live agent roster; positions persist across renders.
  useEffect(() => {
    const prev = new Map(dronesRef.current.map((d) => [d.id, d]));
    const hot = layout?.hotGlobal ?? [];
    dronesRef.current = agents.map((a, i) => {
      const ex = prev.get(a.id);
      const color = droneColor(a.status);
      if (ex) {
        ex.name = a.name;
        ex.color = color;
        return ex;
      }
      return {
        id: a.id,
        name: a.name,
        color,
        x: 0,
        y: 0,
        targetId: hot.length ? hot[i % hot.length] : null,
        trail: [],
        hopAt: 0,
        sx: -1,
        sy: -1,
      };
    });
  }, [agents, layout]);

  // Live file changes (fs:changed): light up the file's recency + push a timeline
  // tick. The draw loop reads liveMtimeRef each frame, so no re-render is needed
  // for the canvas — only the (throttled) tick list re-renders the timeline.
  useEffect(() => {
    if (!layout) return;
    let un: (() => void) | null = null;
    let disposed = false;
    listenFsChanged((paths) => {
      const now = Date.now();
      const fresh: { t: number; color: string; id: number }[] = [];
      for (const path of paths) {
        const norm = path.replace(/\\/g, "/").toLowerCase();
        for (const f of layout.files) {
          if (norm.endsWith(f.path.replace(/\\/g, "/").toLowerCase())) {
            liveMtimeRef.current.set(f.id, now);
            fresh.push({ t: now, color: f.color, id: tickSeq.current++ });
          }
        }
      }
      if (fresh.length) setTicks((prev) => [...prev, ...fresh].slice(-120));
    })
      .then((u) => {
        if (disposed) u();
        else un = u;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      if (un) un();
    };
  }, [layout]);

  const w2s = useCallback((x: number, y: number) => {
    const c = cam.current;
    const v = view.current;
    return { x: v.w / 2 + (x - c.x) * c.s, y: v.h / 2 + (y - c.y) * c.s };
  }, []);
  const s2w = useCallback((sx: number, sy: number) => {
    const c = cam.current;
    const v = view.current;
    return { x: (sx - v.w / 2) / c.s + c.x, y: (sy - v.h / 2) / c.s + c.y };
  }, []);

  const fitScale = useCallback(() => {
    const v = view.current;
    const r = layout?.maxR ?? 600;
    return Math.min(1.2, (Math.min(v.w, v.h) / 2 / r) * 0.92);
  }, [layout]);

  // Resize + fit once the layout is ready.
  useEffect(() => {
    const wrap = wrapRef.current;
    const cv = canvasRef.current;
    if (!wrap || !cv) return;
    const apply = () => {
      const r = wrap.getBoundingClientRect();
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      view.current = { w: r.width, h: r.height, dpr };
      cv.width = Math.max(1, Math.round(r.width * dpr));
      cv.height = Math.max(1, Math.round(r.height * dpr));
    };
    apply();
    cam.current = { x: 0, y: 0, s: fitScale() };
    const ro = new ResizeObserver(apply);
    ro.observe(wrap);
    return () => ro.disconnect();
  }, [fitScale]);

  // ── picking ────────────────────────────────────────────────────────────────
  const pickAt = useCallback(
    (sx: number, sy: number): Pick => {
      if (!layout) return null;
      // drones first (they sit above files)
      for (const d of dronesRef.current) {
        if (d.sx < 0) continue;
        if ((d.sx - sx) ** 2 + (d.sy - sy) ** 2 < 13 * 13)
          return { type: "agent", id: d.id };
      }
      let best: Pick = null;
      let bd = Infinity;
      for (const f of layout.files) {
        const p = w2s(f.x, f.y);
        const dx = p.x - sx;
        const dy = p.y - sy;
        const d = dx * dx + dy * dy;
        if (d < 11 * 11 && d < bd) {
          bd = d;
          best = { type: "file", id: f.id };
        }
      }
      if (best) return best;
      for (const h of layout.hubs) {
        const p = w2s(h.hx, h.hy);
        if ((p.x - sx) ** 2 + (p.y - sy) ** 2 < 22 * 22)
          return { type: "hub", id: h.projectId };
      }
      const c0 = w2s(0, 0);
      if ((c0.x - sx) ** 2 + (c0.y - sy) ** 2 < 36 * 36)
        return { type: "brain" };
      return null;
    },
    [layout, w2s],
  );

  // ── draw loop ────────────────────────────────────────────────────────────────
  useEffect(() => {
    if (!layout) return;
    const cv = canvasRef.current;
    if (!cv) return;
    const ctx = cv.getContext("2d");
    if (!ctx) return;
    t0.current = performance.now();

    const draw = () => {
      const v = view.current;
      const time = (performance.now() - t0.current) / 1000;
      ctx.setTransform(v.dpr, 0, 0, v.dpr, 0, 0);
      ctx.clearRect(0, 0, v.w, v.h);

      // faint grid
      const c = cam.current;
      const gs = 46 * c.s;
      if (gs > 6) {
        ctx.lineWidth = 1;
        ctx.strokeStyle = "rgba(255,255,255,.022)";
        const ox = (((v.w / 2 - c.x * c.s) % gs) + gs) % gs;
        const oy = (((v.h / 2 - c.y * c.s) % gs) + gs) % gs;
        for (let x = ox; x < v.w; x += gs) {
          ctx.beginPath();
          ctx.moveTo(x, 0);
          ctx.lineTo(x, v.h);
          ctx.stroke();
        }
        for (let y = oy; y < v.h; y += gs) {
          ctx.beginPath();
          ctx.moveTo(0, y);
          ctx.lineTo(v.w, y);
          ctx.stroke();
        }
      }

      const focus = focusRef.current;
      const sel2 = selRef.current;
      const hov = hoverRef.current;
      const c0 = w2s(0, 0);

      // spokes brain → hubs
      for (const h of layout.hubs) {
        const p = w2s(h.hx, h.hy);
        const on = !focus || focus === h.projectId;
        ctx.strokeStyle = on ? `${h.color}66` : "rgba(255,255,255,.05)";
        ctx.lineWidth = focus === h.projectId ? 1.6 : 1.1;
        ctx.beginPath();
        ctx.moveTo(c0.x, c0.y);
        ctx.lineTo(p.x, p.y);
        ctx.stroke();
      }

      // per-hub recency rings
      for (const h of layout.hubs) {
        if (focus && focus !== h.projectId) continue;
        const p = w2s(h.hx, h.hy);
        for (let b = BANDS.length - 1; b >= 0; b--) {
          ctx.beginPath();
          ctx.arc(p.x, p.y, BANDS[b].r * c.s, 0, 6.2832);
          ctx.strokeStyle =
            b === 0
              ? "rgba(47,224,138,.28)"
              : b === 1
                ? "rgba(77,141,255,.18)"
                : b === 2
                  ? "rgba(120,135,160,.12)"
                  : "rgba(90,100,120,.09)";
          ctx.setLineDash(b === 3 ? [3, 7] : []);
          ctx.lineWidth = b === 0 ? 1.4 : 1;
          ctx.stroke();
          ctx.setLineDash([]);
        }
      }

      // blast-radius arcs (selected file → its import/anchor neighbors)
      if (blastRef.current && sel2?.type === "file") {
        const sf = layout.byId.get(sel2.id);
        const rel = layout.adj.get(sel2.id);
        if (sf && rel) {
          const sp = w2s(sf.x, sf.y);
          for (const rid of rel) {
            const o = layout.byId.get(rid);
            if (!o) continue;
            const op = w2s(o.x, o.y);
            const mx = (sp.x + op.x) / 2;
            const my = (sp.y + op.y) / 2 - 34;
            ctx.beginPath();
            ctx.moveTo(sp.x, sp.y);
            ctx.quadraticCurveTo(mx, my, op.x, op.y);
            ctx.strokeStyle = "rgba(255,138,61,.5)";
            ctx.lineWidth = 1.3;
            ctx.stroke();
          }
        }
      }

      // files
      for (const f of layout.files) {
        const dim = focus && f.projectId !== focus;
        const p = w2s(f.x, f.y);
        const emph = sel2?.type === "file" && sel2.id === f.id;
        const lm = liveMtimeRef.current.get(f.id);
        const justChanged = lm ? Date.now() - lm < 90_000 : false;
        const isRisk = riskRef.current.has(f.id);
        const recent = (f.band <= 1 || justChanged) && !f.isMemory;
        let col = f.isMemory
          ? "#f5c518"
          : justChanged
            ? "#2fe08a"
            : f.band === 0
              ? "#8fe7ff"
              : f.band === 1
                ? f.color
                : f.band === 2
                  ? "#5c6678"
                  : "#3f4756";
        if (emph) col = "#ffffff";
        const r = f.r * (emph ? 1.9 : 1) * c.s;
        ctx.globalAlpha = dim ? 0.1 : 1;
        // recent pulse
        if (recent && !dim) {
          const pr = r + 4 + 3 * Math.sin(time * 3 + f.x * 0.05);
          ctx.beginPath();
          ctx.arc(p.x, p.y, pr, 0, 6.2832);
          ctx.strokeStyle =
            f.band === 0 || justChanged
              ? `rgba(47,224,138,${0.32 + 0.18 * Math.sin(time * 3 + f.x)})`
              : "rgba(77,141,255,.2)";
          ctx.lineWidth = 1.1;
          ctx.stroke();
        }
        // additive glow for recent
        if (recent && !dim) {
          ctx.save();
          ctx.globalCompositeOperation = "lighter";
          ctx.globalAlpha = 0.45;
          ctx.fillStyle = col;
          ctx.beginPath();
          ctx.arc(p.x, p.y, r + 4.5, 0, 6.2832);
          ctx.fill();
          ctx.restore();
        }
        // risk halo (a pending review-inbox proposal targets this node)
        if (isRisk && !dim) {
          const pr = r + 6 + 3 * Math.sin(time * 4);
          ctx.beginPath();
          ctx.arc(p.x, p.y, pr, 0, 6.2832);
          ctx.strokeStyle = `rgba(255,90,95,${0.5 + 0.3 * Math.sin(time * 4)})`;
          ctx.lineWidth = 1.5;
          ctx.stroke();
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, Math.max(1.1, r), 0, 6.2832);
        ctx.fillStyle = col;
        ctx.fill();
        if (emph) {
          ctx.beginPath();
          ctx.arc(p.x, p.y, r + 5, 0, 6.2832);
          ctx.strokeStyle = "#fff";
          ctx.lineWidth = 1.4;
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
      }

      // hover ring
      if (hov?.type === "file") {
        const f = layout.byId.get(hov.id);
        if (f) {
          const p = w2s(f.x, f.y);
          ctx.beginPath();
          ctx.arc(p.x, p.y, 10, 0, 6.2832);
          ctx.strokeStyle = "rgba(255,255,255,.9)";
          ctx.lineWidth = 1.4;
          ctx.stroke();
        }
      }

      // live agent drones — orbit the brain's hottest files, beam to the target
      const nowMs = Date.now();
      for (const d of dronesRef.current) {
        let target = d.targetId ? layout.byId.get(d.targetId) : null;
        if ((!target || nowMs - d.hopAt > 4200) && layout.hotGlobal.length) {
          const hot = layout.hotGlobal;
          d.targetId =
            hot[(Math.floor(nowMs / 4200) + d.name.length) % hot.length];
          target = layout.byId.get(d.targetId) ?? null;
          d.hopAt = nowMs;
        }
        const orbit = nowMs / 900 + d.name.length;
        const ox = target
          ? target.x + Math.cos(orbit) * 26
          : Math.cos(orbit) * 64;
        const oy = target
          ? target.y + Math.sin(orbit) * 26
          : Math.sin(orbit) * 64;
        d.x += (ox - d.x) * 0.05;
        d.y += (oy - d.y) * 0.05;
        d.trail.push({ x: d.x, y: d.y });
        if (d.trail.length > 22) d.trail.shift();
        const dp = w2s(d.x, d.y);
        d.sx = dp.x;
        d.sy = dp.y;
        if (target) {
          const tp = w2s(target.x, target.y);
          ctx.save();
          ctx.globalAlpha = 0.8;
          ctx.strokeStyle = d.color;
          ctx.lineWidth = 1.2;
          ctx.setLineDash([4, 5]);
          ctx.lineDashOffset = -time * 22;
          ctx.beginPath();
          ctx.moveTo(dp.x, dp.y);
          ctx.lineTo(tp.x, tp.y);
          ctx.stroke();
          ctx.restore();
        }
        ctx.save();
        ctx.globalCompositeOperation = "lighter";
        for (let i = 0; i < d.trail.length; i++) {
          const tpp = w2s(d.trail[i].x, d.trail[i].y);
          const fr = i / d.trail.length;
          ctx.globalAlpha = fr * 0.5;
          ctx.fillStyle = d.color;
          ctx.beginPath();
          ctx.arc(tpp.x, tpp.y, fr * 3.5, 0, 6.2832);
          ctx.fill();
        }
        ctx.globalAlpha = 0.7;
        ctx.fillStyle = d.color;
        ctx.beginPath();
        ctx.arc(dp.x, dp.y, 10, 0, 6.2832);
        ctx.fill();
        ctx.restore();
        ctx.beginPath();
        ctx.arc(dp.x, dp.y, 5, 0, 6.2832);
        ctx.fillStyle = d.color;
        ctx.fill();
        ctx.beginPath();
        ctx.arc(dp.x, dp.y, 2.2, 0, 6.2832);
        ctx.fillStyle = "#fff";
        ctx.fill();
        if (sel2?.type === "agent" && sel2.id === d.id) {
          ctx.beginPath();
          ctx.arc(dp.x, dp.y, 12, 0, 6.2832);
          ctx.strokeStyle = "#fff";
          ctx.lineWidth = 1.4;
          ctx.stroke();
        }
        ctx.fillStyle = "#e8eaed";
        ctx.font = "700 10px Manrope, system-ui, sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(d.name, dp.x, dp.y - 15);
      }

      // project sub-hubs
      for (const h of layout.hubs) {
        const p = w2s(h.hx, h.hy);
        const dim = focus && focus !== h.projectId;
        const emph = focus === h.projectId;
        const hr = 14 * c.s;
        ctx.globalAlpha = dim ? 0.35 : 1;
        // glow
        ctx.save();
        ctx.globalCompositeOperation = "lighter";
        const g = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, hr * 3.4);
        g.addColorStop(0, `${h.color}80`);
        g.addColorStop(1, `${h.color}00`);
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(p.x, p.y, hr * 3.4, 0, 6.2832);
        ctx.fill();
        ctx.restore();
        ctx.beginPath();
        ctx.arc(p.x, p.y, hr, 0, 6.2832);
        ctx.fillStyle = "#0b1018";
        ctx.fill();
        ctx.strokeStyle = h.color;
        ctx.lineWidth = emph ? 2.2 : 1.6;
        ctx.stroke();
        // initials
        ctx.fillStyle = "#eef1f6";
        ctx.font = `800 ${(12 * c.s).toFixed(0)}px Manrope, system-ui, sans-serif`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(initials(h.name), p.x, p.y);
        // label below
        ctx.fillStyle = dim ? "rgba(200,205,214,.4)" : "#c8cdd6";
        ctx.font = `700 ${Math.max(9, 11 * c.s).toFixed(0)}px Manrope, system-ui, sans-serif`;
        ctx.fillText(h.name, p.x, p.y + hr + 11);
        ctx.globalAlpha = 1;
      }

      // brain core
      const brainR = 26 * c.s;
      ctx.save();
      ctx.globalCompositeOperation = "lighter";
      const bg = ctx.createRadialGradient(
        c0.x,
        c0.y,
        0,
        c0.x,
        c0.y,
        brainR * 3.2,
      );
      bg.addColorStop(0, "rgba(77,141,255,.5)");
      bg.addColorStop(1, "rgba(77,141,255,0)");
      ctx.fillStyle = bg;
      ctx.beginPath();
      ctx.arc(c0.x, c0.y, brainR * 3.2, 0, 6.2832);
      ctx.fill();
      ctx.restore();
      const spin = time * 0.5;
      ctx.strokeStyle = "#22d3ee";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(c0.x, c0.y, brainR + 6, spin, spin + 1.7);
      ctx.stroke();
      ctx.strokeStyle = "#4d8dff";
      ctx.beginPath();
      ctx.arc(c0.x, c0.y, brainR + 6, spin + 3.14, spin + 3.14 + 1.7);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(c0.x, c0.y, brainR, 0, 6.2832);
      ctx.fillStyle = "#ffffff";
      ctx.fill();
      ctx.fillStyle = "#06070d";
      ctx.font = `800 ${(20 * c.s).toFixed(0)}px Manrope, system-ui, sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText("K", c0.x, c0.y);
      ctx.fillStyle = "#5b6373";
      ctx.font = "600 8px 'IBM Plex Mono', monospace";
      ctx.fillText("BRAIN", c0.x, c0.y + brainR + 10);

      drawMinimap(ctx, layout, v, s2w);
      raf.current = requestAnimationFrame(draw);
    };
    raf.current = requestAnimationFrame(draw);
    return () => {
      if (raf.current) cancelAnimationFrame(raf.current);
    };
  }, [layout, w2s, s2w]);

  // ── pointer interaction ──────────────────────────────────────────────────────
  const localXY = (e: React.PointerEvent | WheelEvent) => {
    const r = wrapRef.current?.getBoundingClientRect();
    return { x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0) };
  };
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    drag.current = {
      x: e.clientX,
      y: e.clientY,
      cx: cam.current.x,
      cy: cam.current.y,
      moved: false,
    };
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const { x: sx, y: sy } = localXY(e);
    const d = drag.current;
    if (d) {
      const dx = e.clientX - d.x;
      const dy = e.clientY - d.y;
      if (Math.abs(dx) + Math.abs(dy) > 4) d.moved = true;
      cam.current.x = d.cx - dx / cam.current.s;
      cam.current.y = d.cy - dy / cam.current.s;
      setTip(null);
      return;
    }
    const hit = pickAt(sx, sy);
    setHover(hit);
    if (hit?.type === "agent") {
      const d = dronesRef.current.find((x) => x.id === hit.id);
      if (d)
        setTip({
          x: e.clientX,
          y: e.clientY,
          title: d.name,
          sub: "live agent",
        });
    } else if (hit?.type === "file" && layout) {
      const f = layout.byId.get(hit.id);
      if (f)
        setTip({
          x: e.clientX,
          y: e.clientY,
          title: f.name,
          sub: `${topDirOf(f.path)} · ${fmtAge(f.mtime, layout.now)}`,
        });
    } else if (hit?.type === "hub" && layout) {
      const h = layout.hubById.get(hit.id);
      if (h)
        setTip({
          x: e.clientX,
          y: e.clientY,
          title: h.name,
          sub: `${h.fileCount} files`,
        });
    } else if (hit?.type === "brain") {
      setTip({
        x: e.clientX,
        y: e.clientY,
        title: "Koden Brain",
        sub: "all projects",
      });
    } else {
      setTip(null);
    }
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = drag.current;
    drag.current = null;
    if (!d || d.moved) return;
    const { x: sx, y: sy } = localXY(e);
    const hit = pickAt(sx, sy);
    if (!hit || hit.type === "brain") {
      setSel(null);
      setFocusPid(null);
      cam.current = { x: 0, y: 0, s: fitScale() };
      return;
    }
    if (hit.type === "hub") {
      const h = layout?.hubById.get(hit.id);
      setFocusPid(hit.id);
      setSel({ type: "hub", id: hit.id });
      if (h) {
        cam.current = { x: h.hx, y: h.hy, s: Math.max(1.0, cam.current.s) };
      }
      return;
    }
    if (hit.type === "agent") {
      setSel(hit); // drones move; don't fly the camera to them
      return;
    }
    const f = layout?.byId.get(hit.id);
    setSel(hit);
    if (f) {
      setFocusPid(f.projectId);
      cam.current = { x: f.x, y: f.y, s: Math.max(1.4, cam.current.s) };
    }
  };
  useEffect(() => {
    const cv = canvasRef.current;
    if (!cv) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = wrapRef.current?.getBoundingClientRect();
      const sx = e.clientX - (r?.left ?? 0);
      const sy = e.clientY - (r?.top ?? 0);
      const before = s2w(sx, sy);
      const s2 = Math.max(
        0.18,
        Math.min(6, cam.current.s * Math.exp(-e.deltaY * 0.0012)),
      );
      cam.current.s = s2;
      const v = view.current;
      cam.current.x = before.x - (sx - v.w / 2) / s2;
      cam.current.y = before.y - (sy - v.h / 2) / s2;
    };
    cv.addEventListener("wheel", onWheel, { passive: false });
    return () => cv.removeEventListener("wheel", onWheel);
    // s2w is stable (useCallback); re-bind harmless.
  }, [s2w]);

  // ── chrome data ──────────────────────────────────────────────────────────────
  const selFile =
    sel?.type === "file" && layout ? layout.byId.get(sel.id) : null;
  const selHub =
    sel?.type === "hub" && layout ? layout.hubById.get(sel.id) : null;
  const selAgentRec = sel?.type === "agent" ? (agentMap[sel.id] ?? null) : null;
  const related =
    selFile && layout
      ? (layout.adj.get(selFile.id) ?? [])
          .map((id) => layout.byId.get(id))
          .filter((f): f is FNode => !!f)
          .slice(0, 8)
      : [];
  const drawerAccent = selFile
    ? selFile.color
    : selHub
      ? selHub.color
      : selAgentRec
        ? droneColor(selAgentRec.status)
        : "#333";
  const drawerKind = selFile
    ? selFile.isMemory
      ? "memory"
      : "file"
    : selHub
      ? "project"
      : "agent";
  const drawerTitle = selFile
    ? selFile.name
    : selHub
      ? selHub.name
      : (selAgentRec?.name ?? "");

  if (!layout) {
    return (
      <div
        className="flex h-full w-full items-center justify-center text-sm"
        style={{ background: BG_BOT, color: "#8b93a3" }}
      >
        Connecting to the Koden index…
      </div>
    );
  }
  if (graph && graph.nodes.length === 0) {
    return (
      <div
        className="flex h-full w-full flex-col items-center justify-center gap-2 text-sm"
        style={{ background: BG_BOT, color: "#8b93a3" }}
      >
        <span style={{ color: "#eef1f6", fontWeight: 700 }}>
          No activity yet
        </span>
        <span className="font-mono text-[11px]">
          Nothing indexed. Add a project from the Brain pane (+ Add).
        </span>
      </div>
    );
  }

  return (
    <div
      ref={wrapRef}
      className="relative h-full w-full overflow-hidden"
      style={{
        background: `radial-gradient(140% 120% at 50% 44%, ${BG_TOP} 0%, ${BG_MID} 55%, ${BG_BOT} 100%)`,
        fontFamily: "Manrope, system-ui, sans-serif",
        color: "#e8eaed",
      }}
    >
      <canvas
        ref={canvasRef}
        className="absolute inset-0 block h-full w-full"
        style={{
          cursor: drag.current?.moved ? "grabbing" : "grab",
          touchAction: "none",
        }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerLeave={() => {
          setHover(null);
          setTip(null);
        }}
      />

      {/* top bar */}
      <header className="pointer-events-none absolute inset-x-0 top-0 flex h-[52px] items-center justify-between px-4">
        <div className="flex items-center gap-2.5">
          <div
            className="flex size-6 items-center justify-center rounded-[7px] text-[13px] font-extrabold"
            style={{
              background: "linear-gradient(135deg,#4d8dff,#22d3ee)",
              color: "#06070d",
              boxShadow: "0 0 16px rgba(77,141,255,.5)",
            }}
          >
            K
          </div>
          <div className="flex flex-col leading-none">
            <span
              className="text-[13.5px] font-extrabold"
              style={{ color: "#eef1f6" }}
            >
              koden{" "}
              <span style={{ color: "#5b6373", fontWeight: 600 }}>
                /{" "}
                {focusPid
                  ? (layout.hubById.get(focusPid)?.name ?? "brain")
                  : "brain"}
              </span>
            </span>
            <span
              className="font-mono text-[9.5px]"
              style={{ color: "#5b6373", letterSpacing: ".3px" }}
            >
              observatory · {layout.hubs.length} projects
            </span>
          </div>
        </div>
        <div className="pointer-events-auto flex items-center gap-2 font-mono text-[11px]">
          {agents.length > 0 ? (
            <Badge
              color="#9fe6d4"
              bg="rgba(45,224,138,.1)"
              border="rgba(45,224,138,.25)"
              dot="#2fe08a"
            >
              {agents.length} agents
            </Badge>
          ) : null}
          <Badge
            color="#bcd6ff"
            bg="rgba(77,141,255,.1)"
            border="rgba(77,141,255,.25)"
            dot="#4d8dff"
          >
            {layout.edits24h} edits · 24h
          </Badge>
          {risk.count > 0 ? (
            <Badge
              color="#ffb3b5"
              bg="rgba(255,90,95,.1)"
              border="rgba(255,90,95,.25)"
              dot="#ff5a5f"
            >
              {risk.count} risks
            </Badge>
          ) : null}
          <button
            type="button"
            onClick={() => setBlast((b) => !b)}
            className="flex items-center gap-1.5 rounded-lg border px-2.5 py-1 font-bold"
            style={{
              borderColor: blast
                ? "rgba(255,138,61,.5)"
                : "rgba(255,255,255,.1)",
              background: blast
                ? "rgba(255,138,61,.16)"
                : "rgba(255,255,255,.05)",
              color: blast ? "#ffc299" : "#aab2c0",
            }}
          >
            ◎ blast radius
          </button>
        </div>
      </header>

      {/* left rail — legend */}
      <div className="absolute left-3.5 top-16 z-10 w-[200px]">
        <div
          className="flex flex-col gap-3 rounded-[14px] border p-3"
          style={{
            background: "rgba(11,14,21,.82)",
            borderColor: "rgba(255,255,255,.08)",
            backdropFilter: "blur(12px)",
          }}
        >
          <RailSection title="Recency rings">
            {BANDS.map((b) => (
              <RailRow
                key={b.label}
                color={b.color}
                dashed={b.label === "STALE"}
              >
                {b.label.toLowerCase()}
              </RailRow>
            ))}
          </RailSection>
          <div style={{ height: 1, background: "rgba(255,255,255,.06)" }} />
          <RailSection title="Map">
            <RailRow color="#ffffff" filled>
              brain core
            </RailRow>
            <RailRow color="#8fe7ff" filled>
              recently changed
            </RailRow>
            <RailRow color="#f5c518" filled>
              memory / context
            </RailRow>
          </RailSection>
        </div>
      </div>

      {/* bottom timeline (live indicator — scrub is Phase 3) */}
      <div
        className="absolute inset-x-3.5 bottom-3.5 z-10 flex h-[52px] items-center gap-3 rounded-[14px] border px-4"
        style={{
          background: "rgba(11,14,21,.86)",
          borderColor: "rgba(255,255,255,.08)",
          backdropFilter: "blur(12px)",
        }}
      >
        <span
          className="font-mono text-[9px] uppercase tracking-wider"
          style={{ color: "#5b6373" }}
        >
          streaming
        </span>
        <div className="relative h-[34px] flex-1">
          <div
            className="absolute inset-x-0 top-1/2 h-[3px] -translate-y-1/2 rounded-full"
            style={{ background: "rgba(255,255,255,.08)" }}
          />
          {ticks.map((tk) => {
            const span = 10 * 60 * 1000;
            const end = ticks[ticks.length - 1]?.t ?? layout.now;
            const left = Math.max(
              0,
              Math.min(100, ((tk.t - (end - span)) / span) * 100),
            );
            return (
              <div
                key={tk.id}
                className="absolute top-1/2 w-[2px] -translate-x-1/2 -translate-y-1/2 rounded-[1px]"
                style={{
                  height: 14,
                  left: `${left}%`,
                  background: tk.color,
                  opacity: 0.85,
                }}
              />
            );
          })}
          <div
            className="absolute top-1/2 right-0 h-[26px] w-[3px] -translate-y-1/2 rounded-[1px]"
            style={{
              background: "#fff",
              boxShadow: "0 0 10px rgba(255,255,255,.7)",
            }}
          />
        </div>
        <span
          className="flex items-center gap-1.5 rounded-lg border px-3 py-1.5 font-mono text-[11px] font-bold"
          style={{
            borderColor: "rgba(47,224,138,.5)",
            background: "rgba(47,224,138,.14)",
            color: "#9fe6d4",
          }}
        >
          <span
            className="size-1.5 rounded-full"
            style={{ background: "#2fe08a", boxShadow: "0 0 7px #2fe08a" }}
          />
          LIVE
        </span>
      </div>

      {/* detail drawer */}
      {selFile || selHub || selAgentRec ? (
        <aside
          className="absolute right-3.5 top-16 z-20 flex w-[280px] flex-col overflow-hidden rounded-2xl border"
          style={{
            bottom: 76,
            background: "rgba(11,14,21,.92)",
            borderColor: "rgba(255,255,255,.1)",
            backdropFilter: "blur(14px)",
            boxShadow: "0 18px 50px rgba(0,0,0,.6)",
          }}
        >
          <div
            className="flex items-start justify-between gap-2 border-b p-4"
            style={{ borderColor: "rgba(255,255,255,.07)" }}
          >
            <div className="flex min-w-0 items-center gap-2.5">
              <div
                className="flex size-8 flex-none items-center justify-center rounded-[9px] text-[12px] font-extrabold"
                style={{
                  background: drawerAccent,
                  color: "#06070d",
                  boxShadow: `0 0 14px ${drawerAccent}`,
                }}
              >
                {initials(drawerTitle || "?")}
              </div>
              <div className="flex min-w-0 flex-col leading-tight">
                <span
                  className="font-mono text-[9px] uppercase tracking-wide"
                  style={{ color: "#5b6373" }}
                >
                  {drawerKind}
                </span>
                <span
                  className="break-all text-[14px] font-bold"
                  style={{ color: "#eef1f6" }}
                >
                  {drawerTitle}
                </span>
              </div>
            </div>
            <button
              type="button"
              onClick={() => {
                setSel(null);
                setFocusPid(null);
                cam.current = { x: 0, y: 0, s: fitScale() };
              }}
              className="flex size-6 flex-none items-center justify-center rounded-[7px] text-[14px]"
              style={{ background: "rgba(255,255,255,.07)", color: "#9aa0a8" }}
            >
              ✕
            </button>
          </div>
          <div className="flex flex-col gap-3.5 overflow-y-auto p-4">
            {selFile ? (
              <>
                <DrawerRow label="Path" value={selFile.path} mono />
                <DrawerRow
                  label="Last change"
                  value={fmtAge(selFile.mtime, layout.now)}
                />
                <DrawerRow
                  label="Recency"
                  value={BANDS[selFile.band].label.toLowerCase()}
                />
                {related.length ? (
                  <div className="flex flex-col gap-1.5">
                    <span
                      className="font-mono text-[9px] uppercase tracking-wide"
                      style={{ color: "#5b6373" }}
                    >
                      Related · blast radius
                    </span>
                    <div className="flex flex-col gap-1">
                      {related.map((o) => (
                        <button
                          key={o.id}
                          type="button"
                          onClick={() => {
                            setSel({ type: "file", id: o.id });
                            setBlast(true);
                          }}
                          className="flex items-center gap-2 rounded-[7px] px-2 py-1.5 text-left"
                          style={{ background: "rgba(255,255,255,.03)" }}
                        >
                          <i
                            className="size-[7px] flex-none rounded-full"
                            style={{ background: o.color }}
                          />
                          <span
                            className="truncate font-mono text-[11px]"
                            style={{ color: "#c8cdd6" }}
                          >
                            {o.name}
                          </span>
                          <span
                            className="ml-auto font-mono text-[9px]"
                            style={{ color: "#5b6373" }}
                          >
                            {topDirOf(o.path)}
                          </span>
                        </button>
                      ))}
                    </div>
                  </div>
                ) : null}
              </>
            ) : selAgentRec ? (
              <>
                <div
                  className="flex items-center gap-2 rounded-[10px] border px-3 py-2.5"
                  style={{
                    background: `${droneColor(selAgentRec.status)}1f`,
                    borderColor: `${droneColor(selAgentRec.status)}4d`,
                  }}
                >
                  <span
                    className="size-2 rounded-full"
                    style={{
                      background: droneColor(selAgentRec.status),
                      boxShadow: `0 0 8px ${droneColor(selAgentRec.status)}`,
                    }}
                  />
                  <span
                    className="text-[12.5px] font-bold"
                    style={{ color: "#e8eaed" }}
                  >
                    {selAgentRec.status}
                  </span>
                </div>
                <DrawerRow label="Role" value={selAgentRec.role} />
                <DrawerRow
                  label="Note"
                  value="Drones orbit the brain's hottest files (project→file attribution is approximate)."
                />
              </>
            ) : selHub ? (
              <>
                <div className="flex gap-2">
                  <Stat value={String(selHub.fileCount)} label="files" />
                  <Stat value={String(selHub.modules.length)} label="modules" />
                </div>
                <div className="flex flex-col gap-1.5">
                  <span
                    className="font-mono text-[9px] uppercase tracking-wide"
                    style={{ color: "#5b6373" }}
                  >
                    Modules
                  </span>
                  <div className="flex flex-wrap gap-1.5">
                    {selHub.modules.slice(0, 16).map((m) => (
                      <span
                        key={m.name}
                        className="rounded-md border px-2 py-1 font-mono text-[11px]"
                        style={{
                          borderColor: "rgba(255,255,255,.1)",
                          color: "#c8cdd6",
                          background: "rgba(255,255,255,.03)",
                        }}
                      >
                        {m.name} · {m.count}
                      </span>
                    ))}
                  </div>
                </div>
              </>
            ) : null}
          </div>
        </aside>
      ) : null}

      {/* hover tooltip */}
      {tip ? (
        <div
          className="pointer-events-none fixed z-40 -translate-x-1/2 -translate-y-[130%] rounded-lg border px-2.5 py-1.5"
          style={{
            left: tip.x,
            top: tip.y,
            background: "rgba(9,12,19,.95)",
            borderColor: "rgba(255,255,255,.13)",
            backdropFilter: "blur(8px)",
            boxShadow: "0 8px 26px rgba(0,0,0,.55)",
          }}
        >
          <div
            className="whitespace-nowrap text-[12px] font-bold"
            style={{ color: "#eef1f6" }}
          >
            {tip.title}
          </div>
          <div className="font-mono text-[9.5px]" style={{ color: "#8b93a3" }}>
            {tip.sub}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function initials(name: string): string {
  const cleaned = name.replace(/[^a-zA-Z0-9]/g, "");
  return (cleaned.slice(0, 2) || "?").replace(/^./, (c) => c.toUpperCase());
}

function drawMinimap(
  ctx: CanvasRenderingContext2D,
  layout: OLayout,
  v: { w: number; h: number },
  s2w: (x: number, y: number) => { x: number; y: number },
) {
  const W = 122;
  const Hm = 122;
  const x0 = v.w - W - 14;
  const y0 = v.h - Hm - 76;
  ctx.save();
  ctx.globalAlpha = 0.92;
  ctx.fillStyle = "rgba(9,12,19,.8)";
  roundRect(ctx, x0, y0, W, Hm, 10);
  ctx.fill();
  ctx.strokeStyle = "rgba(255,255,255,.1)";
  ctx.lineWidth = 1;
  ctx.stroke();
  const cx = x0 + W / 2;
  const cy = y0 + Hm / 2;
  const sc = (W / 2 - 12) / (layout.maxR || 600);
  for (const f of layout.files) {
    ctx.fillStyle =
      f.band === 0 ? "#8fe7ff" : f.band === 1 ? f.color : "#46506a";
    ctx.fillRect(cx + f.x * sc - 0.7, cy + f.y * sc - 0.7, 1.4, 1.4);
  }
  for (const h of layout.hubs) {
    ctx.fillStyle = h.color;
    ctx.beginPath();
    ctx.arc(cx + h.hx * sc, cy + h.hy * sc, 1.8, 0, 6.2832);
    ctx.fill();
  }
  const tl = s2w(0, 0);
  const br = s2w(v.w, v.h);
  ctx.strokeStyle = "rgba(255,255,255,.4)";
  ctx.lineWidth = 1;
  ctx.strokeRect(
    cx + tl.x * sc,
    cy + tl.y * sc,
    (br.x - tl.x) * sc,
    (br.y - tl.y) * sc,
  );
  ctx.fillStyle = "#5b6373";
  ctx.font = "600 8px 'IBM Plex Mono', monospace";
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  ctx.fillText("MAP", x0 + 8, y0 + 13);
  ctx.restore();
}
function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function Badge({
  children,
  color,
  bg,
  border,
  dot,
}: {
  children: React.ReactNode;
  color: string;
  bg: string;
  border: string;
  dot: string;
}) {
  return (
    <span
      className="flex items-center gap-1.5 rounded-lg border px-2.5 py-1"
      style={{ color, background: bg, borderColor: border }}
    >
      <span
        className="size-1.5 rounded-full"
        style={{ background: dot, boxShadow: `0 0 7px ${dot}` }}
      />
      {children}
    </span>
  );
}

function RailSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <span
        className="font-mono text-[9px] uppercase tracking-wider"
        style={{ color: "#5b6373" }}
      >
        {title}
      </span>
      <div className="mt-2 flex flex-col gap-1.5">{children}</div>
    </div>
  );
}
function RailRow({
  color,
  children,
  dashed,
  filled,
}: {
  color: string;
  children: React.ReactNode;
  dashed?: boolean;
  filled?: boolean;
}) {
  return (
    <div
      className="flex items-center gap-2 text-[11.5px]"
      style={{ color: "#aab2c0" }}
    >
      <span
        className="inline-block size-2.5 flex-none rounded-full"
        style={{
          background: filled ? color : "transparent",
          border: filled
            ? "none"
            : `1.5px ${dashed ? "dashed" : "solid"} ${color}`,
        }}
      />
      {children}
    </div>
  );
}
function DrawerRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span
        className="font-mono text-[9px] uppercase tracking-wide"
        style={{ color: "#5b6373" }}
      >
        {label}
      </span>
      <span
        className={
          mono ? "break-all font-mono text-[11px]" : "text-[13px] font-semibold"
        }
        style={{ color: "#c8cdd6" }}
      >
        {value}
      </span>
    </div>
  );
}
function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div
      className="flex-1 rounded-[9px] px-2.5 py-2"
      style={{ background: "rgba(255,255,255,.04)" }}
    >
      <div className="text-[16px] font-extrabold" style={{ color: "#e8eaed" }}>
        {value}
      </div>
      <div className="font-mono text-[9px]" style={{ color: "#6b7280" }}>
        {label}
      </div>
    </div>
  );
}
