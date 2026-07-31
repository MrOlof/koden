# ADR-012 — Symbol-granular graph (durable identity + call edges)

Status: **Deferred (demand-driven)** — decided 2026-07-11 by Claude as Kosta's
pre-approved proxy. Rationale: the largest brain investment since ADR-006 with
zero usage signal yet; file-granular impact over-approximates but never misses,
and both gauntlets showed agents get correct context without symbol edges.
**Build triggers (any one re-opens this):** (1) a real agent session measurably
wastes context because file-level impact was too coarse (multi-hundred-line
files dominating a blast radius); (2) rename tooling is requested; (3) memory
anchors demonstrably break on within-file refactors often enough to annoy.
Rollout technique when triggered: NorrGit's shadow-parity gate (old + new
resolver side by side, Jaccard ≥ 0.99 before cutover).

Originally Proposed 2026-07-07, drafted during the overnight NorrGit-parity
round as the first of two capabilities that are architecture changes, not steals.

## Context

The brain's code graph is **file-granular**: `code_nodes` records defs (name,
kind, position) but the edge table (`code_edges`) links *files* via import
resolution. Everything downstream — impact analysis (now depth-annotated and
bidirectional, `454fecc`), detect_changes dependents, the Brain Map — inherits
file granularity.

NorrGit (the in-house sibling, `Beefcapone/norrgit`) is symbol-granular:

- **Durable symbol identity**: PK + computed key
  `{language, normalizedPath, fqn, kind, signatureHash}`, reconciled across
  reparse by a 4-pass matcher (exact key → same-file body+range → global
  FQN+sigHash → new), with rename detection rewriting the path segment before
  reconcile. Identity survives edits, so anything keyed on a symbol (history,
  run ledgers, memory anchors) survives too.
- **Call/import edges between symbols**, giving symbol-level impact ("who calls
  this function"), context (1-hop neighborhood), and plan-only rename.

What file granularity costs us, concretely:

1. Impact says "these files depend on the file defining X", not "these
   functions call X". In a 500-line module the blast radius is over-approximated
   by everything else in the file.
2. Memory-note anchors bind to files/lines; a symbol-keyed anchor would survive
   refactors that move code within or across files.
3. No principled rename support (our lexical tier is an over-approximation).

## Decision (proposed)

Adopt **durable symbol identity + symbol edges as a new derived layer**, keeping
the file layer as-is (it stays correct and is what freshness/secrets/FTS key on):

- New derived tables `symbol_identity` (durable id + computed key) and
  `symbol_edges` (caller → callee, resolved within the project), rebuilt
  incrementally per changed file, with the same provable full-rebuild ==
  incremental convergence property `code_edges` has.
- Reconciler: start with a 2-pass matcher (exact key → global FQN+sigHash);
  NorrGit's same-file body+range pass and rename pre-pass are upgrades once the
  basic layer proves out.
- Resolution: tree-sitter gives us defs and call *sites*; cross-file resolution
  can start import-scoped (only resolve calls to names imported from resolved
  files) — honest partial coverage beats a wrong global guess. Unresolved calls
  are recorded as unresolved, never dropped silently.
- Impact/context/detect_changes gain an optional symbol tier; file tier remains
  the default until measured trustworthy (precision-gate discipline: hand-label
  a fixture corpus of call edges, floor the resolution precision).

## Cost / risk

- The single biggest brain investment since ADR-006 — est. multiple verified
  rounds. Touches worker (per-file pipeline), store (migrations, SCHEMA_VERSION
  bump), and every consumer wanting the symbol tier.
- Rust has no off-the-shelf TypeEnv like NorrGit's JS/TS/Python resolvers; our
  per-language resolution quality will vary and must be measured per language,
  not assumed.
- First-index cost grows (call-site extraction + resolution). Must stay inside
  the §12.1 SLA after the parallel-hash work, or the layer goes behind a
  default-off flag until it does.

## Alternatives

- **Do nothing**: file granularity is honest and cheap; impact over-approximates
  but never misses. Viable if agents mostly consume gist + search.
- **Wire NorrGit in as MCP** for symbol queries instead of building: zero Rust
  cost, but adds a Node runtime + stdio hop to every Koden install, splits the
  index into two stores with two freshness models, and its output bypasses our
  secrets gate (NorrGit has none). Rejected as the *default* path; fine as a
  power-user opt-in alongside.
