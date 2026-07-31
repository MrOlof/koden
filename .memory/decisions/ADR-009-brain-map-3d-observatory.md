# ADR-009: Brain Map rebuilt as "Koden Brain 3D" (Three.js, real data)

Status: Accepted — 2026-06-24 (3D is the sole Brain Map; no 2D toggle)

## Context

Kosta brought a Claude Design handoff zip (`AI Brain Visualizer-handoff`) for the Brain
Map (`src/modules/brain/BrainMapPane.tsx`), which was an SVG radial directory-tree. The
bundle had five variants. We iterated: first built the **2D "Observatory"** (the README's
primary, a canvas radial map) across three commits, then Kosta chose the **3D** variant
("build 3D and follow the zip"). Final call: **3D only, no 2D toggle.**

## Decision

Replace BrainMapPane with a **faithful port of `Koden Brain 3D.dc.html`** — a real Three.js
WebGL scene — wired to real Brain data where the design used a fake generator.

- **Dependency added:** `three` (~150 KB gz) + `@types/three`. There is no lighter way to do
  real WebGL 3D; the 3D look was explicitly requested, so the dep is justified (overrides the
  default "no new deps" lean).
- **Ported verbatim (procedural, data-free):** the brain neuron core — a 1500-point fibonacci
  cloud with wrinkle/longitudinal-fissure/hemisphere-separation shaping + brainstem +
  cerebellum, a nearest-neighbour synapse web, twinkle+spark animation, glow-sprite halo.
- **Real-data adaptation:** projects (`brainGraph` kind=project) become glowing lobes on a
  fibonacci sphere (radius RP=60); each project's real files fan into tangent-plane **recency
  shells** (mtime → active<1h / today<24h / week / stale; memory in an outer shell), each
  linked to its nearest already-placed node (branching tree). Capped at 54 files/project.
- **Render (ported):** one GPU point-cloud via a custom `ShaderMaterial` (depth-attenuated
  round sprites), per-node recent-edit glow decaying over 24h, project + agent glow sprites,
  DOM project labels projected 3D→screen each frame, spherical **orbit camera** (eased
  target/radius, auto-rotate when idle), screen-space picking, wheel zoom.
- **Live data:** agents from `useOrchestrationStore.agents` glow on the brain's hottest files
  and re-target every 3.5 s (honest approximation — we know the roster entry, not the file);
  real `fs:changed` (`listenFsChanged`) bumps a file's `lastEdit` so it lights up. Chrome:
  `N agents live` + `edits·24h` badges, search (Enter → focus/select), Highlight-24h dim,
  detail panel + live agent feed.

Alternatives rejected: a 2D/3D toggle (two render paths to maintain — Kosta said 3D only);
keeping the SVG map (superseded). The 2D Observatory work is preserved in git history
(commits `73754af`, `42d2aa0`) if it's ever wanted back.

## Consequences

- **Requires WebGL/a GPU** — it's real 3D. Degrades to nothing if WebGL is unavailable (no 2D
  fallback now). Worth a `gl`-context guard + empty-state if that ever bites headless/RDP.
- **Vite re-optimizes** on first run after adding `three`; a "failed to resolve three" error in
  the pane means the dev server needs one restart.
- **Dropped vs the 2D build:** blast-radius mode, risk halos (proposals), and the timeline
  scrub — none are in the 3D design. The live recent-edit glow was kept (the 3D design has it).
  Any of those could be grafted into 3D later.
- **Honest gaps (unchanged):** agent→file is an approximation (orbit hot files); a freshly
  indexed project clusters in one shell until the watcher re-stamps mtimes.
- **Still UNVERIFIED visually** — tsc + biome clean, but canvas/WebGL needs eyes + a GPU; layout
  constants (RP, SHELL_D/SP, point sizes, rotate speed, camera radius) are first-pass and will
  likely need tuning. Commit `da8b8cd`.
