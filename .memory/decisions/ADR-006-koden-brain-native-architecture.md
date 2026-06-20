# ADR-006: Koden Brain — native in-process architecture (Code + Brain)

Status: **Accepted (founding architecture)** — 2026-06-20. Canonical record for Koden Brain.
Supersedes **ADR-005** (which wrapped Conductr as a managed child). Decision reversed after a
16-agent research pass (mine Conductr + industry prior art + Rust-crate survey + native design):
**Koden Brain is built NATIVE in Rust, in-process** — Conductr is the *idea source*, not a dependency.

> **Branding:** "Koden Brain" (aka Code + Brain) everywhere. Conductr never appears to a Koden user
> and is no longer a runtime component at all. Conductr remains Kosta's separate upstream project; we
> borrowed its proven mechanisms and reimplemented them natively.

## Context

Wrapping Conductr (ADR-005) meant a Node subprocess + an MCP boundary bolted onto a Rust/Tauri app —
"splashing it on top," which fragments the product. The goal is a unique, unified tool where the same
engine that answers the Brain pane's search also feeds every agent's context. Since Koden's backend is
Rust and has no in-process Node, the only way to get true unity ("one binary, no subprocess") is to
**build the brain natively in Rust**. That also unlocks two upgrades Conductr can't easily make: a real
**tree-sitter AST** code graph (vs Conductr's regex/string extraction) and a **warm, GUI-resident,
incrementally-fresh** index (vs cold per-CLI rehash).

Verified Koden primitives this reuses: the `usage/poll.rs::spawn_poller` worker-thread template (named
`std::thread`, fail-open, started from `lib.rs .setup()`); `agent_detect` already emits `koden:agent-signal`
homogenized across claude/codex/gemini/glm; the `--append-system-prompt` prompt-file channel
(`~/.koden/agent-<id>.txt`) already injects context vendor-agnostically; `secrets.rs` keyring; `notify` +
`ignore` crates already present.

## Decision

**One Rust module tree `src-tauri/src/modules/brain/`, one GUI-resident worker thread**, registered like
every other Koden subsystem (`.manage(BrainState)`, spawned from `.setup()` after the usage poller,
fail-open, never blocks first paint). The worker listens directly to `koden:agent-signal` + a brain-owned
recursive file watcher, folding both into one internal `BrainEvent` spine. `pty → cwd → project` resolves
via the PTY leaf map + the workspace registry's root-prefix match.

### Stack (locked 2026-06-20)
- **Storage/search: SQLite via `rusqlite` (bundled + FTS5)** — ONE inspectable file unifying FTS5 BM25 +
  AST graph tables + memory notes + fingerprint manifest + ledger. Behind a `SearchIndex` trait so
  **tantivy** can swap in later without schema churn.
- **AST: tree-sitter** with **TS/JS + Rust grammars in v1** (Python/Go added on demand; all other
  languages still get lexical search). Grammar versions pinned to the core `LANGUAGE_VERSION` range; CI
  smoke-parses one fixture per language.
- **Tokenizer (ported from Conductr):** camelCase/PascalCase/digit-boundary split that keeps the whole
  token AND the parts + additive both-forms stemming + a 50-word stoplist — applied identically to code
  and notes so identifier retrieval holds.
- **Ranking (borrowed):** BM25 K1=1.2/B=0.75, IDF=`log(1+(N-df+0.5)/(df+0.5))`; **RRF k=60** fusion but
  with a first-class per-leg weight param (dropping Conductr's duplicate-the-list hack); multiplicative
  recency re-rank; deterministic id tie-break.
- **Freshness: `blake3`** per-file content hash + sorted aggregate as the PRIMARY signal for ALL projects
  (collapses Conductr's git/no-git branch; git HEAD via the existing subprocess is an optional fast-path
  only — no `git2`/`gix`). Brain-owned **recursive `notify` watcher** (the existing `fs/watch.rs` is
  NonRecursive/per-open-dir, unsuitable) reusing its SKIP_DIRS + debounce constants. `ignore::WalkBuilder`
  for gitignore-aware initial population.
- **LLM (only token-spending path): `reqwest`+`rustls` + keyring `koden-ai`** for an opt-in, **default-OFF**,
  budgeted `reflect` call on the daemon's OWN key. `tauri::async_runtime::block_on` for the rare call (no
  tokio `time` feature).
- **`tauri-plugin-dialog`** (net-new) for the first-boot folder picker.
- **Semantic embeddings: DEFERRED** behind a default-OFF `semantic` cargo feature (does not compile into
  v1). Only the `VectorStore`/`Embedder` trait seams + `embedderId` header land now.

### Storage model (locked)
- **Canonical source = git-committed + MegaSync-portable:** the project registry + per-project memory
  notes live in a committed source folder (shareable team memory, travels between machines), with
  **root-relative portable paths** (Kosta syncs via MegaSync — absolute paths would break on machine #2).
- **Derived cache = local-only:** the SQLite index lives under `app_local_data_dir()/koden/brain/`
  (rebuildable, gitignored-by-location, survives `git clean`, atomic multi-layer updates). It does NOT
  travel — a second machine cold-builds it from the committed source on first run.
- Native naming throughout — NO `.conductr`/`.rulesync` artifacts. (Exact folder names = open Q below.)

### Phased build plan

| Phase | Goal | Gate |
|---|---|---|
| **P0 — Warm lexical brain** | `brain/` tree + worker (poll.rs template) + registry + SQLite/FTS5 store + ported tokenizer + RRF + `ignore` population + `brain_search`/`brain_index_status`/`brain_list_projects` commands + minimal pane. No tree-sitter, no LLM. | Cold start warms all projects without blocking first paint; `brain_search` returns BM25+RRF hits across code+notes <150ms; zero network/tokens. |
| **P1 — Freshness + memory** | Recursive `notify` watcher + blake3 incremental delta index; native memory store (serde_yaml frontmatter → `MemoryNote` into FTS5); native-notes seed importer (~/.claude/.codex/.gemini, lossless); ONE `MemoryProposal` queue (human-gated, gitignored) + deterministic doctor; Brain-pane memory cards + review inbox; 3-step setup wizard (folder picker). | Out-of-band edit reindexes only changed file within one debounce; seeded corpus searchable zero-token; a doctor finding → proposal the user approves/rejects. |
| **P2 — tree-sitter AST graph (XL, differentiator)** | Core + TS/JS/Rust grammars (pinned); per-language `.scm` queries for defs/imports/refs/calls + scope tables; module resolution (tsconfig paths, package exports, Cargo members); typed forward+reverse adjacency persisted to graph tables; incremental re-parse + relink; `brain_code_graph`/`brain_code_impact` (tiered AST-confident vs lexical-candidate)/`brain_neighbors`; validate memory anchors against AST. | `code_impact` returns AST reverse-import+reference closure tiered above lexical; incremental relink == full rebuild (property test). |
| **P3 — Gist injection (payoff)** | Port ContextPack layered fail-open assembly + intent planner + per-layer caps + proportional trim (always keep freshness line); cold-start query synthesis (KODEN_SESSION→project, agent name→intent, git HEAD+changed files, recent files, top notes); `brain_build_gist`/`brain_write_gist` extending `App.tsx`'s `--append-system-prompt` file; confidence gate (thin pack when signal weak); injection toast. | Fresh agent pane gets a relevant token-bounded gist via existing channel, zero tokens to build; **re-launch with unchanged code/notes → byte-identical gist (prompt-cache-safe).** |
| **P4 — Budgeted reflect + crash-resume** | LLM reflect (bounded digest, own-key, serde-validated, fail-open) behind a hard PRE-FLIGHT budget (default $0=off; "only ever PROPOSES"); resume Tier 1 (events-only journal `~/.koden/resume/<sessionKey>.jsonl`, key=cwd+agent+persisted pane uuid); Tier 2 (`claude --resume` when captured, clean fallback). | No key → spends nothing, deterministic-only; with key+ceiling → call blocked pre-flight when over; mid-call crash doesn't leak the spent counter. |
| **P5 — DEFERRED semantic** | Trait seams + `embedderId` only. fastembed-rs + hnsw behind off-by-default `semantic` feature; enable only with key + visible budget. | Deferred by decision; revisit only if lexical proves insufficient AND size/budget headroom exists. |

**Sequencing insight:** P0+P1 give a working keyless zero-token brain (search + memory). P3 delivers the
"unified" payoff (same query path feeds agents). P2 is the marquee upgrade but biggest single piece. P4/P5
are guarded extras.

## Consequences

- **Reuse-heavy on Koden's side** (worker template, event spine, injection channel, keyring, notify/ignore)
  but the brain engine itself is genuinely new native Rust. Net-new deps: `rusqlite` (bundled+FTS5),
  `tree-sitter` + 3 grammars, `blake3`, `serde_yaml`, `tauri-plugin-dialog`.
- **Top risks:** (1) **prompt-cache busting** — the gist sits in the cacheable prefix; a per-turn-mutating
  gist costs ~90% input / ~80% latency. MUST be fingerprint-keyed/byte-stable (P3 gate). (2) binary
  size/compile time (tree-sitter C grammars + bundled SQLite on an LTO-fat profile) — SQLite-over-tantivy
  + 3 grammars + strip mitigate. (3) tree-sitter ABI drift — pin grammar versions, CI smoke test.
  (4) incremental graph relink correctness — maintain reverse adjacency, property-test vs full rebuild.
  (5) two watchers (existing NonRecursive fs/watch + new recursive brain watcher) double-watching →
  inotify exhaustion on Linux — clear ownership split. (6) registry path portability — store root-relative
  paths (MegaSync). (7) own-key budget overshoot — check-reserve-call-reconcile ordering.
- **Open decisions (non-blocking for P0):** exact canonical folder names (`<root>/.koden-brain/` registry +
  `<project>/.koden-memory/*.md` notes proposed); whether registered projects MAY live outside the root
  (registry already authorizes home+launch dir, so technically yes); reflect cadence/budget defaults
  (recommend default-OFF, manual-trigger-only); resume sessionKey source (is there a stable persisted pane
  uuid in `orchestrationStore`, and is Claude session-id reachable from the bus for Tier-2?); whether the
  Rust worker `app.listen()`s directly vs webview-forwarded (direct preferred); whether any external daemon
  ever consumes the `brain_*` command API or it's strictly GUI-resident in-process.
