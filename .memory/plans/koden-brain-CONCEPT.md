# Koden Brain — Conceptual & End-to-End Flow Specification

> **Purpose.** This document explains *what* Koden Brain (the "Librarian") is and *how it flows* inside the
> Koden terminal — at an industry-technical level — so it can be handed to any AI/engineer as a reference,
> and so the algorithmic decision points are exposed for you to tune or replace.
>
> **Scope.** Concept + data/control flow + algorithms + rationale. *Not* a build/integration checklist —
> that is `koden-brain-EXECUTION_PLAN.md`. Architecture of record: `ADR-006`.
>
> **Convention.** `[DP-n]` marks a **Decision Point** — a place where the algorithm/policy is a deliberate
> choice with alternatives, i.e. where you may want to introduce your own approach.

---

## 0. Product promise (the bar every feature is judged against)

> **Koden Brain makes every agent you launch start already understanding your project — within seconds,
> with no file-pasting, no forced cloud dependency, and without breaking the agent's prompt cache.**

Corollaries that gate scope:
- If a feature does not measurably improve that promise, it is not v1.
- **Thin/empty context beats wrong context** (a hard axiom, see §7.2).
- The core works **offline and key-free**; any cloud use is opt-in and user-chosen.
- Nothing the Brain does may **block the terminal** or **leak a secret** (§7.1).

---

## 1. First principles

Koden is a terminal. The Brain is **not an app you visit** — it is an ambient subsystem that makes the
terminal's AI agents effective without the human needing to understand the code. The unifying persona is the
**Librarian**: it keeps the *Library* (your root workspace of projects) indexed, fresh, remembered, and
serves that knowledge to the agents.

Design axioms (every later decision derives from these):

1. **In-process, app-lifetime.** The Librarian is one Rust worker thread inside Koden's Tauri backend. It runs
   while Koden is open, never as an OS daemon/tray. (ADR-006, Host A.)
2. **Ambient-first.** Audience = "vibe coders" who don't read code. The Brain primarily serves the *agents*,
   not the human. The human's felt experience is "my AI gets my project" + "I can resume."
3. **Zero-token core.** Everything essential (lexical search, AST graph, freshness, resume, memory storage)
   costs no tokens and needs no key. LLM/embeddings are a *progressive enhancement*.
4. **Tiered cost.** Cheap work always runs; the paid key + the LLM judgment are spent only where they
   change the outcome (semantic recall + significance decisions).
5. **Propose-not-apply on the user's files.** The Librarian freely maintains *its own* store; it only
   *proposes* changes to the user's project files. Deletion of user content is always human-confirmed.
6. **Preserve over destroy.** Stale ≠ wrong. Default to archive/supersede, never silent delete.
7. **Autonomy behind a glass wall.** Fully autonomous (no approval prompts for its own work) but the human
   can always *see* what it did and what it spent, against a hard cap.
8. **Fail-open.** Any brain error degrades to last-good / keyless behavior; it never blocks first paint, never
   crashes the terminal, never blocks a commit.

---

## 2. System model & terminology

| Term | Definition |
|---|---|
| **Workspace (root)** | The single folder the user picks at first boot. Source of truth; holds many Projects. |
| **Project** | A directory under the root with a code corpus + its own memory. Identified by a stable id; paths stored **root-relative** (MegaSync-portable). |
| **Pane / Session** | A terminal PTY leaf. Has a `cwd` → resolves to a Project. Carries `KODEN_SESSION`. Runs a shell or an agent (claude/codex/gemini/glm). |
| **Librarian** | The in-process worker thread. Owns the event loop, the tiers, the budget, all writes to the store. |
| **Index** | Derived, rebuildable, local-only (SQLite under `app_local_data_dir`). Lexical (FTS5) + AST graph + vectors + fingerprint manifest + resume journal. |
| **Memory** | Curated knowledge notes (markdown + frontmatter). Canonical, git-committed, portable. Temporal + typed + anchored. |
| **Gist** | A token-bounded context bundle synthesized per agent launch and injected into the agent's prompt. |
| **Tier 0/1/2** | The cost classes of Librarian work (free / cheap-paid / judgment-paid). |
| **Significance gate** | The decision function that decides whether a change warrants paid work. |

