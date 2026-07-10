import { listenFsChanged } from "@/modules/explorer/lib/watch";
import { useOrchestrationStore } from "@/modules/orchestration/store/orchestrationStore";
import { useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { type BrainGraph, brainGraph } from "./lib/bindings";

// ── Koden Brain 3D ────────────────────────────────────────────────────────────
// Faithful port of the "Koden Brain 3D" design handoff (Three.js WebGL): a
// brain-shaped neuron core at the center, every indexed project as a glowing lobe
// on a sphere around it, files as a GPU point-cloud, an orbit camera, recent-edit
// glow, live agent glows + activity feed. The design's fake generator is replaced
// with real Brain data: projects/files come from brainGraph, recency from each
// file's mtime, agents from the orchestration roster, and live edits from the real
// fs:changed event. Dark by design — not theme-following.
//
// ADR-014 (Koden Svart) SS6 asked this to at least respect the active theme's
// bg/fg/accent where feasible. Evaluated and kept as-is: every DOM overlay
// here (header, search pill, legend, detail panel, tooltip) is a glass-on-
// near-black HUD where every border/fill is an rgba(255,255,255,X) or
// rgba(0,0,0,X) mix hand-tuned against the fixed #03040a backdrop, and the
// WebGL layer below it is a categorical data palette (project lobe hue,
// node type, agent status, recency) that needs 8-12 simultaneously
// distinguishable hues no single-accent monochrome theme can supply. Swapping
// only the container bg to var(--background) would be a no-op in dark mode
// (#0a0b0b vs #03040a, imperceptible) but would make every white-glass
// overlay illegible against the Svart light port's #f2f1ec, and there's no
// way to verify a full rgba repaint without a GUI run this phase doesn't do.
// Documenting per SS6's "if truly infeasible, skip" rather than shipping an
// unverified partial repaint.

const HUB_PALETTE = [
  "#4d8dff",
  "#2fe08a",
  "#ff5a5f",
  "#ffb020",
  "#a378ff",
  "#2dd4bf",
  "#ff8a3d",
  "#8b8cff",
  "#ff5fa8",
  "#9fb2c9",
];
const TYPE_COLOR: Record<string, string> = {
  brain: "#cfe0ff",
  project: "#cfe0ff",
  file: "#7d8aa3",
  memory: "#ffb020",
};
const STATUS_COLOR: Record<string, string> = {
  working: "#2dd4bf",
  spawning: "#6096ff",
  waiting: "#ffb020",
  ready: "#2fe08a",
  done: "#2fe08a",
  error: "#ff5a5f",
};
const agentColor = (status: string) => STATUS_COLOR[status] ?? "#a378ff";

const EDIT: [number, number, number] = [0.56, 0.91, 1.0];
const RP = 60; // project-lobe radius from the brain core
const HOUR = 3600_000;
const DAY = 24 * HOUR;
const SHELL_D = [22, 42, 64, 86, 104]; // band 0..3 + memory → distance past the lobe
const SHELL_SP = [13, 18, 24, 30, 34]; // in-plane spread per shell
const BAND_LABEL = ["Active", "Today", "This week", "Stale", "Memory"];
const MAX_FILES_PER_PROJECT = 54;

function hexrgb(h: string): [number, number, number] {
  const s = h.replace("#", "");
  return [
    Number.parseInt(s.slice(0, 2), 16) / 255,
    Number.parseInt(s.slice(2, 4), 16) / 255,
    Number.parseInt(s.slice(4, 6), 16) / 255,
  ];
}
function cross(a: number[], b: number[]): [number, number, number] {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}
function norm(a: number[]): [number, number, number] {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
}
function bandOf(mtime: number, now: number): number {
  if (!mtime || mtime <= 0) return 3;
  const age = now - mtime;
  if (age < HOUR) return 0;
  if (age < DAY) return 1;
  if (age < 7 * DAY) return 2;
  return 3;
}
function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}
function initials(name: string): string {
  const c = name.replace(/[^a-zA-Z0-9]/g, "");
  return (c.slice(0, 2) || "?").replace(/^./, (ch) => ch.toUpperCase());
}
function fmtAge(mtime: number): string {
  if (!mtime || mtime <= 0) return "no recent edit";
  const d = Date.now() - mtime;
  if (d < 60_000) return "just now";
  const m = Math.floor(d / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

type N3 = [number, number, number];
type SceneNode = {
  id: string;
  type: "brain" | "project" | "file" | "memory";
  pid: number;
  label: string;
  path?: string;
  color: string;
  band: string;
  pos: N3;
  phase: number;
  lastEdit: number; // 0 = none; live-bumped by fs:changed
  glyph?: string;
  i: number; // index into nodes[]
};
type ProjMeta = { name: string; color: string; pos: N3 };
type SceneData = {
  nodes: SceneNode[];
  links: [number, number, number][];
  projMeta: ProjMeta[];
  byId: Map<string, SceneNode>;
  fileNodes: SceneNode[];
  editsToday: number;
};

function buildScene(graph: BrainGraph, now: number): SceneData {
  const projects = graph.nodes.filter((n) => n.kind === "project");
  const N = Math.max(1, projects.length);
  const ga = Math.PI * (3 - Math.sqrt(5));
  const nodes: SceneNode[] = [];
  const links: [number, number, number][] = [];
  const projMeta: ProjMeta[] = [];
  const byId = new Map<string, SceneNode>();
  const fileNodes: SceneNode[] = [];
  let editsToday = 0;

  const brain: SceneNode = {
    id: "brain",
    type: "brain",
    pid: -1,
    label: "Koden",
    color: "#cfe0ff",
    band: "Core",
    pos: [0, 0, 0],
    phase: 0,
    lastEdit: 0,
    i: 0,
  };
  nodes.push(brain);
  byId.set("brain", brain);

  projects.forEach((proj, pi) => {
    const color = HUB_PALETTE[pi % HUB_PALETTE.length];
    const y = 1 - ((pi + 0.5) / N) * 2;
    const rad = Math.sqrt(Math.max(0, 1 - y * y));
    const th = ga * pi;
    const dir: N3 = [Math.cos(th) * rad, y, Math.sin(th) * rad];
    const up = Math.abs(dir[1]) > 0.9 ? [1, 0, 0] : [0, 1, 0];
    const t1 = norm(cross(dir, up));
    const t2 = norm(cross(dir, t1));
    const hubPos: N3 = [dir[0] * RP, dir[1] * RP, dir[2] * RP];
    const hubIdx = nodes.length;
    const hub: SceneNode = {
      id: proj.id,
      type: "project",
      pid: pi,
      label: proj.label,
      color,
      band: "Project",
      pos: hubPos,
      phase: 0,
      lastEdit: 0,
      glyph: initials(proj.label),
      i: hubIdx,
    };
    nodes.push(hub);
    byId.set(hub.id, hub);
    projMeta[pi] = { name: proj.label, color, pos: hubPos };
    links.push([0, hubIdx, pi]);

    const files = graph.nodes
      .filter(
        (n) =>
          (n.kind === "file" || n.kind === "memory") &&
          n.project_id === proj.project_id,
      )
      .sort((a, b) => (b.mtime ?? 0) - (a.mtime ?? 0))
      .slice(0, MAX_FILES_PER_PROJECT);

    const placed: SceneNode[] = [hub];
    files.forEach((f, fi) => {
      const path = f.path ?? f.label;
      const isMemory = f.kind === "memory";
      const mtime = f.mtime ?? 0;
      if (mtime > 0 && now - mtime < DAY) editsToday++;
      const band = isMemory ? 4 : bandOf(mtime, now);
      const shellD = SHELL_D[band];
      const sp = SHELL_SP[band];
      const a = fi * 2.399963; // 2D golden angle → even spread
      const rr = sp * (0.55 + ((fi * 0.618033988) % 1) * 0.6);
      const ca = Math.cos(a) * rr;
      const sa = Math.sin(a) * rr;
      const pos: N3 = [
        dir[0] * (RP + shellD) + t1[0] * ca + t2[0] * sa,
        dir[1] * (RP + shellD) + t1[1] * ca + t2[1] * sa,
        dir[2] * (RP + shellD) + t1[2] * ca + t2[2] * sa,
      ];
      const idx = nodes.length;
      const node: SceneNode = {
        id: f.id,
        type: isMemory ? "memory" : "file",
        pid: pi,
        label: baseName(path),
        path,
        color,
        band: BAND_LABEL[band],
        pos,
        phase: (fi * 0.37) % 6.28,
        lastEdit: mtime,
        i: idx,
      };
      nodes.push(node);
      byId.set(node.id, node);
      fileNodes.push(node);
      // nearest already-placed node of this project → branching tree link
      let tgt = placed[0];
      let bd = Infinity;
      for (const pp of placed) {
        const dx = pp.pos[0] - pos[0];
        const dy = pp.pos[1] - pos[1];
        const dz = pp.pos[2] - pos[2];
        const d = dx * dx + dy * dy + dz * dz;
        if (d < bd) {
          bd = d;
          tgt = pp;
        }
      }
      links.push([tgt.i, idx, pi]);
      placed.push(node);
    });
  });

  return { nodes, links, projMeta, byId, fileNodes, editsToday };
}

type Agent3 = { id: string; name: string; color: string; nodeId: string };

export function BrainMapPane() {
  const [graph, setGraph] = useState<BrainGraph | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusPid, setFocusPid] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [highlightRecent, setHighlightRecent] = useState(false);
  const [, force] = useState(0);

  const agentMap = useOrchestrationStore((s) => s.agents);

  const mountRef = useRef<HTMLDivElement>(null);
  const labelsRef = useRef<HTMLDivElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);

  // Loop-readable mirrors of React state (the rAF loop can't read state).
  const focusRef = useRef<number | null>(null);
  const selRef = useRef<string | null>(null);
  const hoverRef = useRef<string | null>(null);
  const hiRef = useRef(false);
  const agents3Ref = useRef<Agent3[]>([]);
  // Imperative API the chrome calls into (set up inside the three effect).
  const apiRef = useRef<{
    focusProject: (pid: number) => void;
    selectNode: (id: string) => void;
    reset: () => void;
  } | null>(null);

  focusRef.current = focusPid;
  selRef.current = selectedId;
  hiRef.current = highlightRecent;

  useEffect(() => {
    let alive = true;
    brainGraph(200)
      .then((g) => alive && setGraph(g))
      .catch(() => alive && setGraph({ nodes: [], edges: [] }));
    return () => {
      alive = false;
    };
  }, []);

  const scene = useMemo(
    () => (graph ? buildScene(graph, Date.now()) : null),
    [graph],
  );

  // Map the live agent roster → drones assigned to the brain's hottest files.
  const agents = useMemo(() => Object.values(agentMap), [agentMap]);
  useEffect(() => {
    if (!scene) return;
    const hot =
      scene.fileNodes
        .filter((n) => n.lastEdit > 0)
        .sort((a, b) => b.lastEdit - a.lastEdit)
        .slice(0, 12)
        .map((n) => n.id) ?? [];
    const pool = hot.length
      ? hot
      : scene.fileNodes.slice(0, 12).map((n) => n.id);
    const prev = new Map(agents3Ref.current.map((a) => [a.id, a]));
    agents3Ref.current = agents.map((a, i) => {
      const ex = prev.get(a.id);
      return {
        id: a.id,
        name: a.name,
        color: agentColor(a.status),
        nodeId: ex?.nodeId ?? pool[i % Math.max(1, pool.length)] ?? "brain",
      };
    });
    force((x) => x + 1);
  }, [agents, scene]);

  // ── Three.js scene (rebuilt only when `scene` changes; agents/state read via
  // refs each frame so the roster changing never re-inits the WebGL context) ──
  useEffect(() => {
    const mount = mountRef.current;
    const labelLayer = labelsRef.current;
    if (!scene || !mount || !labelLayer) return;

    let w = mount.clientWidth || 1;
    let h = mount.clientHeight || 1;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const sceneObj = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(45, w / h, 1, 5000);
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(dpr);
    renderer.setSize(w, h);
    mount.appendChild(renderer.domElement);
    renderer.domElement.style.display = "block";

    const glowTex = makeGlowTexture();
    const world = new THREE.Group();
    sceneObj.add(world);

    // starfield
    const SN = 1100;
    const sp = new Float32Array(SN * 3);
    for (let i = 0; i < SN; i++) {
      const r = 700 + Math.random() * 900;
      const u = Math.random() * 2 - 1;
      const t = Math.random() * 6.28;
      const rr = Math.sqrt(1 - u * u);
      sp[i * 3] = Math.cos(t) * rr * r;
      sp[i * 3 + 1] = u * r;
      sp[i * 3 + 2] = Math.sin(t) * rr * r;
    }
    const sg = new THREE.BufferGeometry();
    sg.setAttribute("position", new THREE.BufferAttribute(sp, 3));
    const stars = new THREE.Points(
      sg,
      new THREE.PointsMaterial({
        color: 0x9fb2d6,
        size: 1.6,
        sizeAttenuation: false,
        transparent: true,
        opacity: 0.55,
      }),
    );
    sceneObj.add(stars);

    const core = buildCore(world, glowTex);

    // node point-cloud (sub-nodes + project hubs; brain hidden — the core IS it)
    const nodes = scene.nodes;
    const baseRGB = nodes.map((n) => hexrgb(TYPE_COLOR[n.type] ?? "#7d8aa3"));
    const n = nodes.length;
    const posArr = new Float32Array(n * 3);
    const colArr = new Float32Array(n * 3);
    const sizeArr = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      posArr[i * 3] = nodes[i].pos[0];
      posArr[i * 3 + 1] = nodes[i].pos[1];
      posArr[i * 3 + 2] = nodes[i].pos[2];
    }
    const pg = new THREE.BufferGeometry();
    pg.setAttribute("position", new THREE.BufferAttribute(posArr, 3));
    pg.setAttribute("acolor", new THREE.BufferAttribute(colArr, 3));
    pg.setAttribute("asize", new THREE.BufferAttribute(sizeArr, 1));
    const uScale = { value: h * dpr * 0.5 };
    const pmat = new THREE.ShaderMaterial({
      uniforms: { uScale },
      vertexShader:
        "attribute vec3 acolor;attribute float asize;varying vec3 vC;uniform float uScale;void main(){vC=acolor;vec4 mv=modelViewMatrix*vec4(position,1.0);gl_PointSize=asize*uScale/(-mv.z);gl_Position=projectionMatrix*mv;}",
      fragmentShader:
        "varying vec3 vC;void main(){float d=length(gl_PointCoord-vec2(0.5));float a=smoothstep(0.5,0.0,d);if(a<=0.01)discard;gl_FragColor=vec4(vC,a);}",
      transparent: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    const points = new THREE.Points(pg, pmat);
    world.add(points);

    // project + agent glow sprites
    const projGlows = scene.projMeta.map((m) => {
      const s = new THREE.Sprite(
        new THREE.SpriteMaterial({
          map: glowTex,
          color: new THREE.Color(m.color),
          blending: THREE.AdditiveBlending,
          transparent: true,
          depthWrite: false,
          opacity: 0.7,
        }),
      );
      s.position.set(m.pos[0], m.pos[1], m.pos[2]);
      s.scale.set(44, 44, 1);
      world.add(s);
      return s;
    });
    const agentGlowPool: THREE.Sprite[] = [];
    const ensureAgentGlows = (count: number) => {
      while (agentGlowPool.length < count) {
        const s = new THREE.Sprite(
          new THREE.SpriteMaterial({
            map: glowTex,
            blending: THREE.AdditiveBlending,
            transparent: true,
            depthWrite: false,
            opacity: 0.9,
          }),
        );
        s.scale.set(40, 40, 1);
        world.add(s);
        agentGlowPool.push(s);
      }
    };

    // links
    const links = scene.links;
    const lpos = new Float32Array(links.length * 6);
    const lcol = new Float32Array(links.length * 6);
    const rebuildLines = () => {
      const focus = focusRef.current;
      for (let i = 0; i < links.length; i++) {
        const [a, b, pid] = links[i];
        const A = nodes[a].pos;
        const B = nodes[b].pos;
        lpos[i * 6] = A[0];
        lpos[i * 6 + 1] = A[1];
        lpos[i * 6 + 2] = A[2];
        lpos[i * 6 + 3] = B[0];
        lpos[i * 6 + 4] = B[1];
        lpos[i * 6 + 5] = B[2];
        const c = hexrgb(scene.projMeta[pid]?.color ?? "#6096ff");
        const f = focus != null ? (pid === focus ? 0.9 : 0.04) : 0.42;
        for (let k = 0; k < 2; k++) {
          lcol[i * 6 + k * 3] = c[0] * f;
          lcol[i * 6 + k * 3 + 1] = c[1] * f;
          lcol[i * 6 + k * 3 + 2] = c[2] * f;
        }
      }
      lineGeo.attributes.position.needsUpdate = true;
      lineGeo.attributes.color.needsUpdate = true;
    };
    const lineGeo = new THREE.BufferGeometry();
    lineGeo.setAttribute("position", new THREE.BufferAttribute(lpos, 3));
    lineGeo.setAttribute("color", new THREE.BufferAttribute(lcol, 3));
    rebuildLines();
    const lines = new THREE.LineSegments(
      lineGeo,
      new THREE.LineBasicMaterial({
        vertexColors: true,
        transparent: true,
        opacity: 0.5,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      }),
    );
    world.add(lines);

    // labels (DOM)
    labelLayer.innerHTML = "";
    const projLabelEls = scene.projMeta.map((m) => {
      const d = document.createElement("div");
      d.className = "klabel";
      d.style.cssText +=
        ";position:absolute;left:0;top:0;pointer-events:none;white-space:nowrap;transform:translate(-50%,-50%);font-family:'Inter Variable',system-ui,sans-serif;font-weight:700;font-size:11px;color:#aab2c0;text-shadow:0 1px 6px rgba(0,0,0,.9);";
      d.textContent = m.name;
      labelLayer.appendChild(d);
      return d;
    });

    // camera (spherical orbit + eased target/radius)
    const DEFAULT_R = 340;
    const cam = { theta: 0.6, phi: 1.15, radius: DEFAULT_R };
    const tgt = { x: 0, y: 0, z: 0 };
    let tgtT: { x: number; y: number; z: number } | null = null;
    let radiusT = DEFAULT_R;
    let dragging = false;
    let drag: {
      x: number;
      y: number;
      th: number;
      ph: number;
      moved: boolean;
    } | null = null;

    const camPos = (): N3 => {
      const r = cam.radius;
      return [
        tgt.x + r * Math.sin(cam.phi) * Math.cos(cam.theta),
        tgt.y + r * Math.cos(cam.phi),
        tgt.z + r * Math.sin(cam.phi) * Math.sin(cam.theta),
      ];
    };

    apiRef.current = {
      focusProject: (pid: number) => {
        const m = scene.projMeta[pid];
        if (!m) return;
        tgtT = { x: m.pos[0] * 1.3, y: m.pos[1] * 1.3, z: m.pos[2] * 1.3 };
        radiusT = 150;
        setFocusPid(pid);
        setSelectedId(null);
      },
      selectNode: (id: string) => {
        const nd = scene.byId.get(id);
        if (!nd) return;
        tgtT = { x: nd.pos[0], y: nd.pos[1], z: nd.pos[2] };
        radiusT = Math.min(radiusT, 120);
        setSelectedId(id);
        setFocusPid(nd.pid >= 0 ? nd.pid : null);
      },
      reset: () => {
        tgtT = { x: 0, y: 0, z: 0 };
        radiusT = DEFAULT_R;
        setFocusPid(null);
        setSelectedId(null);
      },
    };

    const v = new THREE.Vector3();
    const pickNode = (sx: number, sy: number): SceneNode | null => {
      let best: SceneNode | null = null;
      let bd = Infinity;
      for (const nd of nodes) {
        if (nd.type === "brain") continue;
        v.set(nd.pos[0], nd.pos[1], nd.pos[2]);
        v.applyMatrix4(world.matrixWorld);
        v.project(camera);
        if (v.z > 1) continue;
        const x = (v.x * 0.5 + 0.5) * w;
        const y = (-v.y * 0.5 + 0.5) * h;
        const dx = x - sx;
        const dy = y - sy;
        const d = dx * dx + dy * dy;
        const thr = nd.type === "project" ? 16 : 7;
        if (d < thr * thr && d < bd) {
          bd = d;
          best = nd;
        }
      }
      return best;
    };

    const localXY = (e: PointerEvent) => {
      const r = renderer.domElement.getBoundingClientRect();
      return { x: e.clientX - r.left, y: e.clientY - r.top };
    };
    const onDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      dragging = true;
      drag = {
        x: e.clientX,
        y: e.clientY,
        th: cam.theta,
        ph: cam.phi,
        moved: false,
      };
      mount.style.cursor = "grabbing";
    };
    const onMove = (e: PointerEvent) => {
      const { x: sx, y: sy } = localXY(e);
      if (dragging && drag) {
        const dx = e.clientX - drag.x;
        const dy = e.clientY - drag.y;
        if (Math.abs(dx) + Math.abs(dy) > 4) drag.moved = true;
        cam.theta = drag.th - dx * 0.005;
        cam.phi = Math.max(0.25, Math.min(2.9, drag.ph - dy * 0.005));
        const tip = tipRef.current;
        if (tip) tip.style.display = "none";
        return;
      }
      const nd = pickNode(sx, sy);
      hoverRef.current = nd?.id ?? null;
      mount.style.cursor = nd ? "pointer" : "grab";
      const tip = tipRef.current;
      if (tip) {
        if (!nd) tip.style.display = "none";
        else {
          tip.style.display = "block";
          tip.style.left = `${e.clientX}px`;
          tip.style.top = `${e.clientY - 6}px`;
          tip.innerHTML = `<div style="font-family:'Commit Mono','JetBrains Mono',ui-monospace,monospace;font-size:12px;font-weight:700;color:#eef1f6;white-space:nowrap;">${escapeHtml(nd.label)}</div><div style="font-family:'Commit Mono','JetBrains Mono',ui-monospace,monospace;font-size:9.5px;color:#7d8492;margin-top:2px;text-transform:uppercase;letter-spacing:.5px;">${escapeHtml(nd.type === "project" ? "project lobe" : nd.band)}</div>`;
        }
      }
    };
    const onUp = (e: PointerEvent) => {
      const d = drag;
      dragging = false;
      drag = null;
      mount.style.cursor = "grab";
      if (!d || d.moved) return;
      const { x: sx, y: sy } = localXY(e);
      const nd = pickNode(sx, sy);
      if (!nd) {
        apiRef.current?.reset();
        return;
      }
      if (nd.type === "project") apiRef.current?.focusProject(nd.pid);
      else apiRef.current?.selectNode(nd.id);
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      radiusT = Math.max(
        70,
        Math.min(900, radiusT * Math.exp(e.deltaY * 0.0012)),
      );
    };
    const dom = renderer.domElement;
    dom.addEventListener("pointerdown", onDown);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    dom.addEventListener("wheel", onWheel, { passive: false });

    const onResize = () => {
      w = mount.clientWidth || 1;
      h = mount.clientHeight || 1;
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
      uScale.value = h * dpr * 0.5;
    };
    const ro = new ResizeObserver(onResize);
    ro.observe(mount);

    // agents re-target among the hot files every few seconds (alive, like the
    // design's hop) — we know an agent's roster entry, not its exact file.
    const hopTimer = window.setInterval(() => {
      const hot = scene.fileNodes
        .filter((nd) => nd.lastEdit > 0)
        .sort((a, b) => b.lastEdit - a.lastEdit)
        .slice(0, 12);
      const pool = hot.length ? hot : scene.fileNodes.slice(0, 12);
      if (!pool.length) return;
      for (const a of agents3Ref.current) {
        a.nodeId = pool[Math.floor(Math.random() * pool.length)].id;
      }
      force((x) => x + 1);
    }, 3500);

    // ── frame loop ───────────────────────────────────────────────────────────
    const clock = new THREE.Clock();
    let raf = 0;
    let lastFocus = focusRef.current;
    const tmp = new THREE.Vector3();
    const draw = () => {
      const dt = Math.min(0.05, clock.getDelta());
      const time = clock.elapsedTime;
      const focus = focusRef.current;
      const sel = selRef.current;
      const hover = hoverRef.current;
      const hi = hiRef.current;
      const now = Date.now();

      if (focus !== lastFocus) {
        lastFocus = focus;
        rebuildLines();
      }
      if (!dragging && focus == null && !sel) cam.theta += dt * 0.5 * 0.28;
      if (tgtT) {
        tgt.x += (tgtT.x - tgt.x) * 0.09;
        tgt.y += (tgtT.y - tgt.y) * 0.09;
        tgt.z += (tgtT.z - tgt.z) * 0.09;
      }
      cam.radius += (radiusT - cam.radius) * 0.09;
      const cp = camPos();
      camera.position.set(cp[0], cp[1], cp[2]);
      camera.lookAt(tgt.x, tgt.y, tgt.z);

      core.animate(time, dt);
      stars.rotation.y += dt * 0.005;

      // node buffers
      const agentByNode = new Map<string, Agent3>();
      for (const a of agents3Ref.current) agentByNode.set(a.nodeId, a);
      for (let i = 0; i < nodes.length; i++) {
        const nd = nodes[i];
        if (nd.type === "brain") {
          sizeArr[i] = 0;
          colArr[i * 3] = colArr[i * 3 + 1] = colArr[i * 3 + 2] = 0;
          continue;
        }
        const base = baseRGB[i];
        let r = base[0];
        let g = base[1];
        let b = base[2];
        let size = nd.type === "project" ? 15 : nd.type === "memory" ? 4 : 3;
        let glow = 0;
        if (nd.lastEdit) {
          const age = now - nd.lastEdit;
          if (age < DAY) glow = 1 - age / DAY;
        }
        if (glow > 0) {
          const pulse = 0.7 + 0.3 * Math.sin(time * 3 + nd.phase);
          const mm = glow * pulse;
          r += (EDIT[0] - r) * 0.7 * glow;
          g += (EDIT[1] - g) * 0.7 * glow;
          b += (EDIT[2] - b) * 0.7 * glow;
          size += glow * 3.2 * pulse;
          r *= 1 + 0.4 * mm;
          g *= 1 + 0.4 * mm;
          b *= 1 + 0.4 * mm;
        }
        const ag = agentByNode.get(nd.id);
        if (ag) {
          const arc = hexrgb(ag.color);
          const p = 0.55 + 0.45 * Math.sin(time * 5 + nd.phase);
          r = arc[0] * (0.8 + 0.7 * p);
          g = arc[1] * (0.8 + 0.7 * p);
          b = arc[2] * (0.8 + 0.7 * p);
          size = Math.max(size, 9) + 5 * p;
        }
        let dim = 1;
        if (focus != null && nd.pid !== focus) dim = 0.12;
        if (hi && !ag && !(glow > 0)) dim = Math.min(dim, 0.13);
        if (nd.id === hover || nd.id === sel) {
          size += 3;
          r = Math.min(1, r * 1.4 + 0.1);
          g = Math.min(1, g * 1.4 + 0.1);
          b = Math.min(1, b * 1.4 + 0.1);
          dim = 1;
        }
        colArr[i * 3] = r * dim;
        colArr[i * 3 + 1] = g * dim;
        colArr[i * 3 + 2] = b * dim;
        sizeArr[i] = size;
      }
      pg.attributes.acolor.needsUpdate = true;
      pg.attributes.asize.needsUpdate = true;

      for (let i = 0; i < projGlows.length; i++) {
        let op = 0.6;
        if (focus != null) op = focus === i ? 0.95 : 0.08;
        if (hi) op *= 0.6;
        projGlows[i].material.opacity =
          op * (0.85 + 0.15 * Math.sin(time * 1.6 + i));
      }
      // agent glows
      const a3 = agents3Ref.current;
      ensureAgentGlows(a3.length);
      for (let i = 0; i < agentGlowPool.length; i++) {
        const s = agentGlowPool[i];
        const a = a3[i];
        if (!a) {
          s.visible = false;
          continue;
        }
        const nd = scene.byId.get(a.nodeId);
        if (!nd) {
          s.visible = false;
          continue;
        }
        s.visible = true;
        s.material.color = new THREE.Color(a.color);
        s.position.set(nd.pos[0], nd.pos[1], nd.pos[2]);
        const p = 0.5 + 0.5 * Math.sin(time * 5 + i);
        s.material.opacity = 0.55 + 0.4 * p;
        s.scale.setScalar(34 + 10 * p);
      }

      // project labels (project 3D → screen)
      for (let i = 0; i < projLabelEls.length; i++) {
        const m = scene.projMeta[i];
        tmp.set(m.pos[0], m.pos[1], m.pos[2]);
        tmp.applyMatrix4(world.matrixWorld);
        tmp.project(camera);
        const el = projLabelEls[i];
        const show = !hi && (focus == null || focus === i);
        if (tmp.z > 1 || !show) {
          el.style.display = "none";
        } else {
          const x = (tmp.x * 0.5 + 0.5) * w;
          const y = (-tmp.y * 0.5 + 0.5) * h;
          el.style.display = "block";
          el.style.opacity = focus != null && focus !== i ? "0.25" : "1";
          el.style.transform = `translate(-50%,-50%) translate(${x.toFixed(1)}px,${(y + 20).toFixed(1)}px)`;
        }
      }

      renderer.render(sceneObj, camera);
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(raf);
      clearInterval(hopTimer);
      ro.disconnect();
      dom.removeEventListener("pointerdown", onDown);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      dom.removeEventListener("wheel", onWheel);
      apiRef.current = null;
      labelLayer.innerHTML = "";
      renderer.dispose();
      glowTex.dispose();
      if (dom.parentNode) dom.parentNode.removeChild(dom);
    };
  }, [scene]);

  // Live file changes (fs:changed) → bump the node's lastEdit so it glows.
  useEffect(() => {
    if (!scene) return;
    let un: (() => void) | null = null;
    let disposed = false;
    listenFsChanged((paths) => {
      const now = Date.now();
      for (const path of paths) {
        const norm0 = path.replace(/\\/g, "/").toLowerCase();
        for (const f of scene.fileNodes) {
          if (
            f.path &&
            norm0.endsWith(f.path.replace(/\\/g, "/").toLowerCase())
          )
            f.lastEdit = now;
        }
      }
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
  }, [scene]);

  const selNode = selectedId && scene ? scene.byId.get(selectedId) : null;
  const selAgent =
    selNode && scene
      ? agents3Ref.current.find((a) => a.nodeId === selNode.id)
      : null;
  const focusName =
    focusPid != null && scene ? (scene.projMeta[focusPid]?.name ?? null) : null;

  const onSearchKey = (e: React.KeyboardEvent) => {
    if (e.key !== "Enter" || !scene) return;
    const t = search.trim().toLowerCase();
    if (!t) return;
    const hub = scene.nodes.find(
      (nd) => nd.type === "project" && nd.label.toLowerCase().includes(t),
    );
    if (hub) {
      apiRef.current?.focusProject(hub.pid);
      return;
    }
    const m = scene.nodes.find(
      (nd) => nd.type !== "project" && nd.label.toLowerCase().includes(t),
    );
    if (m) apiRef.current?.selectNode(m.id);
  };

  if (!scene) {
    return (
      <div
        className="flex h-full w-full items-center justify-center text-sm"
        style={{ background: "#03040a", color: "#8b93a3" }}
      >
        Connecting to the Koden index…
      </div>
    );
  }
  if (graph && graph.nodes.length === 0) {
    return (
      <div
        className="flex h-full w-full flex-col items-center justify-center gap-2 text-sm"
        style={{ background: "#03040a", color: "#8b93a3" }}
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
      className="relative h-full w-full overflow-hidden"
      style={{
        background: "#03040a",
        cursor: "grab",
        fontFamily: "'Inter Variable', system-ui, sans-serif",
      }}
    >
      <div ref={mountRef} className="absolute inset-0" />
      <div
        ref={labelsRef}
        className="pointer-events-none absolute inset-0 overflow-hidden"
        style={{ zIndex: 6 }}
      />
      <div
        ref={tipRef}
        className="pointer-events-none fixed left-0 top-0"
        style={{
          zIndex: 30,
          display: "none",
          transform: "translate(-50%,-130%)",
          background: "rgba(10,13,20,.92)",
          border: "1px solid rgba(255,255,255,.12)",
          backdropFilter: "blur(8px)",
          padding: "7px 11px",
          borderRadius: 9,
          boxShadow: "0 8px 26px rgba(0,0,0,.5)",
        }}
      />
      {/* vignette */}
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          zIndex: 5,
          background:
            "radial-gradient(120% 120% at 50% 48%, transparent 50%, rgba(2,3,8,.6) 100%)",
        }}
      />

      {/* top bar */}
      <header
        className="pointer-events-none absolute inset-x-0 top-0 flex h-14 items-center justify-between px-4"
        style={{ zIndex: 20 }}
      >
        <div className="pointer-events-auto flex items-center gap-2.5">
          <div
            className="flex size-[26px] items-center justify-center rounded-full text-[13px] font-extrabold"
            style={{
              background:
                "radial-gradient(circle at 38% 32%, #ffffff, #bcd3ff 70%, #6f96ff)",
              color: "#0a0c10",
              boxShadow: "0 0 18px rgba(120,160,255,.6)",
            }}
          >
            K
          </div>
          <div className="flex flex-col leading-none">
            <span
              className="text-[14px] font-extrabold"
              style={{ color: "#eef1f6" }}
            >
              koden
            </span>
            <span
              className="font-mono text-[9.5px]"
              style={{ color: "#5f6675" }}
            >
              brain · live index
            </span>
          </div>
        </div>

        <div
          className="pointer-events-auto absolute left-1/2 top-1/2 flex w-[280px] -translate-x-1/2 -translate-y-1/2 items-center gap-2 rounded-[9px] border px-3 py-1.5"
          style={{
            background: "rgba(255,255,255,.05)",
            borderColor: "rgba(255,255,255,.1)",
            backdropFilter: "blur(8px)",
          }}
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="#5f6675"
            strokeWidth="2.2"
            aria-hidden="true"
          >
            <circle cx="11" cy="11" r="7" />
            <path d="M21 21l-4.3-4.3" />
          </svg>
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={onSearchKey}
            placeholder="Find a file or project…"
            className="w-full border-none bg-transparent text-[13px] outline-none"
            style={{ color: "#e8eaed" }}
          />
        </div>

        <div className="pointer-events-auto flex items-center gap-3 font-mono text-[11px]">
          {agents.length > 0 ? (
            <span
              className="flex items-center gap-1.5 rounded-full border px-2.5 py-1"
              style={{
                color: "#9ad9c0",
                background: "rgba(45,212,191,.1)",
                borderColor: "rgba(45,212,191,.25)",
              }}
            >
              <span
                className="size-[7px] rounded-full"
                style={{ background: "#2dd4bf", boxShadow: "0 0 7px #2dd4bf" }}
              />
              {agents.length} agents live
            </span>
          ) : null}
          <span style={{ color: "#7d8492" }}>
            {scene.editsToday} edits · 24h
          </span>
          <button
            type="button"
            onClick={() => setHighlightRecent((x) => !x)}
            className="rounded-lg border px-2.5 py-1.5 font-sans font-bold"
            style={{
              borderColor: highlightRecent
                ? "rgba(143,231,255,.5)"
                : "rgba(255,255,255,.12)",
              background: highlightRecent
                ? "rgba(143,231,255,.16)"
                : "rgba(255,255,255,.05)",
              color: highlightRecent ? "#bdeeff" : "#aab2c0",
            }}
          >
            Highlight 24h
          </button>
        </div>
      </header>

      {/* hint pill */}
      <div
        className="pointer-events-none absolute left-1/2 -translate-x-1/2"
        style={{ top: 64, zIndex: 18 }}
      >
        <div
          className="rounded-full border px-3 py-1 font-mono text-[10.5px]"
          style={{
            color: "#7a818c",
            background: "rgba(12,15,22,.55)",
            borderColor: "rgba(255,255,255,.08)",
            backdropFilter: "blur(4px)",
          }}
        >
          {focusName
            ? `Focused on ${focusName} · drag to orbit · click empty space to pull back`
            : "Click a lobe to dive in · drag to orbit · scroll to zoom · cyan = AI working now"}
        </div>
      </div>

      {/* legend */}
      <div
        className="absolute bottom-4 left-4 flex flex-col gap-1.5 rounded-[11px] border px-3 py-2.5"
        style={{
          zIndex: 18,
          background: "rgba(12,15,22,.6)",
          borderColor: "rgba(255,255,255,.08)",
          backdropFilter: "blur(6px)",
        }}
      >
        <span
          className="font-mono text-[9.5px] uppercase tracking-wider"
          style={{ color: "#5f6675" }}
        >
          Map key
        </span>
        <LegendRow color="#6096ff" glow label="Project lobe" />
        <LegendRow color="#7d8aa3" label="File / node" />
        <LegendRow color="#8fe7ff" glow label="Edited <24h" />
        <LegendRow color="#2dd4bf" glow label="Agent working now" />
      </div>

      {/* right column: detail + live feed */}
      <div
        className="pointer-events-none absolute right-3.5 flex w-[300px] flex-col gap-3"
        style={{ top: 68, bottom: 14, zIndex: 19 }}
      >
        {selNode && selNode.type !== "project" ? (
          <aside
            className="pointer-events-auto flex-none overflow-hidden rounded-2xl border"
            style={{
              background: "rgba(11,14,21,.88)",
              borderColor: "rgba(255,255,255,.1)",
              backdropFilter: "blur(14px)",
              boxShadow: "0 18px 50px rgba(0,0,0,.6)",
            }}
          >
            <div
              className="border-b p-4"
              style={{ borderColor: "rgba(255,255,255,.07)" }}
            >
              <div className="flex items-center justify-between">
                <span
                  className="font-mono text-[9.5px] uppercase tracking-wide"
                  style={{ color: "#5f6675" }}
                >
                  {selNode.type === "memory" ? "Memory node" : "Source file"}
                </span>
                <button
                  type="button"
                  onClick={() => apiRef.current?.reset()}
                  className="flex size-6 items-center justify-center rounded-[7px] text-[14px]"
                  style={{
                    background: "rgba(255,255,255,.07)",
                    color: "#9aa0a8",
                  }}
                >
                  ✕
                </button>
              </div>
              <div className="mt-2.5 flex items-center gap-2.5">
                <div
                  className="flex size-8 flex-none items-center justify-center rounded-[9px] text-[12px] font-extrabold"
                  style={{
                    background: TYPE_COLOR[selNode.type],
                    color: "#0a0c10",
                    boxShadow: `0 0 16px ${TYPE_COLOR[selNode.type]}`,
                  }}
                >
                  {initials(selNode.label)}
                </div>
                <span
                  className="break-all font-mono text-[14px] font-semibold leading-tight"
                  style={{ color: "#eef1f6" }}
                >
                  {selNode.label}
                </span>
              </div>
            </div>
            <div className="flex flex-col gap-3 p-4">
              {selAgent ? (
                <div
                  className="flex items-center gap-2.5 rounded-[10px] border px-3 py-2.5"
                  style={{
                    background: "rgba(45,212,191,.1)",
                    borderColor: "rgba(45,212,191,.28)",
                  }}
                >
                  <span
                    className="size-2 flex-none rounded-full"
                    style={{
                      background: selAgent.color,
                      boxShadow: `0 0 8px ${selAgent.color}`,
                    }}
                  />
                  <div className="flex flex-col leading-tight">
                    <span
                      className="text-[12.5px] font-bold"
                      style={{ color: "#d6fff4" }}
                    >
                      {selAgent.name}
                    </span>
                    <span
                      className="font-mono text-[10.5px]"
                      style={{ color: "#7fd9c6" }}
                    >
                      working near this file
                    </span>
                  </div>
                </div>
              ) : null}
              <DetailRow
                label="Project"
                value={scene.projMeta[selNode.pid]?.name ?? "—"}
                dot={selNode.color}
              />
              <div className="flex gap-2.5">
                <DetailCol label="Layer" value={selNode.band} />
                <DetailCol
                  label="Last edit"
                  value={selAgent ? "editing now" : fmtAge(selNode.lastEdit)}
                  color={
                    selAgent ||
                    (selNode.lastEdit && Date.now() - selNode.lastEdit < DAY)
                      ? "#8fe7ff"
                      : "#c2c7cf"
                  }
                />
              </div>
              {selNode.path ? (
                <div className="flex flex-col gap-1">
                  <span
                    className="font-mono text-[9.5px] uppercase tracking-wide"
                    style={{ color: "#5f6675" }}
                  >
                    Path
                  </span>
                  <span
                    className="break-all font-mono text-[11px]"
                    style={{ color: "#c2c7cf" }}
                  >
                    {selNode.path}
                  </span>
                </div>
              ) : null}
            </div>
          </aside>
        ) : null}

        <aside
          className="pointer-events-auto flex min-h-0 flex-col overflow-hidden rounded-2xl border"
          style={{
            background: "rgba(11,14,21,.82)",
            borderColor: "rgba(255,255,255,.08)",
            backdropFilter: "blur(12px)",
          }}
        >
          <div
            className="border-b px-4 py-3"
            style={{ borderColor: "rgba(255,255,255,.06)" }}
          >
            <span
              className="font-mono text-[9.5px] uppercase tracking-wider"
              style={{ color: "#7fd9c6" }}
            >
              ● Live agent activity
            </span>
          </div>
          <div className="flex flex-col gap-0.5 overflow-y-auto p-1.5">
            {agents.length === 0 ? (
              <div
                className="px-3 py-4 font-mono text-[11px]"
                style={{ color: "#5f6675" }}
              >
                No agents running. Start a session in a terminal.
              </div>
            ) : (
              agents3Ref.current.map((a) => {
                const nd = scene.byId.get(a.nodeId);
                return (
                  <button
                    key={a.id}
                    type="button"
                    onClick={() => nd && apiRef.current?.selectNode(nd.id)}
                    className="flex items-center gap-2.5 rounded-[10px] px-2.5 py-2 text-left"
                  >
                    <span
                      className="size-2.5 flex-none rounded-full"
                      style={{
                        background: a.color,
                        boxShadow: `0 0 9px ${a.color}`,
                      }}
                    />
                    <div className="flex min-w-0 flex-col leading-tight">
                      <span
                        className="text-[12.5px] font-bold"
                        style={{ color: "#e8eaed" }}
                      >
                        {a.name}
                      </span>
                      <span
                        className="truncate font-mono text-[10px]"
                        style={{ color: "#8b9099", maxWidth: 210 }}
                      >
                        near {nd?.label ?? "—"}
                      </span>
                    </div>
                  </button>
                );
              })
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}

// ── brain neuron core (ported verbatim; procedural, no fake data) ──────────────
function buildCore(world: THREE.Group, glowTex: THREE.Texture) {
  const mb = (seed: number) => {
    let a = seed | 0;
    return () => {
      a = (a + 0x6d2b79f5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  };
  const rng = mb(7321);
  const group = new THREE.Group();
  world.add(group);
  const glow = new THREE.Sprite(
    new THREE.SpriteMaterial({
      map: glowTex,
      color: new THREE.Color(0x7aa0ff),
      blending: THREE.AdditiveBlending,
      transparent: true,
      depthWrite: false,
      opacity: 0.4,
    }),
  );
  glow.scale.set(120, 120, 1);
  group.add(glow);
  const net = new THREE.Group();
  net.rotation.set(0.16, 0, 0.1);
  group.add(net);

  const pts: N3[] = [];
  const dim: number[] = [];
  const wrinkle = (x: number, y: number, z: number) =>
    1 +
    0.05 * Math.sin(6 * x + 3 * z) +
    0.05 * Math.sin(5 * y - 4 * z) +
    0.045 * Math.sin(7 * z + 2 * x) +
    0.035 * Math.sin(9 * x * y + 4 * z);
  const N = 1500;
  const ga = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < N; i++) {
    const yy = 1 - ((i + 0.5) / N) * 2;
    const rr = Math.sqrt(Math.max(0, 1 - yy * yy));
    const th = ga * i;
    const dx = Math.cos(th) * rr;
    const dy = yy;
    const dz = Math.sin(th) * rr;
    const wv = wrinkle(dx, dy, dz);
    let x = dx * 22 * wv;
    let y = dy * 17 * wv;
    const z = dz * 27 * wv;
    x += Math.sign(dx) * 1.7;
    if (y > 0) y -= 5.2 * Math.exp(-(x * x) / 13);
    if (y < 0) y *= 0.72;
    pts.push([x, y, z]);
    dim.push(1);
  }
  for (let k = 0; k < 70; k++) {
    const t = k / 70;
    const a = rng() * 6.28;
    const rad = 3.6 * (1 - t * 0.55) * (0.6 + 0.6 * rng());
    const yb = -13 - t * 17;
    pts.push([Math.cos(a) * rad, yb, 6 + Math.sin(a) * rad * 1.1]);
    dim.push(0.8);
  }
  for (let k = 0; k < 150; k++) {
    const yy = 1 - ((k + 0.5) / 150) * 2;
    const rr = Math.sqrt(Math.max(0, 1 - yy * yy));
    const th = ga * k;
    const wv = 1 + 0.08 * Math.sin(yy * 16);
    pts.push([
      Math.cos(th) * rr * 9 * wv,
      -9 + yy * 6.5,
      -19 + Math.sin(th) * rr * 8 * wv,
    ]);
    dim.push(0.85);
  }
  const M = pts.length;
  const cpos = new Float32Array(M * 3);
  const baseArr = new Float32Array(M * 3);
  const phaseArr = new Float32Array(M);
  const colArr = new Float32Array(M * 3);
  for (let i = 0; i < M; i++) {
    cpos[i * 3] = pts[i][0];
    cpos[i * 3 + 1] = pts[i][1];
    cpos[i * 3 + 2] = pts[i][2];
    const vv = dim[i] * (0.8 + 0.2 * rng());
    baseArr[i * 3] = (0.5 + 0.12 * rng()) * vv;
    baseArr[i * 3 + 1] = (0.68 + 0.1 * rng()) * vv;
    baseArr[i * 3 + 2] = 1.0 * vv;
    phaseArr[i] = rng() * 6.28;
  }
  const cgeo = new THREE.BufferGeometry();
  cgeo.setAttribute("position", new THREE.BufferAttribute(cpos, 3));
  cgeo.setAttribute("color", new THREE.BufferAttribute(colArr, 3));
  const cluster = new THREE.Points(
    cgeo,
    new THREE.PointsMaterial({
      size: 2.3,
      sizeAttenuation: true,
      transparent: true,
      opacity: 0.95,
      vertexColors: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      map: glowTex,
    }),
  );
  net.add(cluster);

  const segs: [number, number][] = [];
  for (let i = 0; i < M; i += 3) {
    let bd = Infinity;
    let bj = -1;
    for (let j = 0; j < M; j++) {
      if (j === i) continue;
      const dx = pts[i][0] - pts[j][0];
      const dy = pts[i][1] - pts[j][1];
      const dz = pts[i][2] - pts[j][2];
      const d = dx * dx + dy * dy + dz * dz;
      if (d < bd && d > 0.2) {
        bd = d;
        bj = j;
      }
    }
    if (bj >= 0) segs.push([i, bj]);
  }
  const S = segs.length;
  const lpos = new Float32Array(S * 6);
  const lcol = new Float32Array(S * 6);
  const lphase = new Float32Array(S);
  for (let s = 0; s < S; s++) {
    const [i, j] = segs[s];
    lpos[s * 6] = pts[i][0];
    lpos[s * 6 + 1] = pts[i][1];
    lpos[s * 6 + 2] = pts[i][2];
    lpos[s * 6 + 3] = pts[j][0];
    lpos[s * 6 + 4] = pts[j][1];
    lpos[s * 6 + 5] = pts[j][2];
    lphase[s] = rng() * 6.28;
  }
  const lgeo = new THREE.BufferGeometry();
  lgeo.setAttribute("position", new THREE.BufferAttribute(lpos, 3));
  lgeo.setAttribute("color", new THREE.BufferAttribute(lcol, 3));
  const web = new THREE.LineSegments(
    lgeo,
    new THREE.LineBasicMaterial({
      vertexColors: true,
      transparent: true,
      opacity: 0.5,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    }),
  );
  net.add(web);

  const animate = (time: number, dt: number) => {
    glow.material.opacity = 0.34 + 0.1 * Math.sin(time * 1.6);
    glow.scale.setScalar(118 + 12 * Math.sin(time * 1.6));
    net.rotation.y += dt * 0.12;
    for (let i = 0; i < M; i++) {
      const tw = 0.55 + 0.45 * Math.sin(time * 2.2 + phaseArr[i]);
      const spark =
        Math.max(0, Math.sin(time * 1.3 + phaseArr[i] * 5.0) - 0.86) * 7.0;
      const b = tw + spark;
      colArr[i * 3] = Math.min(1.4, baseArr[i * 3] * b);
      colArr[i * 3 + 1] = Math.min(1.4, baseArr[i * 3 + 1] * b);
      colArr[i * 3 + 2] = Math.min(1.5, baseArr[i * 3 + 2] * b);
    }
    cluster.geometry.attributes.color.needsUpdate = true;
    for (let s = 0; s < S; s++) {
      const pulse = 0.5 + 0.5 * Math.sin(time * 2.6 + lphase[s]);
      const b = 0.12 + 0.8 * pulse;
      lcol[s * 6] = 0.42 * b;
      lcol[s * 6 + 1] = 0.72 * b;
      lcol[s * 6 + 2] = 1.0 * b;
      lcol[s * 6 + 3] = 0.42 * b;
      lcol[s * 6 + 4] = 0.72 * b;
      lcol[s * 6 + 5] = 1.0 * b;
    }
    web.geometry.attributes.color.needsUpdate = true;
  };

  return { animate };
}

function makeGlowTexture(): THREE.Texture {
  const c = document.createElement("canvas");
  c.width = 128;
  c.height = 128;
  const x = c.getContext("2d");
  if (x) {
    const g = x.createRadialGradient(64, 64, 0, 64, 64, 64);
    g.addColorStop(0, "rgba(255,255,255,1)");
    g.addColorStop(0.25, "rgba(255,255,255,.5)");
    g.addColorStop(1, "rgba(255,255,255,0)");
    x.fillStyle = g;
    x.fillRect(0, 0, 128, 128);
  }
  return new THREE.CanvasTexture(c);
}

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c,
  );
}

function LegendRow({
  color,
  label,
  glow,
}: {
  color: string;
  label: string;
  glow?: boolean;
}) {
  return (
    <div
      className="flex items-center gap-2 text-[11.5px] font-medium"
      style={{ color: "#b8bdc6" }}
    >
      <span
        className="inline-block size-2.5 rounded-full"
        style={{
          background: color,
          boxShadow: glow ? `0 0 9px ${color}` : undefined,
        }}
      />
      {label}
    </div>
  );
}
function DetailRow({
  label,
  value,
  dot,
}: {
  label: string;
  value: string;
  dot?: string;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span
        className="font-mono text-[9.5px] uppercase tracking-wide"
        style={{ color: "#5f6675" }}
      >
        {label}
      </span>
      <span
        className="flex items-center gap-1.5 text-[13px] font-bold"
        style={{ color: "#dfe2e6" }}
      >
        {dot ? (
          <i
            className="size-2.5 rounded-full"
            style={{ background: dot, boxShadow: `0 0 7px ${dot}` }}
          />
        ) : null}
        {value}
      </span>
    </div>
  );
}
function DetailCol({
  label,
  value,
  color,
}: {
  label: string;
  value: string;
  color?: string;
}) {
  return (
    <div className="flex flex-1 flex-col gap-1">
      <span
        className="font-mono text-[9.5px] uppercase tracking-wide"
        style={{ color: "#5f6675" }}
      >
        {label}
      </span>
      <span
        className="text-[13px] font-semibold"
        style={{ color: color ?? "#c2c7cf" }}
      >
        {value}
      </span>
    </div>
  );
}