---

## 3. Architecture (concept level)

```
        TERMINAL PANES (1..N, each a PTY → a Project)
        claude · codex · gemini · glm · shell
                 │ stdout byte stream            │ file writes
                 ▼                                ▼
        ┌─────────────────── EVENT SPINE ───────────────────┐
        │  agent-signal (lifecycle)   fs-changed (watcher)   │
        │  normalize → coalesce/debounce → resolve→Project   │
        └───────────────────────┬────────────────────────────┘
                                 ▼
        ┌──────────────── LIBRARIAN WORKER (1 thread) ───────┐
        │  catch-up reconcile · event loop · tier dispatch    │
        │  significance gate · budget ledger                  │
        └───┬───────────────┬───────────────┬─────────────────┘
            ▼               ▼               ▼
        TIER 0          TIER 1          TIER 2
        (free)          (key)           (key)
        blake3 Δ        embeddings      LLM significance
        FTS5 + AST      semantic        + memory reflect
            │               │               │
            ▼               ▼               ▼
        ┌─────────────── SQLite (one file) ──────────────────┐
        │ FTS5 │ AST nodes/edges │ vectors │ notes │ manifest │ resume │
        └──────────────────────┬──────────────────────────────┘
                               ▼  (same query path)
        ┌─────────── CONSUMERS ────────────┐
        │ GIST → injected into agents       │   ← the payoff (for agents)
        │ SEARCH → optional human query     │
        │ RESUME → recovery cards on boot   │   ← the payoff (for human)
        └───────────────────────────────────┘
```

The terminal's existing output stream *is* the sensor (no new hooks). The same retrieval path serves both
the human's search box and the agent's gist — one engine, two consumers.

---

## 4. Data plane — what the Brain stores, and the algorithms

### 4.1 Code index — three representations of the same corpus

**(a) Lexical (always-on, zero-token).**
- **Tokenizer** `[DP-1]`: lowercase → split on `[^A-Za-z0-9]` → camelCase/PascalCase/digit-boundary split,
  emitting **both** the whole token and the parts (`writeAiFiles` → `writeaifiles`,`write`,`ai`,`files`) →
  additive light stemming emitting **both** forms (`validation`→`validate`) → 50-word stoplist. Applied
  identically to code and notes so identifiers survive. *Alt: Porter/Snowball stemmer, ICU tokenizer.*
- **Ranking** `[DP-2]`: Okapi **BM25**, `k1=1.2`, `b=0.75`,
  `IDF = ln(1 + (N − df + 0.5)/(df + 0.5))`,
  `score = Σ IDF(qᵢ)·[f·(k1+1)] / [f + k1·(1 − b + b·|D|/avgdl)]`.
  Path field weighted ~3× (filename match should outrank a body mention). *Alt: BM25F, TF-IDF, SPLADE.*
- **Storage:** SQLite FTS5 virtual table. `[DP-3]` integration is either an FTS5 external tokenizer or a
  pre-tokenization pass (the synthetic stem/part tokens are not substrings of the input, which constrains
  the external-tokenizer route). *Alt: tantivy.*

**(b) AST graph (always-on, zero-token) — the upgrade over Conductr's regex.**
- **Parser:** tree-sitter (TS/JS + Rust v1; others fall back to lexical). Incremental re-parse on edit.
- **Nodes:** real definitions (functions, methods, classes, default/re-exports, arrow-consts). **Edges**
  `[DP-4]`: `declares · imports · references · calls · tested-by · documents · supersedes`. Stored as
  forward + reverse adjacency.
- **Module resolution** `[DP-5]`: tsconfig `paths`/`baseUrl`, `package.json` exports, Cargo workspace
  members, extension fallback. *Alt: full LSP/type-resolution (heavier, more accurate).*
- **Incremental relink:** on a changed file, delete its out-edges, re-extract, rebind inbound edges from
  other files in `O(neighbors)` via the reverse index. Correctness gate = property test "incremental ==
  full rebuild."
- **Queries:** `neighbors(symbol)`, `code_graph(BFS depth-k)`, `code_impact(symbol)` returning a **tiered**
  result — AST-confident edges vs lexical-candidate matches (tree-sitter is syntax-only; generics / dynamic
  dispatch / re-export chains can under-link, so the lexical layer is the over-approximation safety net).

**(c) Semantic (Tier 1) — needs an embedder; pluggable, the user's choice.**
- **Provider-agnostic embedder** `[DP-6]`: the user picks **local** (fastembed/ONNX, or a model via Ollama —
  no key, nothing leaves the machine) **or cloud** (OpenAI, Qwen/DashScope, Voyage, Cohere, any
  OpenAI-compatible endpoint — needs a key, code is transmitted). Same BYO philosophy as the terminal agents.
  v1 ships at least one **zero-config local default** so semantic works key-free and private out of the box;
  cloud is opt-in config. Per **chunk**, not per file.
- **`embedderId` in the index header:** switching model/provider (or its vector dimension) **forces a
  re-embed** for that project — vectors from different models are incompatible and must not be mixed. Lexical
  + AST are untouched. **Secret redaction runs before embedding regardless of provider** (§7.1); with a cloud
  embedder that redaction is the only barrier to the provider, so the denylist stays conservative.
- **Chunking** `[DP-7]`: ~40-line windows, 4-line overlap, symbol-anchored, cap ~120 chunks/file.
  *Alt: AST-node-bounded chunks (function-granular), semantic splitting.*
- **ANN** `[DP-8]`: brute-force cosine for small corpora; HNSW (`hnsw_rs`) past a threshold. *Alt: IVF, DiskANN.*
- **Re-embed only changed chunks** (blake3-gated) — this is what keeps the cost bounded.

**Hybrid fusion** `[DP-9]`: **Reciprocal Rank Fusion**, `score(d) = Σ_legs wₗ · 1/(k + rankₗ(d))`, `k=60`,
with **first-class per-leg weights** `wₗ` (our improvement over Conductr's "duplicate the list N times" hack).
Legs: path+symbol BM25, content BM25, semantic cosine. RRF fuses by rank, so the incomparable BM25/cosine
scales need no calibration. *Alt: learned-to-rank, weighted score normalization, Cross-encoder rerank.*

### 4.2 Memory — curated knowledge

- **Note schema:** markdown body + YAML frontmatter (`id`, `type`, `scope`, `provenance` (human/inferred),
  `status`, `created`, `revalidate_after?`, `supersedes?`/`superseded_by?`, `anchors[]`). Frontmatter parse
  is intentionally null-stripping (Zod-acceptance parity) `[DP-10]`.
- **Typed memory** `[DP-11]`: decision / convention / glossary / incident / reference — each with
  confidence + revalidation cadence. *Alt: free-form notes only.*
- **Anchors:** a note binds to code symbols via the AST index. If the symbol moves → re-anchor; if it
  vanishes → `broken_anchor` (a staleness signal, §6 Flow G).
- **Supersession graph:** notes link `supersedes`/`superseded_by`; cycles + dangling links are doctor checks.
- **Temporal model** `[DP-12]`: recency + access-frequency feed a multiplicative re-rank boost on retrieval;
  `revalidate_after` flips a note to "needs revalidation." *Alt: exponential decay half-life (tunable).*

### 4.3 Freshness manifest

- **Fingerprint** `[DP-13]`: **blake3** per file (fast, non-crypto). Workspace aggregate = blake3 over the
  sorted `(root-relative-path, file-hash)` list → a Merkle-style digest that changes iff any file changes,
  order-independent. *Alt: xxhash, mtime+size (weaker), git object ids (git-only).*
- **Change detection:** compare current manifest to stored → `{added, changed, removed}` set diff. Works
  for **git and non-git** projects uniformly (git HEAD is only an optional fast-path to shortcut the walk).

### 4.4 Resume journal

- **Rolling working memory** `[DP-14]`: per Project, a ring of the last *N* "meaningful" events.
  Event = `{ts, pane, agent, kind, summary?}`. "Meaningful" = task-start / user-prompt / session-end
  (filter out pure working/heartbeat). The *content* ("you were adding Stripe checkout") needs a source —
  captured user prompts vs a cheap 1-line summary `[DP-15]` (open). *Alt: full transcript snapshotting.*
- Written append-only (crash-safe; tolerant parse on read, à la the verified `subagentBus` pattern).

---

## 5. Control plane — the Librarian loop & the tier model

### 5.1 Lifecycle

1. **Spawn** from Tauri `.setup()` (after the usage poller), fail-open, `.manage(BrainState)`.
2. **Catch-up reconcile** (§6 Flow B): blake3 sweep → Tier-0 reindex of the delta. Bounded, fast.
3. **Steady-state event loop:** block on an `mpsc` of `BrainEvent`s; dispatch by tier; honor budget.
4. **Shutdown:** flush, checkpoint resume journal, drop. Nothing survives the process (Host A).

### 5.2 Event spine

- Sources: `agent-signal` (pane lifecycle, carries agent name) + `fs-changed` (recursive `notify` watcher).
- **Normalization:** into one `BrainEvent` enum. `pane → cwd → Project` resolution via the registry's
  root-prefix match.
- **Coalescing/debounce** `[DP-16]`: file events are debounced (≈150 ms quiet → flush; ≈1 s max-wait) and
  coalesced per Project so a "save-all" or a `git pull` is one delta, not 200 events. *Alt: token-bucket
  rate limiting, adaptive debounce by burst size.*

### 5.3 The three tiers (triggers + cost)

| Tier | Trigger | Work | Cost | Key |
|---|---|---|---|---|
| **0** | every coalesced file delta | blake3 Δ → incremental FTS5 + AST relink; resume journal append | $0, ms | no |
| **1** | delta contains *content* changes that survive the no-op filter | re-embed changed chunks → refresh vectors | embedding tokens, batched | yes |
| **2** | significance gate trips (§5.4) | LLM reads change digest → significance verdict → optional memory proposal | cheap-chat tokens, rare | yes |

Re-indexing (Tier 0) is so cheap that gating it would cost more than running it — so it is **never** gated.
The judgment is reserved for Tier 2.

### 5.4 The significance gate `[DP-17]` (the core "is this worth it?" algorithm)

Two-stage: a **free heuristic** decides skip / act / *escalate*; the LLM is consulted **only** on the
escalate band.

**Stage 1 — heuristic feature vector** over a coalesced delta:
- `files_changed`, `lines_churned` (added+removed)
- `structural_delta` — did the AST symbol set change (defs added/removed/renamed)? (boolean/count from §4.1b)
- `export_surface_delta` — did the public/exported API change?
- `code_ratio` — fraction of churn that is code vs comments/whitespace/strings
- `anchor_hits` — does the change touch symbols that memory notes are anchored to?
- `time_since_last_analysis`, `cost_remaining_today`

**Stage 2 — score → bands.** A transparent weighted score (so you can tune/replace) `[DP-18]`:
```
sig = w1·norm(lines_churned)
    + w2·structural_delta
    + w3·export_surface_delta
    + w4·anchor_hits
    − w5·(1 − code_ratio)            // comment-only churn suppresses
score band:  sig < LOW  → SKIP (Tier-0 only)
             LOW..HIGH   → ESCALATE to LLM (Tier-2 judges)
             sig ≥ HIGH  → ACT (analyze without asking the LLM "if")
```
*Defaults are illustrative; `wᵢ`, `LOW`, `HIGH` are tunables.* *Alt: a learned classifier; a cheaper
embedding-similarity "novelty" score; pure rule table.*

**Why an LLM at all (vs pure heuristics):** the heuristic nails the obvious 90% (3-line comment = skip; 200
lines touching exports = act). The model earns its keep on the *borderline* — "this 12-line change is
architecturally significant" / "these 3 edits are one logical refactor worth a memory note" — judgments a
counter can't make.

### 5.5 Budget enforcement `[DP-19]`

- **check → reserve → call → reconcile.** Estimate cost (chars/4 heuristic for chat; exact for embeddings)
  → reserve against a daily ledger → call → reconcile with actual usage. A crash mid-call charges the
  estimate (orphan-sweep on boot) so the counter can't leak.
- **Hard cap** (default e.g. $0/day = off until set). Exceeding downgrades to Tier-0-only and *says so*.
- **Visible meter** ("12¢ today · 2 significance calls · cap $1.00"). Autonomy behind a glass wall.
- Provider/key decoupled: Librarian uses its **own** key (OpenAI for embeddings + cheap chat), never the
  terminal agents' keys.

---

## 6. End-to-end flows

### Flow A — First boot / setup
1. Wizard: pick/create the **root**. 2. Walk (gitignore-aware, bounded) → discover Projects (markers:
`.git`, `package.json`, `Cargo.toml`, `pyproject`). 3. Seed memory from existing native notes
(`~/.claude` etc.), **verify count > 0 or fail loud** (the empty-corpus trap). 4. Build Tier-0 index for
each Project. 5. If a key is present, Tier-1 embed in the background with the budget meter visible.

### Flow B — Open Koden after time away (catch-up reconcile)
1. Worker computes the workspace blake3 aggregate; diffs vs stored manifest. 2. For each changed file:
Tier-0 incremental reindex + AST relink. 3. Significance gate evaluates the *cumulative* delta; may queue a
Tier-2 pass. 4. UI shows "Library up to date" within seconds. This is what hides the fact that nothing ran
while closed.

### Flow C — Launch an agent in a pane (gist synthesis + injection)
1. Pane resolves to Project P. 2. **Query synthesis** `[DP-20]`: with no explicit task yet, synthesize an
intent query from ambient signal — agent name → intent, recent git HEAD + changed files, recently-opened
files, top memory notes. 3. Run the **same retrieval path** as search over P. 4. **Assemble the gist**:
layered (always-keep freshness line → code skeleton → top snippets → graph neighbors → top notes), per-layer
caps, proportional trim to a token budget (`[DP-21]` calibrated chars/type heuristic — no exact cross-vendor
tokenizer). 5. **Cache-stable key**: `blake3(project_fingerprint ‖ query ‖ budget ‖ schema_version)`. If
unchanged → emit the **byte-identical** prior gist (this is non-negotiable: the gist sits in the cacheable
prompt prefix; a per-launch-mutating gist busts the agent's prompt cache, ~90% input-cost penalty). 6. Write
to the existing `~/.koden/agent-<id>.txt` and inject via `--append-system-prompt` (vendor-agnostic). 7. Toast:
"Gist injected: N files · M notes · ~Xk tokens." 8. **Confidence gate** `[DP-22]`: if ambient signal is weak,
inject a thin/empty pack rather than a speculative distractor.

### Flow D — You work; files change
Watcher → coalesce → Tier-0 (always) → no-op filter → Tier-1 re-embed changed chunks (if key) → significance
gate → maybe Tier-2. All autonomous, all within budget, no prompts.

### Flow E — A meaningful unit of work completes
Agent session-end (or a high-significance delta) → Tier-2 LLM reads a **bounded digest** (e.g. ≤60 notes ×
200 chars, ≤8 proposals) → emits **structured, Zod-validated** memory proposals → appended to the gitignored
proposal queue. **Never auto-applied.** Surfaced as review cards.

### Flow F — Crash / close → reopen (resume)
On each meaningful event the resume journal is appended (crash-safe). On boot, after reconcile, the Brain
reads the journals → renders **recovery cards** next to the restored cold tabs: "Project candle-shop — you
were adding Stripe checkout (3 events, 14:22)." Tier-2 may enrich with a one-line "what changed since."
Optional Tier-2/agent: `claude --resume <id>` when a Claude session id was captured (gated on version probe).

### Flow G — Stale-ADR curation (the write-judgment scenario)
1. **Detection (free signals):** age + passed `revalidate_after`; `broken_anchor` (ADR cites code/symbols
   the AST graph shows are gone/moved); `superseded_present` (a newer note links `supersedes` it); high churn
   in the referenced area; (LLM-only) direct contradiction by a newer note.
2. **Escalate to Tier-2** only when signals cross threshold. The model reads the ADR + current state and
   **classifies**: *still-valid / stale-but-keep-as-history / obsolete-replaceable*, and recommends a graded
   action: **archive** (default bias — keep file, mark superseded) · **supersede** · **update** · **delete**
   (rare).
3. **Boundary:** the ADR is a *user file* → the Librarian **proposes**, never silently edits/deletes.
   The human (or an agent the human tasked) applies. **Deletion always confirmed.** Bias: preserve over
   destroy — *old ≠ wrong*.
4. **Rejection sticks:** a persisted reject-signature (djb2 over scope|action|normalized-title) means a
   declined proposal does not return on the next pass.

### Flow H — Plain-language query (optional, human-facing)
Brain pane / command → same retrieval path → ranked hits across code + notes + graph neighbors. Secondary
surface (the audience rarely uses it); the agents are the primary consumers.

---

## 7. Cost, safety & autonomy matrix

| Operation | Autonomous? | Cost | Notes |
|---|---|---|---|
| Tier-0 index/graph/freshness | ✅ always | $0 | never gated |
| Resume journaling | ✅ always | $0 | crash-safe append |
| Tier-1 re-embed changed chunks | ✅ (if key, in budget) | cheap | blake3-gated, batched |
| Tier-2 significance judgment | ✅ (if key, in budget) | rare/cheap | heuristic-gated; never prompts |
| Write to **its own** memory store (mark stale, re-anchor, archive superseded) | ✅ | $0 | soft ops, preserve-bias |
| Write/edit/delete a **user project file** | ❌ propose only | — | human/agent applies; delete confirmed |
| Hard-delete memory | ❌ propose/confirm | — | archive preferred |
| Spend beyond cap | ❌ blocked | — | downgrade to Tier-0, surfaced |

---

## 8. Multi-pane / multi-agent semantics

- N panes on the same Project share **one** brain (index + memory) — agent in pane 1 and pane 3 read the same
  fresh state. The terminal grid is the crew; the Brain is the shared head.
- **SQLite concurrency** `[DP-23]`: WAL mode; **single writer** (the Librarian thread) + many readonly readers
  (command threads). Readers never block the writer. *Alt: per-Project DB shards.*
- Gist injection is per-pane (each agent gets a Project-scoped, intent-tuned gist).

---

## 9. Failure modes & degradation

- **No key:** full Tier-0 experience (lexical search, AST graph, freshness, resume, memory storage). Semantic
  + significance simply absent. First launch is never dead.
- **Corrupt index/manifest:** detected → rebuild from source (index is derived/disposable).
- **Corrupt journal/proposal JSONL:** tolerant line-parse recovers the good lines (verified pattern).
- **Embedder/key error:** Tier-1 degrades to lexical-only; logged, not fatal.
- **Worker panic:** fail-open — last-good state served; terminal unaffected.
- **Known wrinkle:** the lifecycle bus filename split (`director-bus.jsonl` writer vs `agent-bus.jsonl`
  reader) must be unified before Tier-2 resume relies on it (see EXECUTION_PLAN §0/B5).

---

## 10. Open algorithmic decision points (where you may inject your own)

Consolidated for fast scanning — each is a place the rationale is genuinely yours to set:

- `[DP-1]` tokenizer/stemmer · `[DP-2]` ranking function (BM25 vs BM25F/SPLADE) · `[DP-6]` embedder provider
  (pluggable: local model OR cloud API — user's choice; OpenAI/Qwen/etc.) · `[DP-7]` chunking strategy ·
  `[DP-8]` ANN structure · `[DP-9]` fusion (RRF vs rerank)
- `[DP-12]` temporal decay model · `[DP-13]` fingerprint hash · `[DP-16]` debounce/coalescing policy
- `[DP-17]/[DP-18]` **the significance gate** (the highest-leverage one — heuristic weights, bands, or a
  learned model) · `[DP-19]` budget/estimation strategy
- `[DP-20]` cold-start query synthesis · `[DP-21]` token budgeting · `[DP-22]` injection confidence gate
- `[DP-14]/[DP-15]` resume working-memory selection + content source · `[DP-23]` storage concurrency/sharding

---

## 11. Quick reference (for handoff to other AIs)

- **It is:** an in-process Rust worker ("Librarian") in Koden's Tauri backend; runs while the app is open.
- **It does:** keeps a per-Project index (lexical + AST + optional semantic) fresh, holds curated memory,
  injects a cache-stable gist into every agent, and journals per-Project activity for crash-resume.
- **Cost model:** Tier-0 free/always; Tier-1 (embeddings) + Tier-2 (significance LLM) only with the
  Librarian's own key, heuristic-gated, budgeted, visible.
- **Safety:** maintains its own store autonomously; proposes (never silently applies) changes to user files;
  preserve-over-destroy; deletion always confirmed.
- **Borrowed from Conductr, reimplemented native:** tokenizer, BM25, RRF, temporal memory, doctor checks,
  propose-not-apply. **Improved:** real tree-sitter AST graph (vs regex), warm incremental freshness (vs cold
  CLI rehash), first-class RRF leg weights, cache-stable gist.

---

# Part II — Hardening & Operational Spec

> Added after an external review. These turn "beautiful architecture" into "provably safe and bounded."
> The feature **boundary stays per ADR-006** (semantic + significance gate are core; Python tree-sitter is
> deferred) — i.e. we kept our scope decisions, not the reviewer's re-tiering.

## 7. Safety: secrets, gist quality

### 7.1 Secrets & sensitive-data policy (mandatory, not polish)

A workspace indexer can ingest `.env`, keys, certs, tokens, dumps, Terraform/Azure creds — and a **cloud
embedder transmits them off-machine**. Therefore, applied **before any indexing OR embedding**:

- **File denylist** (never indexed/embedded): `.env*`, `*.pem`, `*.key`, `id_rsa*`, `*.pfx`, `*.p12`,
  `.npmrc`, `.pypirc`, `*.tfstate`, `*-service-account*.json`, `credentials*`, `*.kdbx`, known cloud-cred
  files. `[DP-24]`
- **High-entropy redaction:** scan chunk text for secret-shaped tokens (Shannon entropy + provider regexes:
  `sk-…`, `gh[ps]_…`, AWS `AKIA…`, JWT, PEM blocks) and **redact before index/embed**. `[DP-25]`
- **Ignore files:** honor `.gitignore` + a Koden-specific **`.kodenignore`**; a hardcoded base denylist for
  the cases above even if un-ignored.
- **Visible & overridable:** show "excluded N files as secret-like"; allow a local-only, explicit
  "include anyway" override (never silent).
- **Never in the gist:** a detected secret is never injected into an agent prompt, period.

### 7.2 Gist quality rules (the killer feature needs guardrails)

**Axiom: thin/empty context beats wrong context.** Concretely `[DP-26]`:
- **Always include:** the freshness line (project + last-updated); never trimmed.
- **Never include:** denylisted/secret content; generated/build files; binaries; vendored deps.
- **Dedup:** collapse near-duplicate snippets (same file/symbol) to the best one.
- **Downrank:** stale memory (failed `revalidate_after`), low-confidence inferred notes, generated dirs.
- **Source mix budget:** cap proportion from code vs memory vs graph so one layer can't crowd out the others.
- **Test-vs-source:** rank source above tests for an implementation query, invert for a "how is X tested" query.
- **Inject nothing** when ambient confidence is below threshold (better a blank than a distractor).

## 8. Performance & hard limits `[DP-27]`

- **Per-file:** skip > **1 MB** for indexing by default; binary detection (NUL-byte sniff) → skip.
- **Chunks:** cap per file (~120) and per project; backpressure when watcher events storm (coalesce, drop
  duplicates — reuse Koden's existing overflow/backpressure discipline).
- **Ignored dirs (base):** `node_modules, dist, build, .next, target, .git, .venv, coverage, .turbo,
  generated`, plus gitignore.
- **Index budget:** initial-index CPU cap; max SQLite size before prune/compaction.
- **Generated-file detection:** lockfiles, minified bundles, `*.generated.*` → index-light or skip.
- **Hard rule:** one cursed repo must **degrade gracefully, never freeze the terminal**.

## 9. Internal API contract (the boundary, not the impl) `[DP-28]`

```
brain.index_project(project_id)            -> IndexStats
brain.search(project_id, query, mode)      -> Hit[]        // mode: lexical|semantic|hybrid
brain.get_symbol(project_id, symbol)       -> SymbolInfo
brain.code_impact(project_id, symbol)      -> Impact{ ast_confident[], lexical_candidates[] }
brain.make_gist(project_id, intent, budget)-> Gist{ bytes, fingerprint, sources[] }
brain.record_event(project_id, pane, evt)  -> ()
brain.get_resume_cards(project_id)         -> ResumeCard[]
brain.status(project_id)                   -> BrainStatus   // §10
brain.doctor(project_id)                   -> Finding[]
brain.rebuild(project_id) / brain.disable(project_id, on)
```
These are the only surfaces the UI / watcher / agent-launch touch — prevents a pane↔watcher↔indexer↔launch
spaghetti. Realized as `#[tauri::command]`s over the worker.

## 10. Observability — "why did it do that?"

When an agent behaves oddly the user must be able to inspect the injected context. The Brain surface exposes:
indexed/skipped counts (incl. **skipped-as-secret**), last update time + current fingerprint, search legs
used, **the raw injected gist** (viewable) + its token estimate + source files + memory notes included,
Tier 1/2 cost meter, last errors. `[DP-29]`

## 11. Operational controls & recovery

Boring, but it's what earns trust: **disable Brain** globally / per project · **rebuild** index ·
**clear** resume journal / semantic cache · **safe mode** (search only, no watcher) · **doctor** ·
**export/import memory** · **reset fingerprint**. Pairs with fail-open (§9 of Part I).

## 12. Acceptance criteria & benchmark harness

### 12.1 Measurable gates (pass/fail)

| Area | Target |
|---|---|
| First project index | usable search in **5–15 s** for a normal repo |
| Agent gist | injected **before** the agent starts responding |
| Prompt cache | same `(fingerprint, query, budget)` → **byte-identical** gist |
| Watcher | save-all / `git pull` → **one** coalesced project delta |
| Search quality | labeled target in **top 5** for benchmark queries (see caveat) |
| Crash safety | corrupt index rebuilds; corrupt journal skips bad lines |
| No-key mode | still useful: lexical + AST + resume |
| Large/cursed repo | degrades gracefully, terminal never freezes |
| Secrets | denylisted/high-entropy never indexed, embedded, or injected |

### 12.2 Benchmark fixtures `[DP-30]`

Small TS app · Rust/Tauri app · mixed TS/Rust · renamed-symbols repo · broken-imports repo · generated-files
repo · huge-ignored-dirs repo · stale-memory-notes repo · moved-files repo. Measure: indexing time, search
relevance, symbol extraction, gist usefulness, **incremental == full-rebuild** equality, prompt-cache
stability, anchor-break detection.

> **Relevance-benchmark caveat (rigor):** a suite that scores "target in top-5" on every query is *vanity*.
> It MUST include **labeled ground-truth queries** AND a **negative control** (queries that should return
> nothing / where the right answer is "not here"). Report measured-only averages + coverage, and prefer an
> honest gap to a pretty 1.0. Objective gates (index time, incremental==full, cache-stability, secret
> exclusion) need no labels; only *relevance* does.

## 13. New decision points (Part II)

`[DP-24]` secret denylist · `[DP-25]` entropy/redaction strategy · `[DP-26]` gist include/exclude/dedup rules ·
`[DP-27]` performance hard limits · `[DP-28]` internal API shape · `[DP-29]` observability surface ·
`[DP-30]` benchmark fixtures + relevance labeling.
