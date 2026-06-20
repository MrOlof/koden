# Koden Brain - Execution Plan v2 (Code + Brain, native Rust)

> **Status:** v2 - corrections folded in; ready for human peer review - 2026-06-20
> **Canonical design:** ADR-006 (`terax-workspace/.memory/decisions/ADR-006-koden-brain-native-architecture.md`).
> **How to read this doc:** Section 0 below is **authoritative** - it was ground-truthed against the real
> Koden + Conductr source (every claim quotes file:line). Where the detailed phase sections (1+) conflict
> with Section 0, **Section 0 wins**. The original adversarial review is retained as an appendix; Section 0 resolves it.

---

## 0. Corrections applied in v2 (authoritative, code-verified)

A first draft and two independent AI reviews each asserted some Koden/Conductr internals that were wrong.
Every item below was re-verified directly against the source. **These override the phase sections.**

### 0.1 Verified blockers - must clear before P0 code lands

| # | Code-confirmed fact (file:line) | Why it blocks | Fix |
|---|---|---|---|
| B1 | `Session` has **no `cwd` field** - `session.rs:43-62` holds only `_job/shell_pid/killer/writer/master`; `:114` is the spawn param, consumed by `build_command` (`:132`) and dropped | pty->cwd->project (gist P3, agent-signal tagging, resume P4) is unimplementable as written | Store `cwd` on `Session` at construction (small, but **new work, not "reuse"**) |
| B2 | `AgentSignal` is **Serialize-only**; `kind: &'static str` - `agent_detect.rs:36-39` (no `Deserialize`) | A worker `app.listen` callback can't deserialize it as written | Add `Deserialize` (and own `kind`), **or** feed the worker via in-process `mpsc` from the existing emit site (preferred - no wire-format change) |
| B3 | `PtyState.sessions` is **private** - `pty/mod.rs:21-26` (no `pub`) | Any live-cwd probe via `shell_pid` can't reach the map | Add a `pub` accessor on `PtyState` |
| B4 | Agent-spawn request carries **no leaf handle** - `SpawnTerminalRequest = {agentId, role, task, model}` (`DirectorView.tsx:41-46`); `leafId` minted at `App.tsx:891` but never put on the request | `brain_build_gist` has no `KODEN_SESSION` to resolve project/intent | Thread `leafId`/`KODEN_SESSION` onto the request at all 3 call sites |
| B5 | **Bus filename split** - writer -> `~/.koden/director-bus.jsonl` (`agent.rs:23-32`, test `:228`); reader `AgentBusBridge` tails `~/.koden/agent-bus.jsonl` (`App.tsx:374`). Rust never writes `agent-bus.jsonl` | Tier-2 `claude --resume` capture assumes reader==writer; it isn't | Resolve the split (one path end-to-end) before resume relies on it. **Pre-existing known issue** |
| B6 | `WorkspaceRegistry` **already exists** + is managed Tauri state - `workspace.rs:20/26/33/118`, `lib.rs:169-176` | Section 4.6's "net-new registry" is false; a name clash shadows real auth state | Name the brain registry distinctly (e.g. `KodenBrainRegistry`) |
| B7 | `lib.rs:3` has a parallel `use modules::{agent, fs, git, ...}` | Adding `brain` to `modules/mod.rs` alone is a compile-fail | Edit **both** `modules/mod.rs` and the `lib.rs:3` use-list (+ handler registration) |

### 0.2 Claims that were WRONG (v1 body and/or a reviewer) - corrected

| Claim as stated | Verdict | Truth (file:line) |
|---|---|---|
| "`MAX_FILE_BYTES` (256KB) doesn't exist; only byte cap is `MAX_READ_BYTES`=10MB" | **REFUTED** | `MAX_FILE_BYTES` exists = **2MB** (`git/types.rs:7`); a 256KB cap also exists as `MAX_OUTPUT_BYTES` (`shell/mod.rs:26`); `MAX_READ_BYTES`=10MB (`file.rs:11`). The brain's 256KB file cap is a **new invention** - do not cite `fs/search.rs` for it |
| "`reflect-llm` omits `usefulness`/`risk`/`evidenceQuality`" | **REFUTED** | The schema **includes** all three (`reflect-llm.ts:53-55`); they feed prior-blending (`:169-176`). Port them |
| "Doctor has 17 checks" | **PARTIAL** | It's **18** (`doctor.ts:18-36`) + the `TYPED_CHECK_MAP` snake_case bridge (`:514-520`) must be ported or typed checks vanish |
| "`app.listen` off a worker is unproven / needs a spike first" | **OVERCAUTIOUS** | Zero `.listen` sites today (true; 8 `emit` sites) but Tauri 2.11.2 `AppHandle: Listener` exists - one-line wiring (register in `.setup()`, push to `mpsc`), not a blocker |
| "No Linux keyring -> P4 reflect can't read a key on Linux" | **REFUTED** | `secrets.rs:46-108` has a complete Linux file fallback (`secrets.json`, mode 0600) used for get/set/delete. P4 works on Linux |
| "`DEFAULT_LIMIT=2000` reusable as the index/corpus cap" | **CLARIFIED** | `2000` (`search.rs:164`) is a **file-listing result cap** in `fs_list_files` (clamp `HARD_LIMIT=10_000`); fuzzy cap separate (`unwrap_or(200).min(1000)`, `:62`). Neither is a corpus cap - the brain defines its own |

### 0.3 Conductr port realities - NEW work, not "simple reuse"

- **Frontmatter is deliberately lossy** (null-strip, `frontmatter.ts:156-161`) - lossless round-trip is a Koden *addition*, and the null-strip must still be replicated for Zod schema-acceptance parity.
- **`ProposalAction` = 3 variants** (create/update/supersede, `reflect-proposals.ts:22`); **archive is an apply-op** in a **6-op machine matrix** (`:32-38`) - absent from v1, load-bearing for `apply-proposals`.
- **18 doctor checks** + `TYPED_CHECK_MAP` (above).
- **Two signature schemes stay separate:** `rejectSignature` (djb2, persisted, `proposal-store.ts:225-257`) vs `proposalSignature` (plain join, in-memory). Unifying breaks reject-suppression or in-run dedup.
- **RRF weighting hack is in 4 sites** (`hybrid-search.ts:193/213/229/263-266`) + a separate intra-leg path-inflation hack (`lib/code/search.ts:15-23`); `rrf.ts` uses a **global k only** - so the plan's first-class per-leg weight param is a **genuine improvement**, not redundant.

### 0.4 Citation hygiene (apply globally below)

`App.tsx` is at **`src/app/App.tsx`** (not `src/App.tsx`); Conductr `hybrid-search.ts`/`context-pack.ts`/`indexer.ts` live under **`src/lib/code/`** / **`src/lib/brain/`**. Treat all `:NNN` refs in sections 1+ as approximate - re-grep before editing.

### 0.5 Net status

The architecture (native in-process Rust, SQLite/FTS5, tree-sitter AST graph, RRF/tokenizer port, cache-stable gist) is **sound and confirmed faithful** to Conductr's proven mechanisms. The v1 "Request Changes" verdict was driven by the wrong-internals above - all now resolved or scoped as explicit new work. The corrected blocker list (0.1) + port realities (0.3) are the real pre-P0 work.

---


## Detailed phase sections

*(Authored in v1; read under Section 0's corrections.)*



---

## 1. Overview

Koden Brain (Code + Brain) is a native, in-process workspace-intelligence subsystem built directly into Koden's Tauri 2 Rust backend. It is **not** a Conductr wrapper, a Node subprocess, or an MCP server: per ADR-006 (`terax-workspace/.memory/decisions/ADR-006-koden-brain-native-architecture.md`), Conductr is the *idea source* whose proven mechanisms (lexical tokenizer, BM25/RRF ranking, ContextPack assembly, durable JSONL recovery) are reimplemented natively in Rust. The whole subsystem lives in one module tree, `src-tauri/src/modules/brain/`, and runs on one GUI-resident worker thread cloned from the usage poller template.

### 1.1 Goals (from ADR-006)

- **One binary, no subprocess.** The brain compiles into the existing Tauri binary and runs in-process. No external daemon, no Node, no MCP boundary.
- **One query path serves two consumers (the unified thesis).** The same ranked retrieval engine that answers the Brain pane's interactive search (`brain_search`) also synthesizes the cold-start gist injected into every agent (`brain_build_gist`). Search and context injection are not two features — they are two callers of one `SearchIndex::query()`.
- **Warm, incrementally-fresh index.** Because the brain is GUI-resident, it populates on launch and stays fresh via a recursive file watcher + blake3 deltas — versus Conductr's cold per-CLI rehash.
- **Zero-token by default.** P0–P3 (search, memory, AST graph, gist) spend no tokens and need no API key. The only token path is an opt-in, default-OFF, budgeted `reflect` (P4).
- **Real AST graph (the marquee upgrade).** tree-sitter replaces Conductr's regex symbol extraction (`Conductr/src/lib/code/indexer.ts:21-27`), giving real defs (incl. methods, re-exports, arrow-consts), imports, refs, and calls.
- **Fail-open, never blocks first paint.** The worker is spawned from `.setup()` after the usage poller and degrades to last-good state on any error — identical discipline to `usage::poll::spawn_poller` (`src-tauri/src/modules/usage/poll.rs:384`).
- **Cache-safe injection.** The gist sits in the cacheable prompt prefix; re-launch on unchanged code/notes must yield a byte-identical file (fingerprint-keyed) or it busts prompt cache (~90% input cost).
- **Portable canonical source, local-only derived index.** Git-committed + MegaSync-portable registry/notes with root-relative paths; the SQLite index is rebuildable and local-only under `app_local_data_dir()/koden/brain/`.

### 1.2 Non-goals (v1)

- **No semantic embeddings in v1.** Only the `VectorStore`/`Embedder` trait seams + an `embedderId` header land now; embeddings are deferred behind a default-OFF `semantic` cargo feature that does not compile into v1 (ADR-006 P5).
- **No `git2`/`gix` dependency.** blake3 per-file hashing is the primary freshness signal for all projects; git HEAD via the existing subprocess is an optional fast-path only.
- **No Python/Go grammars in v1.** TS/JS + Rust only; other languages still get full lexical search.
- **No `.conductr`/`.rulesync` artifacts.** Native naming throughout.
- **No autonomous writes.** `reflect` (P4) *only ever proposes*; humans approve every memory mutation.
- **Not a general LSP.** The AST graph answers def/import/ref/call/impact queries, not type inference or completion.

## 2. Architecture

### 2.1 The unified thesis: one query path, two consumers

```
                         ┌─────────────────────────────┐
   Brain pane search ───▶│                             │
   (webview command)     │  SearchIndex::query(...)    │──▶ ranked hits
                         │  tokenizer → FTS5 BM25 +    │     (code + notes
   Gist synthesis    ───▶│  AST graph + RRF + recency  │      + symbols)
   (cold-start, P3)      │                             │
                         └─────────────────────────────┘
```

`brain_search` returns the ranked hits directly to the pane. `brain_build_gist` issues the *same* query (synthesized from session context — agent name → intent, KODEN_SESSION → project, git HEAD + changed files, recent files, top notes), then runs the result through the ContextPack layered assembler to produce a token-bounded, fingerprint-keyed gist. Both go through `SearchIndex` so ranking improvements benefit both consumers and there is exactly one retrieval implementation to test.

### 2.2 Module tree — `src-tauri/src/modules/brain/`

```
brain/
  mod.rs              // BrainState, BrainConfig, public re-exports; .manage() target
  worker.rs           // spawn_brain_worker (clone of poll.rs:384); the single worker loop
  events.rs           // BrainEvent enum + the event spine (folds agent-signal + watcher)
  resolve.rs          // pty(id) → cwd → project resolution (PTY leaf map + registry prefix)
  registry.rs         // KodenBrainRegistry: project list, root-relative portable paths
  store/
    mod.rs            // SearchIndex trait + open/migrate; single SQLite file handle
    schema.rs         // SQL DDL (FTS5 + AST graph + notes + manifest + ledger)
    sqlite.rs         // SqliteIndex: impl SearchIndex over rusqlite (bundled+FTS5)
    migrate.rs        // versioned migrations; schema_version pragma
  tokenize.rs         // port of Conductr lexical.ts:61-137 (split+stem+stoplist)
  rank.rs             // BM25 params, IDF, RRF (weighted), multiplicative recency re-rank
  freshness/
    mod.rs            // FileFingerprint, manifest aggregate hash, delta computation
    hash.rs           // blake3 per-file content hash
    walk.rs           // ignore::WalkBuilder initial population (fs/search.rs bounds)
    watch.rs          // brain-owned RECURSIVE notify watcher (fs/watch.rs constants)
  ast/
    mod.rs            // language registry; LANGUAGE_VERSION pins; parse dispatch
    grammars.rs       // tree-sitter grammar handles (TS/JS + Rust v1)
    queries/          // per-language .scm capture queries (defs/imports/refs/calls)
    extract.rs        // tree → CodeNode/CodeEdge rows
    graph.rs          // forward+reverse adjacency persistence + incremental relink
    resolve_mod.rs    // module resolution (tsconfig paths, pkg exports, Cargo members)
  memory/
    mod.rs            // MemoryNote model; serde_yaml frontmatter → FTS5
    seed.rs           // lossless importer (~/.claude, ~/.codex, ~/.gemini)
    proposal.rs       // MemoryProposal queue (human-gated, gitignored) + doctor
  gist/
    mod.rs            // ContextPack layered fail-open assembly + intent planner
    synth.rs          // cold-start query synthesis from session context
    fingerprint.rs    // gist fingerprint key (byte-stable re-emit guarantee)
    write.rs          // extends App.tsx ~/.koden/agent-<id>.txt channel
  reflect/
    mod.rs            // P4: budgeted LLM reflect (reqwest+rustls, own key); default-OFF
    budget.rs         // check-reserve-call-reconcile ledger
  resume/
    mod.rs            // P4: events-only journal (~/.koden/resume/<sessionKey>.jsonl)
  semantic/           // P5 DEFERRED; behind cargo feature "semantic" (no v1 compile)
    mod.rs            // VectorStore + Embedder traits + embedderId header only
  commands.rs         // #[tauri::command] surface: brain_search, brain_index_status, ...
```

Net-new crates (locked, ADR-006): `rusqlite` `0.31` (features `bundled`, `backup`; FTS5 is in `bundled` SQLite — confirm via build smoke test, see open items), `tree-sitter` `0.22`, `tree-sitter-typescript` `0.21`, `tree-sitter-rust` `0.21` (pinned to a single ABI; CI smoke-parse per language), `blake3` `1.5`, `serde_yaml` `0.9`, `tauri-plugin-dialog` `2`. `notify`, `ignore`, `reqwest`+`rustls`, `keyring`, `serde`/`serde_json` are already in-tree.

### 2.3 Worker lifecycle

The worker is spawned from `lib.rs` `.setup()` **immediately after** `usage::poll::spawn_poller(app.handle().clone())` (`src-tauri/src/lib.rs:159`), and `BrainState` is registered alongside the other `.manage()` calls (`src-tauri/src/lib.rs:162-177`). The spawn function mirrors `spawn_poller` (`src-tauri/src/modules/usage/poll.rs:384-389`) exactly:

```rust
// brain/worker.rs
pub fn spawn_brain_worker(app: AppHandle) {
    std::thread::Builder::new()
        .name("koden-brain-worker".into())
        .spawn(move || brain_loop(app))
        .expect("spawn koden-brain worker thread");
}
```

```rust
// lib.rs .setup() — after the usage poller
usage::poll::spawn_poller(app.handle().clone());
brain::worker::spawn_brain_worker(app.handle().clone()); // NEW
```

```rust
// lib.rs .manage() block — alongside existing states
.manage(brain::BrainState::default()) // NEW
```

`BrainState::default()` is cheap and infallible (it allocates locks and an empty config — no I/O), so registration cannot block first paint. All real work (open SQLite, walk, parse) happens inside `brain_loop` on the worker thread.

`brain_loop` phases, all fail-open:

1. **Open store.** Resolve `app_local_data_dir()/koden/brain/index.sqlite`; open + migrate. On failure: `log::warn!`, set `BrainState.status = Degraded { reason }`, and continue serving an empty index (commands return empty results, never error). This matches `poller_loop`'s "client build failed → poller disabled" path (`poll.rs:391-397`).
2. **Bootstrap registry.** Load `KodenBrainRegistry` from the committed source folder; resolve root-relative → absolute against the current machine root.
3. **Warm population.** For each project, walk via `ignore::WalkBuilder` (reusing `fs/search.rs` bounds: `git_ignore/git_global/git_exclude/ignore(true)`, `follow_links(false)`, `MAX_SCANNED 50_000`, `PRUNE_DIRS`; plus `MAX_FILE_BYTES 256KB` skip), tokenize, upsert into FTS5, write fingerprint manifest. Population runs project-by-project so the first project is searchable before the last finishes.
4. **Arm watcher.** Start the brain-owned recursive `notify` watcher over each project root.
5. **Event loop.** Block on the internal event channel; fold `BrainEvent`s (below) into incremental index updates. No fixed sleep cadence (unlike the usage poller) — the loop is event-driven, waking only on watcher/agent events plus a periodic idle tick (60s) to reconcile the budget ledger and flush WAL.

### 2.4 BrainEvent enum and the event spine

**Decision: the Rust worker `app.listen()`s directly** (ADR-006 "direct preferred"). Routing `koden:agent-signal` back through the webview to re-emit to Rust would add a frontend round-trip, couple the brain to webview liveness, and lose events while the pane is unmounted. The worker registers an `app.listen("koden:agent-signal", ...)` handler that deserializes `AgentSignal` (`src-tauri/src/modules/pty/agent_detect.rs:37-41`: `{ id: u32, kind: &str, agent: Option<String> }`, emitted at `src-tauri/src/modules/pty/session.rs:227,258`) and forwards it onto the internal channel. The brain-owned watcher feeds the same channel. Both legs fold into one enum:

```rust
// brain/events.rs
pub enum BrainEvent {
    /// From app.listen("koden:agent-signal"). id = pty session id.
    Agent {
        pty_id: u32,
        kind: AgentKind,        // Started{agent}|Working|Attention|Finished|Exited
        agent: Option<String>,
    },
    /// From the brain-owned recursive notify watcher (debounced).
    Fs {
        project: ProjectId,
        changed: Vec<PathBuf>,  // create/modify/remove, already coalesced
    },
    /// Periodic self-tick: reconcile ledger, flush WAL, retry degraded store.
    Tick,
    /// Webview-initiated reindex request (e.g. wizard "rescan").
    Rescan { project: ProjectId },
}
```

Spine wiring:

```
app.listen("koden:agent-signal") ─┐
                                   ├─▶ mpsc::Sender<BrainEvent> ─▶ brain_loop recv
recursive notify watcher ──────────┤
periodic 60s tick thread ──────────┘
```

The watcher and the listen-callback are thin: they only translate and `send` onto the channel. All state mutation happens on the single worker thread, so the index is never touched concurrently by ingest paths.

**Event handling:**
- `Agent{Started{agent}}` → resolve project (§2.5), record `(pty_id → project, agent, intent)` in the live session map, and (P3) trigger gist synthesis for that pty/agent if injection is enabled.
- `Agent{Finished|Exited}` → mark session idle / drop from the live map; (P4) close the resume journal entry.
- `Fs{changed}` → blake3-hash each changed file, diff against the manifest, reindex only files whose hash changed (FTS5 delete+insert; AST re-parse + relink in P2). Watcher events are already debounced at 150ms / 1000ms-window (reusing `fs/watch.rs:14-15` constants), so a burst of saves collapses to one reindex pass.

### 2.5 pty → cwd → project resolution

`brain/resolve.rs` resolves a `koden:agent-signal`'s `pty_id` to a project:

1. **pty → cwd.** Read the PTY session's working directory. `PtyState.sessions` is a `RwLock<HashMap<u32, Arc<Session>>>` (`src-tauri/src/modules/pty/mod.rs:21-22`); `Session` carries the spawn `cwd` (`session.rs:114`). For v1 we resolve against the spawn cwd (stable for the common case where the agent runs in the pane's launch dir). **Open item:** live foreground-process cwd (an agent that `cd`s deeper) is not currently tracked — `Session` stores only the initial cwd. The fast, portable v1 answer is "use spawn cwd"; a later enhancement can read the leaf process cwd on supported platforms.
2. **cwd → project.** Match cwd against registered project roots using the same root-prefix discipline as `WorkspaceRegistry::is_authorized` (`src-tauri/src/modules/workspace.rs:33-36`: `set.iter().any(|root| target.starts_with(root))`). The brain picks the **longest** matching root (most specific project) when roots nest.
3. **No match** → fail-open: the signal is recorded with `project: None`; gist synthesis is skipped (no project context to inject). Never errors.

The brain reuses `WorkspaceRegistry` (already `.manage()`d) for authorization but keeps its own `KodenBrainRegistry` for the indexed-project list (which may be a curated subset and carries portable root-relative paths + per-project memory folder locations).

### 2.6 Concurrency model

The worker thread is the **single writer**. Everything that mutates the SQLite index, the fingerprint manifest, the AST graph, and the live session map happens on `brain_loop`. Ingest paths (watcher callback, agent-signal listener, tick) never touch state directly — they only `send` `BrainEvent`s.

Webview `#[tauri::command]`s are **readers** and must not block the worker:

```rust
// brain/mod.rs
pub struct BrainState {
    /// Connection pool for read queries from command threads. SQLite WAL mode
    /// allows concurrent readers while the worker writes. One r/o connection
    /// per command thread via a small pool; the worker holds the sole writer.
    readers: ReaderPool,                 // r2d2-style or thread-local r/o conns
    status: RwLock<BrainStatus>,         // Warming{pct}|Ready|Degraded{reason}
    sessions: RwLock<HashMap<u32, LiveSession>>, // pty_id → {project, agent, intent}
    config: RwLock<BrainConfig>,         // injection on/off, reflect budget, etc.
    tx: Mutex<Option<Sender<BrainEvent>>>, // commands enqueue Rescan here
}
```

Concurrency rules:

- **SQLite in WAL mode** (`PRAGMA journal_mode=WAL`) — the single writer (worker) and many readers (command threads) proceed without blocking each other. Commands open **read-only** connections (`SQLITE_OPEN_READONLY`); the writer connection lives only on the worker thread. This is the core no-block guarantee.
- **`RwLock` for in-memory state** (`status`, `sessions`, `config`): commands take read locks (cheap, concurrent); the worker takes brief write locks only at state transitions. Locks are never held across SQLite I/O or across `.await`/`block_on`.
- **Single-flight on expensive synthesis.** Gist synthesis for a given `(pty_id)` is single-flight: an `AtomicBool` per live session prevents overlapping syntheses, mirroring the usage poller's `in_flight` guard (`poll.rs:417-421`).
- **No `block_on` on the worker except the rare reflect call** (P4), exactly like the usage poller's one `tauri::async_runtime::block_on(fetch_once(...))` (`poll.rs:425`) — no tokio `time` feature needed.

Read commands therefore see a consistent WAL snapshot and are wait-free relative to the worker's writes.

### 2.7 Fail-open and error-handling conventions

Borrowed wholesale from `poller_loop` (`poll.rs:391-460`):

- **No panics on the worker.** Every fallible step is `match`/`if let Err(e)` with `log::warn!`/`log::debug!` and a fallback to last-good state. A poisoned `RwLock` is the only place we accept `.expect()` (consistent with `WorkspaceRegistry`, `workspace.rs:28`), since a poisoned lock indicates a prior panic and is unrecoverable.
- **Degraded mode, not dead mode.** Store-open failure → `BrainStatus::Degraded`; commands return empty/last-good results, never `Err`. The tick re-attempts store open so a transient failure self-heals.
- **Commands return `Result<T, String>`** (Koden convention, e.g. `fs_search` `src-tauri/src/modules/fs/search.rs:48`) but reserve `Err` for genuine bad input (e.g. malformed query), not for "index not ready" — readiness is reported via `brain_index_status`, so a warming brain returns partial results with a `warming: true` flag rather than failing.
- **Watcher resilience.** A watcher error on one project root is logged and that root is dropped; other roots keep watching. inotify-exhaustion risk (two watchers: existing NonRecursive `fs/watch.rs` + new recursive brain watcher) is mitigated by clear ownership — the brain watcher covers project roots for indexing; `fs/watch.rs` covers UI-opened explorer dirs. They are not pointed at the same trees by design.
- **Budget safety (P4).** check → reserve → call → reconcile ordering ensures a mid-call crash cannot leak the spent counter; the ledger row is reserved before the HTTP call and reconciled after (rolled back on failure).

### 2.8 Component diagram

```
                         Tauri app (single binary, single process)
 ┌───────────────────────────────────────────────────────────────────────────┐
 │ Webview (React)                                                             │
 │   Brain pane ── invoke ─▶ brain_search / brain_index_status / brain_*       │
 │   App.tsx agent launch ── writes ~/.koden/agent-<id>.txt (gist appended)    │
 └───────▲───────────────────────────────────────────────────────────────┬───┘
         │ ranked hits / status (read, WAL snapshot, non-blocking)         │
         │                                                  brain_build_gist│
 ┌───────┴────────────────────────────────────────────────────────────────▼──┐
 │ Rust backend                                                               │
 │                                                                            │
 │  #[tauri::command] surface (commands.rs) ── read-only conns ──┐            │
 │                                                               │            │
 │  ┌──────────────── koden-brain-worker thread ────────────────▼─────────┐  │
 │  │  brain_loop:  recv BrainEvent  ──▶  single WRITER                    │  │
 │  │    ├─ store/ (SqliteIndex: FTS5 + AST graph + notes + manifest)      │  │
 │  │    ├─ tokenize / rank (BM25+RRF+recency)                             │  │
 │  │    ├─ freshness/ (blake3 manifest, recursive notify watcher)         │  │
 │  │    ├─ ast/ (tree-sitter parse + graph relink)  [P2]                  │  │
 │  │    ├─ gist/ (ContextPack synth + fingerprint)  [P3]                  │  │
 │  │    └─ reflect/ (budgeted block_on reqwest)     [P4, default-OFF]     │  │
 │  └───────▲──────────────────────▲───────────────────────▲──────────────┘  │
 │          │ BrainEvent::Agent     │ BrainEvent::Fs        │ Tick/Rescan      │
 │  app.listen("koden:agent-signal")│ recursive notify      │                  │
 │          │                       │  watcher              │                  │
 │  pty/session.rs emits ───────────┘                       │                  │
 │  (agent_detect Transition→AgentSignal)                   │                  │
 │                                                                            │
 │  reuses: WorkspaceRegistry (root-prefix), PtyState (cwd), secrets.rs       │
 │          (keyring 'koden-ai'), App.tsx --append-system-prompt channel      │
 └────────────────────────────────────────────────────────────────────────────┘

 Storage:
   Canonical (git-committed, MegaSync-portable, root-relative):
     <root>/.koden-brain/registry.*       (project list)          [name TBC]
     <project>/.koden-memory/*.md         (memory notes)          [name TBC]
   Derived (local-only, rebuildable):
     app_local_data_dir()/koden/brain/index.sqlite   (FTS5 + graph + manifest + ledger)
```


---

## 4. Dependencies & Data Model

This section locks the crate set, the unified SQLite schema, the on-disk split between portable committed source and local derived cache, and the core Rust type seams (`SearchIndex`, `MemoryNote`, `WorkspaceRegistry`). It is the contract every later phase builds against. All net-new crates are justified against binary-size / compile-time budgets, since Koden ships on an LTO-fat, `opt-level="s"`, `panic="abort"`, `strip=true` release profile (`src-tauri/Cargo.toml:96-101`).

### 4.1 Crate table

Existing Koden deps verified in `src-tauri/Cargo.toml` (lines 22-51, plus the per-target `keyring` at 60-72). Net-new crates are added under `[dependencies]` and (for grammars) carry an explicit ABI pin.

| Crate | Version | Features | Status | Why / where used |
|---|---|---|---|---|
| `rusqlite` | `0.32` | `["bundled", "fts5", "blob", "functions"]` | **NEW** | The one unified store (FTS5 BM25 + AST graph + notes + manifest + ledger). `bundled` compiles SQLite from source (no system libsqlite dependency, deterministic across Win/macOS/Linux/MegaSync machines). `fts5` enables the virtual tables. `functions` registers the custom scalar used for the recency multiplier and the deterministic tie-break sort key. `blob` reserved for P5 vector payloads. |
| `tree-sitter` | `0.24` | default | **NEW** (P2) | Core parser runtime. ABI 14/15. The brain crate compiles against this exact minor; grammars MUST match its `LANGUAGE_VERSION` range (see ABI note below). |
| `tree-sitter-typescript` | `0.23` | default | **NEW** (P2) | TS + TSX grammars (`language_typescript()`, `language_tsx()`). |
| `tree-sitter-javascript` | `0.23` | default | **NEW** (P2) | JS / JSX / mjs / cjs. |
| `tree-sitter-rust` | `0.23` | default | **NEW** (P2) | Rust grammar. |
| `blake3` | `1.5` | default | **NEW** (P1) | Per-file content hash → freshness fingerprint. Pure-Rust, SIMD, no C toolchain. Primary freshness signal for ALL projects (ADR-006: collapses the git/no-git branch). |
| `serde_yaml` | `0.9` | default | **NEW** (P1) | Parse / emit `MemoryNote` YAML frontmatter. `0.9` is the last published line (crate is in maintenance) — acceptable: frontmatter is a frozen, small grammar. **Open item:** evaluate `serde_yaml_ng` fork if a security advisory lands. |
| `tauri-plugin-dialog` | `2` | default | **NEW** (P1) | First-boot folder picker for the setup wizard. Matches the `tauri = "2"` plugin generation already in use. |
| `notify` | `8.2.0` | default | **EXISTING** (`Cargo.toml:51`) | Brain-owned **recursive** watcher (`RecursiveMode::Recursive`) — distinct from `fs/watch.rs`'s `NonRecursive` per-open-dir watcher (`fs/watch.rs:186`). Reuse its `DEBOUNCE` (150ms, `fs/watch.rs:14`), `MAX_WINDOW` (1000ms, `fs/watch.rs:15`), and `SKIP_DIRS` (`fs/watch.rs:19`) constants by lifting them into a shared `brain::watch` const block (copy, do not import — keep ownership split clean per ADR-006 risk #5). |
| `ignore` | `0.4` | default | **EXISTING** (`Cargo.toml:31`) | `ignore::WalkBuilder` for gitignore-aware initial population. Reuse `fs/search.rs` bounds: `MAX_SCANNED = 50_000` (`fs/search.rs:30`) and a brain-local `MAX_FILE_BYTES = 256 * 1024` (256 KB cap — `fs/search.rs` caps by entry count, not bytes, so the brain adds the byte cap explicitly). |
| `serde` / `serde_json` | `1` | derive | **EXISTING** (`Cargo.toml:24-25`) | All command DTOs + the resume JSONL journal. |
| `reqwest` | `0.12` | `rustls-tls` | **EXISTING** (`Cargo.toml:43-46`) | P4 reflect call ONLY. Same client config as the usage poller. |
| `tokio` | `1` | `["rt"]` only | **EXISTING** (`Cargo.toml:50`) | `tauri::async_runtime::block_on` for the reflect call. No `time` feature — the worker sleeps via `std::thread::sleep`, mirroring `poll.rs` (`poll.rs:1-4`). |
| `keyring` | `3.6` | per-target native | **EXISTING** (`Cargo.toml:60-72`) | P4 reflect key under service `"koden-ai"` via `secrets.rs` (`entry()` at `secrets.rs:111`, `key()` at `secrets.rs:42`). |
| `dirs` | `6` | — | **EXISTING** (`Cargo.toml:27`) | Resolve `~/.koden/` and seed-import source dirs. App-local cache dir comes from `app.path().app_local_data_dir()` (Tauri), not `dirs`. |

**Binary-size / compile-time impact + mitigations**

- `rusqlite` with `bundled` compiles the full SQLite amalgamation (~250k LOC C) → the single largest new compile-time hit and roughly +1.5–2.5 MB to the stripped binary. Mitigation: it is the deliberate ADR-006 choice over tantivy precisely to keep the binary *smaller* than a Rust full-text engine + its transitive deps; `strip=true` + `opt-level="s"` already applied. Set `SQLITE_OMIT_*` build flags is NOT pursued (loses FTS5); accepted cost.
- `tree-sitter` core + 3 grammars are each a small C parser (~100–400 KB compiled, generated tables). Combined estimate +1–2 MB stripped, +20–40s cold compile on the LTO-fat profile. Mitigation: grammars are P2-gated — P0/P1 ship without them. Consider `[profile.release.package."tree-sitter-*"] opt-level = 0` to cut grammar codegen time without touching the brain's own opt level. **Open item:** measure actual delta on CI before committing the override.
- `blake3` SIMD pulls a small `cfg`-gated asm/intrinsics path; negligible size, fast compile.
- `serde_yaml` is small and already transitively common.
- `tauri-plugin-dialog` adds a webview dialog bridge; small.

Net-new at P0: `rusqlite` only. `blake3` + `serde_yaml` + `tauri-plugin-dialog` at P1. tree-sitter trio at P2. This keeps the P0 "warm lexical brain" cheap to build and verify.

### 4.2 SQLite schema (DDL)

One file, `index.sqlite`, opened with `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;`. All tables carry a `project_id` (FK to `projects`) so the single file serves the whole multi-project workspace. Schema version lives in `meta`. The store is created/migrated behind the `SearchIndex` trait (§4.4) so a future tantivy backend can satisfy the same trait without these tables.

```sql
-- ============================================================
-- meta: schema version + index header (incl. embedderId for P5)
-- ============================================================
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- seeded rows: schema_version, tokenizer_version, language_version,
-- created_at, embedder_id (NULL until P5 semantic feature populates it).

-- ============================================================
-- projects: registry projection (canonical source is the committed
-- WorkspaceRegistry file; this table is the rebuildable local mirror)
-- ============================================================
CREATE TABLE projects (
  id            INTEGER PRIMARY KEY,
  slug          TEXT NOT NULL UNIQUE,   -- stable id from registry
  root_rel      TEXT NOT NULL,          -- root-relative path (portable)
  abs_root      TEXT NOT NULL,          -- resolved on THIS machine (local only)
  display_name  TEXT NOT NULL,
  fingerprint   TEXT,                   -- aggregate blake3 of project
  indexed_at    INTEGER                 -- epoch ms of last full/delta index
);

-- ============================================================
-- code_files: one row per indexed source file
-- ============================================================
CREATE TABLE code_files (
  id           INTEGER PRIMARY KEY,
  project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  path_rel     TEXT NOT NULL,           -- project-root-relative, '/'-normalized
  lang         TEXT NOT NULL,           -- ts|tsx|js|jsx|mjs|cjs|rs|md|other
  size_bytes   INTEGER NOT NULL,
  blake3       TEXT NOT NULL,           -- per-file content hash
  mtime_ms     INTEGER NOT NULL,
  indexed_at   INTEGER NOT NULL,
  UNIQUE(project_id, path_rel)
);
CREATE INDEX idx_code_files_proj ON code_files(project_id);
CREATE INDEX idx_code_files_hash ON code_files(project_id, blake3);

-- ============================================================
-- FTS5 — code. content=external (contentless-linked) so we don't
-- duplicate file bodies; rowid == code_files.id.
-- Columns split so per-column weights give path the 3x boost.
-- detail=full keeps offsets for snippets.
-- ============================================================
CREATE VIRTUAL TABLE code_fts USING fts5(
  path,            -- tokenized path + filename (weighted 3x at query time)
  symbols,         -- def/identifier names extracted (regex in P0, AST in P2)
  body,            -- pre-tokenized file content
  content='',      -- contentless: we store only the index, not the text
  tokenize = 'koden_tok'   -- external tokenizer registered at open (see note)
);

-- ============================================================
-- FTS5 — notes. Same external tokenizer, applied IDENTICALLY (ADR-006).
-- rowid == notes.id.
-- ============================================================
CREATE VIRTUAL TABLE notes_fts USING fts5(
  title,
  tags,
  body,
  content='',
  tokenize = 'koden_tok'
);

-- ============================================================
-- notes: memory notes (frontmatter projected from .koden-memory/*.md)
-- ============================================================
CREATE TABLE notes (
  id           INTEGER PRIMARY KEY,
  project_id   INTEGER REFERENCES projects(id) ON DELETE CASCADE, -- NULL = root/global
  note_id      TEXT NOT NULL,          -- frontmatter id (stable, slug-like)
  path_rel     TEXT NOT NULL,          -- root-relative md path
  title        TEXT NOT NULL,
  mtype        TEXT NOT NULL,          -- MemoryType (see §4.5)
  status       TEXT NOT NULL DEFAULT 'active',  -- active|stale|deprecated|superseded|proposed
  tags_json    TEXT NOT NULL DEFAULT '[]',
  created_ms   INTEGER,
  updated_ms   INTEGER,
  blake3       TEXT NOT NULL,
  UNIQUE(note_id)
);
CREATE INDEX idx_notes_proj ON notes(project_id);
CREATE INDEX idx_notes_status ON notes(status);

-- ============================================================
-- AST graph (P2). Nodes = real defs (incl. methods, re-exports,
-- arrow-const fns). Edges = typed, stored BOTH directions for O(1)
-- forward+reverse traversal.
-- ============================================================
CREATE TABLE ast_nodes (
  id           INTEGER PRIMARY KEY,
  project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  file_id      INTEGER NOT NULL REFERENCES code_files(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL,   -- function|method|class|interface|type|enum|const_fn|reexport|module
  name         TEXT NOT NULL,
  qualified    TEXT,            -- e.g. ClassName.method, module path
  start_byte   INTEGER NOT NULL,
  end_byte     INTEGER NOT NULL,
  start_row    INTEGER NOT NULL,
  exported     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_ast_nodes_file ON ast_nodes(file_id);
CREATE INDEX idx_ast_nodes_name ON ast_nodes(project_id, name);

CREATE TABLE ast_edges (
  id           INTEGER PRIMARY KEY,
  project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  src          INTEGER NOT NULL,   -- ast_nodes.id OR code_files.id per edge_type
  dst          INTEGER NOT NULL,
  edge_type    TEXT NOT NULL,      -- declares|imports|references|calls|tested-by|documents|supersedes
  resolved     INTEGER NOT NULL DEFAULT 1,  -- 0 = unresolved import target (candidate)
  UNIQUE(src, dst, edge_type)
);
-- Forward + reverse traversal indexes (reverse adjacency is load-bearing
-- for brain_code_impact; ADR-006 risk #4 property-tests it).
CREATE INDEX idx_ast_edges_fwd ON ast_edges(project_id, src, edge_type);
CREATE INDEX idx_ast_edges_rev ON ast_edges(project_id, dst, edge_type);

-- ============================================================
-- fingerprint_manifest: per-project sorted-aggregate freshness.
-- aggregate = blake3 over the sorted (path_rel, blake3) lines of the
-- project's code_files. PRIMARY freshness signal (no git2/gix).
-- ============================================================
CREATE TABLE fingerprint_manifest (
  project_id   INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  aggregate    TEXT NOT NULL,    -- blake3 hex of sorted per-file digest lines
  file_count   INTEGER NOT NULL,
  git_head     TEXT,             -- optional fast-path only; NULL if no git
  computed_ms  INTEGER NOT NULL
);

-- ============================================================
-- proposals: ONE human-gated ledger (memory proposals + doctor findings
-- + reflect output). Append-only lifecycle; gitignored location.
-- ============================================================
CREATE TABLE proposals (
  id           INTEGER PRIMARY KEY,
  project_id   INTEGER REFERENCES projects(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL,   -- capture|doctor|reflect|transition
  status       TEXT NOT NULL DEFAULT 'queued', -- queued|approved|rejected|applied
  payload_json TEXT NOT NULL,   -- the proposed note / edit / supersede
  source       TEXT NOT NULL,   -- deterministic-doctor | reflect-llm | seed-import
  created_ms   INTEGER NOT NULL,
  decided_ms   INTEGER,
  dedupe_key   TEXT,            -- prevents re-queuing the same finding
  UNIQUE(dedupe_key)
);
CREATE INDEX idx_proposals_status ON proposals(project_id, status);
```

**bm25 / column-weight / tokenizer approach (explicit, per ADR-006)**

FTS5's built-in `bm25()` ranking function uses **fixed k1=1.2 and b=0.75** internally — which is *exactly* the ADR-006 target (`K1=1.2/B=0.75`). So we do NOT need to reimplement BM25 over raw postings for the standard case; we call `bm25(code_fts, w_path, w_symbols, w_body)` with **per-column weights** to get the 3x path boost: `bm25(code_fts, 3.0, 2.0, 1.0)`. FTS5's bm25 IDF is `log((N - df + 0.5)/(df + 0.5))`, which differs from ADR-006's `log(1 + (N - df + 0.5)/(df + 0.5))` only by the `1 +` smoothing term (FTS5 clamps the rare negative-IDF case instead). Decision: **accept FTS5's bm25() with column weights as the v1 ranker** (matches k1/b exactly, gives path-3x via weights, zero custom-ranking code), and expose the score to the Rust RRF layer as the per-leg input. The ADR's manual IDF formula is reserved for the AST-symbol leg where we score over our own postings (P2). This is recorded as an **open item** (the `1+` IDF discrepancy is negligible for ranking order but must be noted to the reviewer).

The **custom tokenizer** is integrated as an **FTS5 external tokenizer** registered at connection open via `rusqlite`'s `create_module`/FTS5 tokenizer API (a single `koden_tok` C-ABI callback that calls into the ported Rust `tokenize()`), NOT a pre-tokenization pass. Rationale: external tokenizer means FTS5 applies the *same* lowercasing + camel/Pascal/digit split + additive light-stemming + 50-word stoplist to BOTH the indexed text and the query string automatically, guaranteeing the "applied identically to code AND notes" invariant (ADR-006) without us re-tokenizing queries by hand. The tokenizer is a direct port of Conductr `lexical.ts:61` (`tokenize`), `lexical.ts:77` (`pushToken` + stem), `lexical.ts:101-126` (`stemLight` rules), and `lexical.ts:130-137` (`splitCamel`). **Open item:** the FTS5 external-tokenizer API requires emitting tokens with byte offsets; Conductr's tokenizer emits *additive* forms (whole token AND parts AND stem) which have overlapping/synthetic offsets — FTS5 supports `FTS5_TOKEN_COLOCATED` for exactly this (synonym tokens at the same position). The port must set the colocated flag on the part/stem forms. Flagged for the reviewer as the single trickiest integration point.

### 4.3 On-disk layout

Two physically separate trees: **committed portable source** (travels via git + MegaSync, root-relative paths) and **local derived cache** (rebuildable, never committed, machine-specific absolute paths live only here).

```
<workspace-root>/                         # the launch/home root the registry authorizes
├─ .koden-brain/                          # COMMITTED, MegaSync-portable
│  └─ registry.yaml                       # WorkspaceRegistry: projects w/ ROOT-RELATIVE paths
│
├─ <projectA>/
│  └─ .koden-memory/                      # COMMITTED per-project notes
│     ├─ decisions/ADR-001-foo.md         # MemoryNote (YAML frontmatter + body)
│     └─ invariants/bar.md
└─ <projectB>/
   └─ .koden-memory/...

# LOCAL-ONLY derived cache (Tauri app_local_data_dir(); gitignored by location,
# survives `git clean`, cold-rebuilt on a second machine on first run):
<app_local_data_dir>/koden/brain/
├─ index.sqlite                           # the unified store (§4.2)  [+ -wal, -shm]
├─ proposals/                             # mirror of queued proposals (human review inbox)
└─ resume/<sessionKey>.jsonl              # P4 events-only crash-resume journal
```

Path rules:
- `registry.yaml` and every note store **root-relative** paths only (`projectA/src/foo.ts`), never absolute — MegaSync syncs the file to machine #2 where `<workspace-root>` differs. Absolute roots are resolved at load time into `projects.abs_root` (local-only column).
- The SQLite cache is addressed via `app.path().app_local_data_dir()?.join("koden/brain/index.sqlite")`. It is NEVER written into the committed tree.
- Native naming throughout: `.koden-brain` / `.koden-memory`. No `.conductr` / `.rulesync` artifacts (ADR-006). **Open item / confirm:** these two folder names are the ADR's *proposed* names pending Kosta's confirmation.
- `~/.koden/agent-<id>.txt` (the gist injection target, written via `App.tsx` + `agentCommand.ts`) and `~/.koden/resume/` are runtime scratch under home, distinct from both trees above.

### 4.4 The `SearchIndex` trait

The single seam that lets tantivy (or any engine) replace the rusqlite/FTS5 backend without touching callers. Lives in `src-tauri/src/modules/brain/index/mod.rs`.

```rust
/// One entry returned by a search leg, pre-fusion. `score` is the raw
/// backend score (FTS5 bm25() is negative-is-better; the impl normalizes
/// to ascending-rank order before returning).
pub struct IndexHit {
    pub doc_id: i64,        // code_files.id or notes.id
    pub kind: DocKind,      // Code | Note
    pub path_rel: String,
    pub score: f64,         // ascending rank position (0 = best) after normalization
    pub snippet: Option<String>,
}

pub enum DocKind { Code, Note }

pub struct IndexDoc<'a> {
    pub project_id: i64,
    pub path_rel: &'a str,
    pub kind: DocKind,
    pub lang: &'a str,
    pub fields: IndexFields<'a>,   // path/symbols/body OR title/tags/body
    pub blake3: &'a str,
}

/// Backend-agnostic full-text index. rusqlite/FTS5 is the v1 impl
/// (struct `SqliteIndex`); tantivy slots in behind the same trait.
pub trait SearchIndex: Send + Sync {
    fn open(db_path: &std::path::Path) -> Result<Self, BrainError> where Self: Sized;

    /// Upsert one doc (delete-then-insert keyed on (project_id, path_rel)).
    fn upsert(&self, doc: &IndexDoc<'_>) -> Result<i64, BrainError>;

    /// Remove a doc by path (file deleted on disk).
    fn remove(&self, project_id: i64, path_rel: &str) -> Result<(), BrainError>;

    /// Per-leg query. `columns` carries the per-column weights
    /// (path=3.0 default). Returns ascending-rank IndexHits, capped at `limit`.
    fn query(
        &self,
        scope: QueryScope,      // project_id filter + DocKind filter
        text: &str,
        limit: usize,
    ) -> Result<Vec<IndexHit>, BrainError>;

    /// Counts for IDF / status reporting.
    fn doc_count(&self, scope: QueryScope) -> Result<u64, BrainError>;
}
```

RRF fusion (ADR-006: k=60, **first-class per-leg weight**, dropping Conductr's duplicate-the-list hack at `hybrid-search.ts:263`) lives *above* this trait in `brain/search/fuse.rs`:

```rust
pub struct RrfLeg { pub hits: Vec<IndexHit>, pub weight: f64 }
/// score(doc) = Σ_leg  weight_leg / (k + rank_leg(doc))   [k=60]
/// then multiplicative recency re-rank, then deterministic id tie-break.
pub fn reciprocal_rank_fusion(legs: &[RrfLeg], k: f64) -> Vec<FusedHit>;
```

### 4.5 `MemoryNote` frontmatter schema

YAML frontmatter parsed by `serde_yaml` from `.koden-memory/*.md`. Types ported from Conductr `typed-memory.ts:20-43` (the `MEMORY_TYPES` and `MEMORY_STATUSES` lists) and `frontmatter.ts:106/134` (parse/stringify). All path fields are **root-relative**.

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct MemoryNote {
    pub id: String,                       // stable slug id (required)
    pub title: String,
    #[serde(rename = "type")]
    pub mtype: MemoryType,                // see enum below
    #[serde(default = "MemoryStatus::active")]
    pub status: MemoryStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub supersedes: Vec<String>,          // note ids this replaces (drives doctor)
    pub created: Option<String>,          // ISO-8601
    pub updated: Option<String>,
    /// Root-relative paths this note is "about" (validated vs AST in P2).
    #[serde(default)]
    pub anchors: Vec<String>,
    #[serde(skip)]
    pub body: String,                     // markdown after frontmatter
    #[serde(skip)]
    pub path_rel: String,                 // filled at load
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "PascalCase")]
pub enum MemoryType {
    Decision, Invariant, Architecture, Convention, Workflow,
    Bug, Fix, Risk, Environment, Dependency, CredentialPointer,
    UserPreference, DeprecatedFact, RoadmapItem, OpenQuestion,
}   // exact port of typed-memory.ts:20-37

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus { Active, Stale, Deprecated, Superseded, Proposed }
// default (absent) == Active, per typed-memory.ts:41
```

Deserialization is case-insensitive on `type` (humans write `decision` or `Decision`, per Conductr `typed-memory.ts:46`); implement via a custom `Deserialize` that lowercases before matching, or a `#[serde(alias=...)]` set. **Open item:** decide alias-set vs custom deserializer (alias-set is simpler but must enumerate both cases for all 15 types).

### 4.6 `WorkspaceRegistry` / `ProjectEntry`

The committed `.koden-brain/registry.yaml`. Net-new in Koden (verified: no existing workspace-registry struct in `src-tauri/src/modules` — `grep` for `registry` hits only file-explorer code). Used by the worker's `pty → cwd → project` resolution: an `AgentSignal` (`pty/agent_detect.rs:37`, fields `id`/`kind`/`agent`) maps `id → pty leaf cwd → project` via **root-prefix match** against each `ProjectEntry.root_rel` resolved to `abs_root`.

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct WorkspaceRegistry {
    pub version: u32,                 // schema version of the registry file
    pub projects: Vec<ProjectEntry>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ProjectEntry {
    pub slug: String,                // stable id; FK basis for projects.slug
    pub display_name: String,
    /// ROOT-RELATIVE path from <workspace-root> (portable; MegaSync-safe).
    /// Resolved to an absolute root at load time, never persisted absolute.
    pub root_rel: String,
    #[serde(default)]
    pub languages: Vec<String>,      // hint set; pruning still by extension
    #[serde(default = "default_true")]
    pub enabled: bool,
}
```

Resolution helper (`brain/registry.rs`):

```rust
/// Resolve a PTY leaf cwd to the owning project by longest root-prefix match.
/// Returns None for cwds outside every registered root (fail-open: lexical
/// search still works, no gist injected).
pub fn project_for_cwd(reg: &ResolvedRegistry, cwd: &Path) -> Option<&ResolvedProject>;
```

`ResolvedRegistry` holds the in-memory `abs_root` (joined with the launch `<workspace-root>` at startup, per `LaunchDir` managed state, `lib.rs:177`). The registry is loaded in the worker's setup (after `spawn_poller`, `lib.rs:159`) and `.manage(BrainState)`'d (`lib.rs:162-177` pattern).


---

## Phase 0 — Warm lexical brain (zero-token search)

**Goal.** Stand up the `brain/` module tree, a GUI-resident worker cloned from the usage poller, the project registry, the unified SQLite store with FTS5, the ported tokenizer, BM25 + RRF ranking, `ignore::WalkBuilder` population, three `#[tauri::command]`s, and a minimal Brain pane. No tree-sitter, no watcher (P1), no LLM (P4).

**Acceptance gate (must all hold):**
1. Cold start warms every registered project on a background thread; first paint is never blocked (verified by asserting `.setup()` returns before population completes).
2. `brain_search` returns BM25+RRF-fused hits across code AND notes in **<150 ms** on a ~2,000-file project (criterion bench + integration assertion).
3. Zero network, zero tokens, zero keyring reads on the entire P0 path (verified by a `reqwest`-free module + a test that fails if `net.rs`/`secrets.rs` are imported under `brain/`).

All line references verified against `C:/Users/Snorlax/Snorlax/Products/terax-workspace` and `C:/Users/Snorlax/Snorlax/Products/Conductr` on 2026-06-20.

---

### 0.1 Cargo deps (add to `src-tauri/Cargo.toml`)

Existing and reused (verified `Cargo.toml:29,50`): `ignore = "0.4"`, `notify = "8.2.0"` (P1), `serde`, `serde_json`, `tauri = "2"`.

Net-new for P0:

```toml
rusqlite = { version = "0.32", features = ["bundled", "blob"] }
# bundled => no system sqlite; FTS5 ships in the amalgamation but must be enabled:
#   rusqlite "bundled" compiles SQLite with SQLITE_ENABLE_FTS5 ON by default (>=0.31). Verify at build:
#   a CI smoke test runs `SELECT fts5(?1)` pragma-probe (see test_fts5_available).
blake3 = "1.5"   # used in P1 freshness, declared now so the manifest table DDL is stable
```

> **Open item:** confirm `rusqlite 0.32` is the latest compatible with the workspace's MSRV. If FTS5 is not on by default for the pinned version, add `features = ["bundled", "bundled-full"]` or pass `-DSQLITE_ENABLE_FTS5`.

---

### 0.2 Module scaffold

Create `src-tauri/src/modules/brain/` and register it in `modules/mod.rs` (alphabetical, after `agent`, verified `modules/mod.rs:1`):

```rust
// modules/mod.rs  (add line)
pub mod brain;
```

File-by-file:

| File | Responsibility |
|---|---|
| `brain/mod.rs` | `BrainState`, public re-exports, `EVENT` const, module doc. |
| `brain/worker.rs` | `spawn_worker(app)` — clone of `poll.rs::spawn_poller` (`usage/poll.rs:384`). Owns cold population in P0; folds events in P1. |
| `brain/registry.rs` | `Registry` (`Arc<RwLock<RegistryData>>`), atomic temp-write load/save, root-relative path portability. |
| `brain/store.rs` | `Store` wrapping `rusqlite::Connection`, migrations, `SearchIndex` trait impl. |
| `brain/search_index.rs` | `pub trait SearchIndex` (tantivy swap seam). |
| `brain/tokenizer.rs` | Port of Conductr `lexical.ts:54-137`. |
| `brain/rank.rs` | `reciprocal_rank_fusion` (per-leg weights), recency re-rank, BM25 helpers. |
| `brain/walk.rs` | `ignore::WalkBuilder` population (reuse `fs/search.rs` bounds). |
| `brain/commands.rs` | `brain_search`, `brain_index_status`, `brain_list_projects` + request/response types. |

`brain/mod.rs`:

```rust
//! Koden Brain — native, in-process workspace intelligence. Fail-open by
//! design: any error keeps the last-good index and never crashes the host.
//! P0 = warm lexical brain (zero-token BM25 + RRF). No network, no LLM.

pub mod commands;
pub mod registry;
pub mod store;
pub mod tokenizer;
pub mod rank;
pub mod walk;
pub mod worker;
mod search_index;

use std::sync::{Arc, RwLock};

pub const BRAIN_EVENT: &str = "koden:brain-signal"; // matches koden:usage-signal naming (poll.rs:32)

/// Tauri-managed handle. Cheap to clone (all Arc inside).
#[derive(Clone)]
pub struct BrainState {
    pub registry: registry::Registry,
    pub store: Arc<store::Store>,
    pub status: Arc<RwLock<IndexStatus>>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub projects_total: usize,
    pub projects_warmed: usize,
    pub files_indexed: usize,
    pub last_warm_ms: Option<i64>,
    pub warming: bool,
}

impl BrainState {
    /// Build state synchronously (opens DB, runs migrations, loads registry).
    /// Fast: no file walking happens here — that is the worker's job.
    pub fn init(app: &tauri::AppHandle) -> Result<Self, String> {
        let store = store::Store::open(app)?;            // <app_local_data_dir>/koden/brain/brain.db
        let registry = registry::Registry::load(app)?;   // git-committed source, root-relative paths
        Ok(Self {
            registry,
            store: Arc::new(store),
            status: Arc::new(RwLock::new(IndexStatus::default())),
        })
    }
}
```

---

### 0.3 Worker wiring (clone of `poll.rs:384`)

`brain/worker.rs` mirrors `spawn_poller` exactly: named `std::thread::Builder`, fail-open, started from `.setup()` AFTER the usage poller. In P0 the worker only runs the cold warm pass once, then idles (the `notify` loop lands in P1).

```rust
use tauri::{AppHandle, Emitter, Manager};
use crate::modules::brain::{BrainState, BRAIN_EVENT, walk};

/// Spawn the brain worker thread. Mirrors usage/poll.rs:384 spawn_poller:
/// one dedicated std::thread, fail-open, never panics the host.
pub fn spawn_worker(app: AppHandle) {
    std::thread::Builder::new()
        .name("koden-brain-worker".into())
        .spawn(move || worker_loop(app))
        .expect("spawn brain worker thread");
}

fn worker_loop(app: AppHandle) {
    // Cold warm pass: walk + index every registered project. Each project is
    // independent; one failing project must not abort the rest (fail-open).
    let state = app.state::<BrainState>().inner().clone();
    let projects = state.registry.list();   // Vec<ProjectEntry> (absolute paths resolved here)
    set_warming(&state, true, projects.len());

    for p in &projects {
        match walk::warm_project(&state.store, p) {
            Ok(n) => {
                bump_warmed(&state, n);
                log::info!("brain: warmed {} ({n} files)", p.name);
            }
            Err(e) => log::warn!("brain: warm failed for {}: {e}", p.name),
        }
    }
    set_warming(&state, false, projects.len());
    let _ = app.emit(BRAIN_EVENT, &*state.status.read().unwrap());
    // P0 ends here (idle). P1 attaches the recursive notify watcher loop below.
}
```

Wire in `lib.rs` (verified `lib.rs:159` for the poller call site and `lib.rs:162-177` for the `.manage(...)` block):

```rust
// inside .setup(|app| { ... }), AFTER usage::poll::spawn_poller (lib.rs:159):
match brain::BrainState::init(&app.handle()) {
    Ok(brain_state) => {
        app.manage(brain_state);                       // .manage(BrainState)
        brain::worker::spawn_worker(app.handle().clone());
    }
    Err(e) => log::warn!("brain: disabled (init failed, fail-open): {e}"),
}
```

> Note: we `.manage()` from inside `.setup()` (via `app.manage`) rather than the chained `.manage(...)` at `lib.rs:162` because `BrainState::init` is fallible and we must fail-open. This is the same pattern the registry block at `lib.rs:169-176` already uses (constructs then bootstraps inside a block).

Register the three commands in the `tauri::generate_handler![...]` macro (verified starts `lib.rs:178`):

```rust
brain::commands::brain_search,
brain::commands::brain_index_status,
brain::commands::brain_list_projects,
```

---

### 0.4 Registry (`Arc<RwLock>`, atomic temp-write, root-relative paths)

Canonical source is git-committed + MegaSync-portable. The registry file stores **root-relative** project paths (absolute paths break on machine #2 per ADR-006 risk #6). Proposed location: `<root>/.koden-brain/registry.json` (native naming, NOT `.conductr`).

```rust
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    /// Path RELATIVE to the registry root (portable across machines/MegaSync).
    pub rel_path: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RegistryData {
    pub version: u32,             // = 1
    pub projects: Vec<ProjectEntry>,
}

#[derive(Clone)]
pub struct Registry {
    inner: Arc<RwLock<RegistryData>>,
    root: PathBuf,                // absolute registry root (resolved at load)
    file: PathBuf,               // <root>/.koden-brain/registry.json
}

impl Registry {
    pub fn load(app: &tauri::AppHandle) -> Result<Self, String> {
        let root = resolve_registry_root(app)?;        // launch dir / home, reuse workspace authorize logic
        let file = root.join(".koden-brain").join("registry.json");
        let data = match std::fs::read_to_string(&file) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| e.to_string())?,
            Err(_) => RegistryData { version: 1, projects: Vec::new() },
        };
        Ok(Self { inner: Arc::new(RwLock::new(data)), root, file })
    }

    /// List projects with rel_path resolved to ABSOLUTE paths for the worker.
    pub fn list(&self) -> Vec<ResolvedProject> {
        let g = self.inner.read().expect("registry poisoned");
        g.projects.iter().map(|p| ResolvedProject {
            name: p.name.clone(),
            abs_path: self.root.join(&p.rel_path),
        }).collect()
    }

    /// Atomic save: write to <file>.koden-tmp then rename (matches poll.rs:357-364).
    fn save(&self) -> Result<(), String> {
        let g = self.inner.read().expect("registry poisoned");
        let serialized = serde_json::to_string_pretty(&*g).map_err(|e| e.to_string())?;
        if let Some(dir) = self.file.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
        let tmp = self.file.with_extension("json.koden-tmp");
        std::fs::write(&tmp, serialized).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.file).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    }
}
```

The atomic temp-write+rename is a direct copy of `write_window_stamp` (`usage/poll.rs:355-365`, `.koden-tmp` suffix and rename-with-cleanup). The registry is authorized through the existing `WorkspaceRegistry` (`workspace.rs:20-36`) so brain projects must sit under an authorized root.

> **Open item:** P0 seeds the registry from the already-authorized launch dir + home (reuse `workspace::bootstrap_registry`, `workspace.rs:118`). The interactive add-project flow + folder picker (`tauri-plugin-dialog`) is P1's wizard. For P0, `registry.json` may be empty — the brain warms zero projects and `brain_search` returns `[]` cleanly.

---

### 0.5 SQLite store init + migrations (`store.rs`)

One file under `app_local_data_dir()/koden/brain/brain.db` (local-only, rebuildable, does not travel — ADR-006 storage model). `user_version` pragma drives migrations.

```rust
use rusqlite::Connection;

pub struct Store {
    conn: std::sync::Mutex<Connection>,    // single writer; reads also take the lock in P0
}

impl Store {
    pub fn open(app: &tauri::AppHandle) -> Result<Self, String> {
        use tauri::Manager;
        let dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?
            .join("koden").join("brain");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let conn = Connection::open(dir.join("brain.db")).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
        let store = Self { conn: std::sync::Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if v < 1 { conn.execute_batch(MIGRATION_001).map_err(|e| e.to_string())?;
                   conn.pragma_update(None, "user_version", 1).map_err(|e| e.to_string())?; }
        Ok(())
    }
}
```

**Migration 001 DDL** (P0 uses `doc`, `doc_fts`, `manifest`; the AST graph tables are created now so P2 needs no migration churn — they stay empty in P0; trait seam below keeps tantivy swappable):

```sql
-- MIGRATION_001
-- Canonical doc store. One row per indexed unit (a file in P0; a chunk later).
CREATE TABLE IF NOT EXISTS doc (
  id          INTEGER PRIMARY KEY,          -- deterministic via rowid; tie-break key
  project     TEXT NOT NULL,
  path        TEXT NOT NULL,                -- root-relative POSIX path
  kind        TEXT NOT NULL,                -- 'code' | 'note'
  mtime_ms    INTEGER NOT NULL,
  blake3      TEXT NOT NULL,                -- per-file content hash (P1 freshness; populated now)
  content     TEXT NOT NULL,               -- pre-tokenized stream (see 0.6 decision)
  UNIQUE(project, path)
);
CREATE INDEX IF NOT EXISTS idx_doc_project ON doc(project);

-- FTS5 over the PRE-TOKENIZED content. We feed FTS5 a token stream produced by
-- our Rust tokenizer (decision in 0.6), so FTS5's own tokenizer is the trivial
-- whitespace 'ascii' tokenizer with no further folding.
-- path column carried so we can apply a 3x column weight at query time.
CREATE VIRTUAL TABLE IF NOT EXISTS doc_fts USING fts5(
  path,                                     -- pre-tokenized path tokens
  body,                                     -- pre-tokenized content tokens
  content='',                               -- contentless (external content); we manage rows by rowid
  tokenize='ascii tokenchars '''            -- pass-through: split on spaces only
);

-- Fingerprint manifest (blake3 per file + sorted aggregate per project). P1.
CREATE TABLE IF NOT EXISTS manifest (
  project        TEXT PRIMARY KEY,
  files_json     TEXT NOT NULL,             -- {relpath: blake3}
  aggregate      TEXT NOT NULL,             -- blake3 of sorted concatenation
  built_ms       INTEGER NOT NULL
);

-- AST graph tables (P2). Declared now, empty in P0.
CREATE TABLE IF NOT EXISTS ast_def  (id INTEGER PRIMARY KEY, project TEXT, path TEXT, name TEXT, kind TEXT, start_line INTEGER, end_line INTEGER);
CREATE TABLE IF NOT EXISTS ast_edge (src INTEGER, dst INTEGER, rel TEXT);   -- import|ref|call
CREATE INDEX IF NOT EXISTS idx_ast_def_name ON ast_def(name);
CREATE INDEX IF NOT EXISTS idx_ast_edge_dst ON ast_edge(dst);
```

`SearchIndex` trait seam (`search_index.rs`):

```rust
pub struct Hit { pub id: i64, pub project: String, pub path: String, pub kind: String, pub bm25: f64 }
pub trait SearchIndex: Send + Sync {
    /// `legs`: pre-tokenized query streams per leg (path-only, body). Returns per-leg ranked hits.
    fn search_leg(&self, project: Option<&str>, query_tokens: &str, column: Column, k: usize) -> Result<Vec<Hit>, String>;
    fn upsert_doc(&self, project: &str, path: &str, kind: &str, mtime_ms: i64, blake3: &str, tokenized: &str) -> Result<i64, String>;
    fn delete_project(&self, project: &str) -> Result<(), String>;
}
pub enum Column { Path, Body, Both }
```

`Store` implements `SearchIndex`. Tantivy can replace it later (ADR-006) without touching `commands.rs` or `rank.rs`.

---

### 0.6 Tokenizer port (`tokenizer.rs`) + FTS5 integration decision

**Direct port of Conductr `lexical.ts:54-137`** — verified algorithm:
- `tokenize()` (`lexical.ts:54-67`): match `[A-Za-z0-9]+`, lowercase the whole word and `pushToken`, then `splitCamel` and push each lowered part that differs from the whole.
- `pushToken()` (`lexical.ts:69-83`): drop tokens `< 2` chars, drop the 50-word stoplist, push token, then additively push `stemLight(token)` if `stem != token && stem.len >= 3`.
- `stemLight()` (`lexical.ts:95-126`): exact rule order — `-ation→ate` (len>7); `-ated→-ate` (len>6, drop trailing `d`); `-ion→strip` (len>7, not `-ation`, base≥4, char-before-`ion` is a consonant); `-ed→strip` (len>4, not `-eed`/`-ied`); `-ied→-y` (len>4). Order matters — copy verbatim.
- `splitCamel()` (`lexical.ts:129-137`): four regex passes producing space-separated parts — `([a-z0-9])([A-Z])`, `([A-Z]+)([A-Z][a-z])`, `([A-Za-z])([0-9])`, `([0-9])([A-Za-z])` — keep-whole-AND-parts.
- Stoplist (`lexical.ts:15-49`): copy the exact 50-word `Set` into a `static STOPWORDS: &[&str]` / `phf` set.

Rust signature (applied identically to code AND notes):

```rust
pub fn tokenize(text: &str) -> Vec<String>;          // whole+parts+stems, stoplist-filtered, len>=2
pub fn tokenize_to_stream(text: &str) -> String;     // tokens joined by ' ' for FTS5 storage
fn split_camel(word: &str) -> Vec<String>;
fn stem_light(token: &str) -> String;
```

`splitCamel`'s overlapping JS regex passes are implemented with a hand-written boundary scanner (Rust `regex` crate can't do overlapping `replace` chains in one pass; a char-class state machine over `chars()` reproduces the four boundaries deterministically and is faster). Property test pins it to the JS output for a fixture corpus.

**FTS5 integration — DECISION: pre-tokenization pass, NOT an external/custom FTS5 tokenizer.**

Justification:
1. The Conductr algorithm is *generative* (one input word emits whole + N camel parts + up-to-2 stems, additively). FTS5's external tokenizer API (`fts5_tokenizer`) is a streaming callback expected to emit substrings of the *input* with byte offsets; emitting synthetic stem tokens that don't exist as substrings, plus duplicate whole+part tokens, fights the offset contract and complicates snippet/highlight. A pre-tokenization pass sidesteps all of it.
2. A custom FTS5 tokenizer in Rust requires unsafe FFI registration (`sqlite3_create_module`/`fts5_api`) — brittle across the bundled SQLite version and a maintenance hazard. Pre-tokenizing in safe Rust is trivial and identical for the future tantivy backend (which would use a `tantivy::tokenizer::Tokenizer` — same logical seam).
3. We store the already-tokenized stream in `doc.content`/`doc_fts.body` and configure FTS5 with the trivial `ascii` pass-through tokenizer (splits on whitespace only, no folding). So FTS5 sees exactly our tokens. Query strings go through the *same* `tokenize_to_stream` before being handed to `MATCH`.

Tradeoff acknowledged: snippet offsets point into the tokenized stream, not the original source. P0's pane shows the file path + a re-read source line range (we keep `mtime_ms`; the UI lazy-reads the file for display), so we don't rely on FTS5 `snippet()`.

---

### 0.7 BM25 K1=1.2/B=0.75 + path-3x weighting — DECISION

FTS5's `bm25()` uses fixed **k1=1.2, b=0.75** internally (the SQLite FTS5 defaults are exactly the ADR's targets), and `bm25(tbl, w_path, w_body)` accepts **per-column weights**. So:

**DECISION: use FTS5 `bm25()` with column weights for the ranking math, NOT a manual BM25 over postings.** Rationale: FTS5's built-in BM25 already implements K1=1.2/B=0.75 and the IDF form `log(...)` we want; reimplementing postings in Rust (Conductr did this only because TS had no FTS5 engine, `lexical.ts:160-221`) duplicates the engine and the recency/RRF layers don't need raw postings.

Path-3x is achieved with **column weights, not a separate leg-list duplication**:

```sql
-- Body-weighted leg (NL→code): body dominates.
SELECT rowid, bm25(doc_fts, 1.0, 1.0) AS score
FROM doc_fts WHERE doc_fts MATCH :q ORDER BY score LIMIT :k;

-- Path-weighted leg: path token matches count 3x.
SELECT rowid, bm25(doc_fts, 3.0, 0.0) AS score
FROM doc_fts WHERE doc_fts MATCH :q ORDER BY score LIMIT :k;
```

Note: FTS5 `bm25()` returns a **negative** score (more negative = better). We negate to `-bm25(...)` before RRF so "higher is better" downstream, and order by `bm25() ASC` (best first) at the SQL layer.

The two legs (body-weighted, path-3x-weighted) become the two RRF inputs. This replaces Conductr's "duplicate the content list to fake a per-leg weight" hack (verified `hybrid-search.ts:263-269`, the `[lexicalFileIds, contentFileIds, contentFileIds, contentFileIds, ...]` five-list trick) with first-class per-leg weights in our RRF (0.8). We get K1=1.2/B=0.75 for free and the path-3x emphasis via column weights.

> **Open item:** confirm the bundled SQLite FTS5 exposes the `bm25` auxiliary with the expected default k1/b for the pinned version (it has since FTS5 GA; pin-probe in `test_bm25_defaults`). If a future SQLite changes defaults, fall back to manual BM25 over `fts5vocab` postings (the formula in `lexical.ts:204-211` is the reference, `idf = ln(1+(N-df+0.5)/(df+0.5))`).

---

### 0.8 RRF with per-leg weights (`rank.rs`, ~20 lines, no crate)

Port of `rrf.ts:11-29` with a `weights` param added (drops the duplicate-list hack):

```rust
/// score(id) = Σ_legs weight_leg * 1 / (k + rank), rank starts at 1.
/// Deterministic id tie-break (ascending), matching rrf.ts:27.
pub fn reciprocal_rank_fusion(
    legs: &[(&[i64], f64)],   // (ranked ids best-first, leg weight)
    k: f64,                    // = 60.0
) -> Vec<(i64, f64)> {
    use std::collections::HashMap;
    let mut fused: HashMap<i64, f64> = HashMap::new();
    for (ids, weight) in legs {
        for (idx, &id) in ids.iter().enumerate() {
            let rank = (idx + 1) as f64;
            *fused.entry(id).or_insert(0.0) += weight * (1.0 / (k + rank));
        }
    }
    let mut out: Vec<(i64, f64)> = fused.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0))); // score desc, id asc
    out
}
```

Recency re-rank (multiplicative, ADR-006) applied after fusion:

```rust
/// multiplier in [recency_floor, 1.0]; newer files keep more of their score.
pub fn recency_rerank(fused: &mut [(i64, f64)], mtime_by_id: &HashMap<i64,i64>, now_ms: i64) {
    const HALF_LIFE_MS: f64 = 1000.0 * 60.0 * 60.0 * 24.0 * 30.0; // 30 days
    const FLOOR: f64 = 0.5;
    for (id, score) in fused.iter_mut() {
        let age = (now_ms - *mtime_by_id.get(id).unwrap_or(&now_ms)).max(0) as f64;
        let mult = FLOOR + (1.0 - FLOOR) * 0.5_f64.powf(age / HALF_LIFE_MS);
        *score *= mult;
    }
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
}
```

P0 call site: `legs = [(&body_ids, 1.0), (&path_ids, 1.0)]`, `k = 60.0`. The 3x path emphasis already lives in the column-weighted BM25 (0.7), so the RRF leg weights stay 1.0 in P0; the parameter exists for P2 (AST leg) and P5 (semantic leg).

---

### 0.9 `ignore::WalkBuilder` population (`walk.rs`)

Reuse `fs/search.rs` bounds and prune list verbatim (verified `fs/search.rs:30,34-46,74-92,164-166`):
- `MAX_FILES` cap = `2_000` (Conductr `indexer.ts:12` `MAX_FILES_DEFAULT = 2000` and `fs/search.rs:164` `DEFAULT_LIMIT`).
- `MAX_FILE_BYTES = 262144` (256 KB, Conductr `indexer.ts:13`).
- `PRUNE_DIRS` = copy `fs/search.rs:34-46` (`node_modules`, `.git`, `target`, `dist`, `build`, `.next`, `.turbo`, `.cache`, `.venv`, `__pycache__`) — and additionally skip `.koden-brain` (our own registry dir).
- WalkBuilder config copied from `fs/search.rs:74-91`: `.hidden(true)`, `.git_ignore(true)`, `.git_global(true)`, `.git_exclude(true)`, `.ignore(true)`, `.parents(true)`, `.follow_links(false)`, `.filter_entry(prune)`.

```rust
pub fn warm_project(store: &Store, p: &ResolvedProject) -> Result<usize, String> {
    let mut count = 0usize;
    let walker = build_walker(&p.abs_path);   // mirrors fs/search.rs:74
    for dent in walker.flatten() {
        if count >= MAX_FILES { break; }
        let path = dent.path();
        if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) { continue; }
        let Some(kind) = classify(path) else { continue }; // code by ext (ts/tsx/js/jsx/rs/...) or note (.md)
        let meta = match std::fs::metadata(path) { Ok(m) => m, Err(_) => continue };
        if meta.len() > MAX_FILE_BYTES as u64 { continue; }
        let body = match std::fs::read_to_string(path) { Ok(s) => s, Err(_) => continue }; // skip binary
        let rel = to_rel_posix(&p.abs_path, path);
        let hash = blake3::hash(body.as_bytes()).to_hex().to_string();
        let tokenized = tokenizer::tokenize_to_stream(&body);
        let path_tok = tokenizer::tokenize_to_stream(&rel);
        store.upsert_doc(&p.name, &rel, kind, mtime_ms(&meta), &hash, &tokenized /*+path_tok*/)?;
        count += 1;
    }
    Ok(count)
}
```

`upsert_doc` writes both the `doc` row and the contentless `doc_fts` row keyed by `rowid` (path column = `path_tok`, body column = `tokenized`).

---

### 0.10 Commands (`commands.rs`) + request/response types

```rust
use serde::{Deserialize, Serialize};
use tauri::State;
use crate::modules::brain::{BrainState, IndexStatus, rank, tokenizer};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainSearchRequest {
    pub query: String,
    pub project: Option<String>,   // None = all warmed projects
    pub limit: Option<usize>,      // default 20, hard cap 100
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainSearchHit {
    pub id: i64,
    pub project: String,
    pub path: String,        // root-relative POSIX
    pub kind: String,        // "code" | "note"
    pub score: f64,          // fused + recency-reranked
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainSearchResponse {
    pub hits: Vec<BrainSearchHit>,
    pub took_ms: u64,
    pub warmed: bool,        // false => index still warming, results may be partial
}

#[tauri::command]
pub fn brain_search(req: BrainSearchRequest, state: State<'_, BrainState>) -> Result<BrainSearchResponse, String> {
    let started = std::time::Instant::now();
    let limit = req.limit.unwrap_or(20).min(100);
    let q = tokenizer::tokenize_to_stream(&req.query);
    if q.is_empty() {
        return Ok(BrainSearchResponse { hits: vec![], took_ms: 0, warmed: state.status.read().unwrap().warming == false });
    }
    let kfetch = (limit * 5).max(50);
    let body = state.store.search_leg(req.project.as_deref(), &q, Column::Body, kfetch)?;
    let path = state.store.search_leg(req.project.as_deref(), &q, Column::Path, kfetch)?;
    let body_ids: Vec<i64> = body.iter().map(|h| h.id).collect();
    let path_ids: Vec<i64> = path.iter().map(|h| h.id).collect();
    let mut fused = rank::reciprocal_rank_fusion(&[(&body_ids, 1.0), (&path_ids, 1.0)], 60.0);
    // hydrate mtime/path/project from the union of leg hits, recency re-rank, take limit
    // ... (see rank::recency_rerank)
    let hits = hydrate_and_take(&state.store, fused, limit)?;
    Ok(BrainSearchResponse { hits, took_ms: started.elapsed().as_millis() as u64, warmed: !state.status.read().unwrap().warming })
}

#[tauri::command]
pub fn brain_index_status(state: State<'_, BrainState>) -> IndexStatus {
    state.status.read().expect("status poisoned").clone()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainProject { pub name: String, pub rel_path: String, pub files_indexed: usize }

#[tauri::command]
pub fn brain_list_projects(state: State<'_, BrainState>) -> Result<Vec<BrainProject>, String> {
    state.store.project_summaries()   // SELECT project, COUNT(*) FROM doc GROUP BY project
}
```

---

### 0.11 Minimal Brain pane (React)

New `src/components/brain/BrainPane.tsx` + a thin client `src/lib/brain/brainClient.ts` wrapping `invoke`. Matches the existing Tauri command-invoke pattern (the frontend already consumes `koden:usage-signal`).

```tsx
// brainClient.ts
import { invoke } from "@tauri-apps/api/core";
export interface BrainSearchHit { id: number; project: string; path: string; kind: "code" | "note"; score: number; }
export interface BrainSearchResponse { hits: BrainSearchHit[]; tookMs: number; warmed: boolean; }
export const brainSearch = (query: string, project?: string, limit = 20) =>
  invoke<BrainSearchResponse>("brain_search", { req: { query, project, limit } });
export const brainIndexStatus = () => invoke("brain_index_status");
export const brainListProjects = () => invoke("brain_list_projects");
```

`BrainPane.tsx`: a debounced search input (150 ms), a "warming N/M projects" banner driven by `brain_index_status` (poll once on mount + on `koden:brain-signal` via `listen`), and a results list (path, project badge, code/note kind chip, monospace path). No design polish in P0 — functional pane only. Clicking a hit opens the file via the existing file-open path (lazy-reads source for display since FTS5 snippet offsets are tokenized, per 0.6).

---

### 0.12 Tests + acceptance gate

**Rust unit tests** (`brain/tokenizer.rs`, `brain/rank.rs`):
- `test_tokenize_keeps_whole_and_parts` — `"writeAiFiles"` → contains `writeaifiles`, `write`, `ai`, `files`.
- `test_tokenize_digit_boundary` — `"utf8Decode"` / `"sha256"` split on digit boundaries both directions.
- `test_stem_light_rules` — table test of all five rules from `lexical.ts:95-126` (`validation→validate`, `validated→validate`, `rejection→reject`, `parsed→parse`, `applied→apply`); asserts additive (both forms present).
- `test_stoplist_drops_50` — every stoplist word filtered; `"the"`/`"and"` absent.
- `test_tokenizer_matches_conductr_fixture` — golden file: run a 200-word corpus through the JS `tokenize` (committed expected output JSON) and assert byte-equality.
- `test_rrf_per_leg_weight` — two legs, asymmetric weights, deterministic ordering + id tie-break (`b.score, then a.id`).
- `test_rrf_no_duplicate_hack` — confirms weighting via param yields the same ranking the old 3x-list hack produced for a known input (regression guard vs `hybrid-search.ts:263`).
- `test_recency_rerank_monotonic` — older file with equal fused score ranks below newer.

**Rust integration tests** (`brain/store.rs` + `#[cfg(test)]` harness over a temp dir):
- `test_fts5_available` — pragma-probe that FTS5 compiled in (gate on bundled build).
- `test_bm25_defaults` — confirms `bm25()` ordering matches K1=1.2/B=0.75 reference on a 3-doc fixture.
- `test_store_migrate_idempotent` — `migrate()` twice, `user_version` stays 1, no duplicate tables.
- `test_upsert_then_search` — index 3 files, `search_leg(Body)` returns the right rowid for an identifier query.
- `test_path_weight_3x` — a query matching only a path token ranks the path-leg hit; column-weight 3.0 boosts it above an equal body-only match.
- `test_warm_project_bounds` — fixture with a >256KB file and a `node_modules` dir; both skipped; count respects `MAX_FILES`.
- `test_search_empty_registry` — zero projects → `brain_search` returns `{ hits: [], warmed: true }`, no panic.

**Acceptance / perf gate:**
- `bench_brain_search_2k` (criterion) over a 2,000-file synthetic project: end-to-end `brain_search` (two BM25 legs + RRF + recency) p95 **< 150 ms**. CI fails over budget.
- `test_setup_nonblocking` — assert `.setup()` returns and first window paints before `IndexStatus.warming` flips false (worker runs off-thread).
- `test_no_network_in_brain` — a build/lint check (deny `reqwest`/`net.rs`/`secrets.rs` imports under `modules/brain/` in P0) proving zero-token, zero-network.

**TS test** (`brainClient.test.ts`, vitest): mocks `invoke`, asserts request shape `{ req: { query, project, limit } }` and camelCase response mapping.


---

## Phase 1 — Freshness, native memory, and the setup wizard

Goal (per ADR-006 P1 row): a recursive brain-owned watcher driving a blake3 incremental delta index; a native memory store searchable zero-token; a lossless seed importer for existing `~/.claude|.codex|.gemini` notes; one human-gated `MemoryProposal` queue with a deterministic (no-LLM) doctor; the Brain-pane review inbox; and a 3-step first-boot wizard. P1 assumes P0 has already landed `src-tauri/src/modules/brain/` with the `SearchIndex` trait, the `rusqlite` store, the ported tokenizer (`brain/lexical.rs`), the RRF/BM25 ranker, the worker thread + `BrainEvent` spine, and `.manage(BrainState)` wired from `lib.rs .setup()` after `usage::poll::spawn_poller` (lib.rs:159). P1 only *adds* to that spine — it does not re-spawn a thread.

All new code lives under `src-tauri/src/modules/brain/`. New files this phase:
`watcher.rs`, `fingerprint.rs`, `delta.rs`, `memory/mod.rs`, `memory/note.rs`, `memory/store.rs`, `memory/seed.rs`, `proposals.rs`, `doctor.rs`, `wizard.rs`, `safe_write.rs`. New deps (workspace `Cargo.toml`): `blake3 = "1.5"`, `serde_yaml = "0.9.34"` (last 0.9; pin exactly — it is in maintenance), `tauri-plugin-dialog = "2"`. (`notify`, `ignore`, `rusqlite` already present from P0; `serde`, `serde_json` already in tree.)

### 1.1 Ownership split: brain-owned recursive watcher vs existing `fs/watch.rs`

`fs/watch.rs` is **NonRecursive and per-open-dir** — `add_paths` calls `watcher.watch(&canonical, RecursiveMode::NonRecursive)` (watch.rs:188) and is refcounted against explorer-expanded + editor-open dirs. It exists to drive the `fs:changed` UI event for *visible* directories. It must **not** be reused for the brain: making it recursive would double-watch every subtree the explorer already watches non-recursively, and on Linux each `inotify` watch is a kernel descriptor — recursively watching a large monorepo on top of the existing per-dir watches risks `inotify` watch exhaustion (ADR-006 risk #5).

Locked split:

- **`fs/watch.rs` keeps its exact current contract** — NonRecursive, refcounted to UI viewport, emits `fs:changed`. Zero changes in P1.
- **`brain/watcher.rs` owns exactly one `RecommendedWatcher` per registered project root**, each `RecursiveMode::Recursive`, scoped to that root. It never watches `$HOME` or arbitrary dirs — only roots that passed the wizard / registry. It feeds the brain's own mpsc channel, *not* `fs:changed`, and folds results into the `BrainEvent` spine the P0 worker already drains. The two subsystems never watch the same path with the same recursion mode for the same purpose; the UI watcher is viewport-shallow, the brain watcher is root-deep-but-few.
- **De-dup of inotify pressure:** the recursive brain watch on a root subsumes the UI's non-recursive watches under it at the kernel level (separate watcher instances, but `notify` collapses to a small number of recursive descriptors via `inotify_add_watch` per subdir either way). We accept the overlap because the two have different lifetimes (UI watches churn as the user expands/collapses; brain watches are stable per session) and merging them would entangle UI latency with index correctness. We document the overlap and add a Linux watch-budget guard (§1.2).

Reused constants (imported, not copied — `pub(crate)` them in `fs/watch.rs` so `brain/` can reference the single source of truth):

```rust
// fs/watch.rs — promote to pub(crate) so brain/watcher.rs reuses the SAME list/values.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(150);   // watch.rs:14
pub(crate) const MAX_WINDOW: Duration = Duration::from_millis(1000); // watch.rs:15
pub(crate) const SKIP_DIRS: &[&str] = &[ /* watch.rs:19-92, unchanged */ ];
pub(crate) fn is_skipped(path: &Path) -> bool { /* watch.rs:94 */ }
```

`brain/watcher.rs` signatures:

```rust
// brain/watcher.rs
pub struct BrainWatcher {
    // One recursive watcher per root; key = canonical root path.
    watchers: Mutex<HashMap<PathBuf, RecommendedWatcher>>,
    tx: mpsc::Sender<notify::Result<Event>>, // shared sink into the brain debounce loop
}

impl BrainWatcher {
    /// Begin recursively watching a registered project root. Idempotent: a second
    /// call for the same canonical root is a no-op. Fail-open: a watch error logs
    /// and degrades to fingerprint-on-demand rescans, never panics.
    pub fn watch_root(&self, root: &Path) -> Result<(), String>;
    pub fn unwatch_root(&self, root: &Path);
}

/// The brain-owned debounce loop. Mirrors fs/watch.rs::drain_loop (watch.rs:137)
/// EXACTLY for timing semantics (DEBOUNCE quiet-gap, MAX_WINDOW latency cap),
/// but instead of emitting fs:changed it (a) filters out SKIP_DIRS + Access events
/// the way `collect` (watch.rs:172) does, (b) buckets canonical paths by owning root
/// via the workspace registry root-prefix match, and (c) pushes ONE
/// BrainEvent::FilesChanged { root, paths } per (root, window) into the spine.
fn brain_drain_loop(rx: mpsc::Receiver<notify::Result<Event>>, tx_event: mpsc::Sender<BrainEvent>, registry: WorkspaceRegistry);
```

`BrainEvent` (defined in P0 `brain/mod.rs`) gains one variant this phase:

```rust
pub enum BrainEvent {
    AgentSignal(AgentSignal),         // from koden:agent-signal (P0)
    FilesChanged { root: PathBuf, paths: Vec<PathBuf> }, // P1: folded watcher output
    // ... P3+ variants later
}
```

### 1.2 blake3 fingerprint manifest + incremental delta algorithm

**Primary freshness signal for ALL projects** (ADR-006): a blake3 content hash per indexed file plus a sorted aggregate root hash. No `git2`/`gix`. (Git HEAD is read via the *existing* git subprocess only as an optional cold-start hint in P3; it is not a P1 input.)

Manifest is a SQLite table in the P0 store file (one unified DB under `app_local_data_dir()/koden/brain/index.sqlite3`):

```sql
-- brain/migrations/0002_fingerprint.sql
CREATE TABLE IF NOT EXISTS fingerprint (
  root        TEXT NOT NULL,           -- canonical project root
  rel_path    TEXT NOT NULL,           -- root-relative, '/'-normalized (portable form, even though DB is local)
  blake3      TEXT NOT NULL,           -- 64-hex of blake3::hash(content)
  size_bytes  INTEGER NOT NULL,
  mtime_ms    INTEGER NOT NULL,        -- cheap pre-filter ONLY; hash is authoritative
  indexed_at  INTEGER NOT NULL,        -- epoch ms when this row's content was last folded into FTS5
  PRIMARY KEY (root, rel_path)
);
CREATE INDEX IF NOT EXISTS idx_fingerprint_root ON fingerprint(root);

-- Root aggregate: sorted-hash-of-hashes, the single "is this project fresh?" value.
CREATE TABLE IF NOT EXISTS root_fingerprint (
  root           TEXT PRIMARY KEY,
  aggregate      TEXT NOT NULL,        -- blake3 over sorted "rel_path\0blake3\n" lines
  file_count     INTEGER NOT NULL,
  computed_at    INTEGER NOT NULL
);
```

```rust
// brain/fingerprint.rs
pub struct FileFp { pub rel_path: String, pub blake3: String, pub size_bytes: u64, pub mtime_ms: i64 }

/// Hash one file. Reused bound: skip if size > fs/search.rs MAX_FILE_BYTES (256 KiB);
/// such files are recorded with a sentinel hash "oversize:<size>" so they don't churn.
pub fn hash_file(path: &Path) -> std::io::Result<FileFp>;

/// blake3 over the SORTED "rel_path\0blake3\n" join. Order-independent, deterministic,
/// byte-stable — this is the value P3's gist fingerprint key reads (cache-stability seed).
pub fn aggregate(files: &[FileFp]) -> String;
```

**Initial population** (cold start, P0 already does the walk for FTS5 — P1 piggybacks the hash on the same pass) uses `ignore::WalkBuilder` configured *identically* to `fs/search.rs` (search.rs:74-92): `.hidden(true).git_ignore(true).git_global(true).git_exclude(true).ignore(true).parents(true).follow_links(false)` plus the `filter_entry` SKIP_DIRS prune. Reused bounds: `MAX_SCANNED = 50_000` (search.rs:30) as the file-count ceiling, `MAX_FILE_BYTES = 256 * 1024` for per-file content read (confirm exact const name in `fs/search.rs`/`fs/file.rs`; if absent, define `const MAX_FILE_BYTES: u64 = 256 * 1024;` in `brain/fingerprint.rs` and note it as the canonical value).

**Linux watch-budget guard:** before `watch_root`, on `cfg(target_os = "linux")` read `/proc/sys/fs/inotify/max_user_watches`; if `file_count` for the root exceeds 50% of the remaining budget, skip the recursive watch for that root and fall back to a 30 s periodic `aggregate()` recompute (poll) instead of inotify. Logged once per root.

**Incremental delta algorithm** — runs once per debounce window from `BrainEvent::FilesChanged`:

```
on FilesChanged { root, paths }:                       # paths already SKIP_DIR/Access-filtered, deduped
  load prev: HashMap<rel_path, (blake3, mtime_ms)> = SELECT rel_path,blake3,mtime_ms FROM fingerprint WHERE root=?
  changed = []; added = []; removed = []
  for p in paths:                                       # only the touched files, NOT a full rescan
    rel = root-relative('/'-normalized) of p
    if !exists(p) OR is_dir(p) and now-gone:
       if prev.contains(rel): removed.push(rel); continue
       else: continue                                   # transient temp file we never indexed
    if is_oversize(p) or binary(p): skip + upsert sentinel; continue
    st = stat(p)
    if prev.get(rel) is Some(prevfp):
        if st.mtime_ms == prevfp.mtime_ms: continue      # cheap pre-filter: untouched, no hash
        fp = hash_file(p)
        if fp.blake3 == prevfp.blake3: upsert(mtime only); continue  # touched, content identical -> no reindex
        changed.push((rel, fp))
    else:
        added.push((rel, hash_file(p)))
  # A delete event may arrive as a parent-dir event; reconcile orphans only for the
  # affected subtree, not the whole root:
  for rel in prev.keys() where rel under any removed-dir path and !exists: removed.push(rel)

  txn:                                                  # ONE transaction per window = atomic delta
    for (rel, fp) in added ++ changed:
       reindex_file_into_fts5(root, rel, fp)            # delete old FTS rows for doc, re-tokenize, insert
       upsert fingerprint row (indexed_at = now)
    for rel in removed:
       delete FTS5 rows for doc; DELETE FROM fingerprint WHERE root=? AND rel_path=?
    recompute root_fingerprint(root) = aggregate(all current rows)  # cheap: re-select hashes, no re-hash of files
  emit BrainEvent-side notification -> brain_index_status push (so the pane shows "fresh")
```

Key properties the gate will assert: **only touched files are hashed** (mtime pre-filter + path-scoped, never a full walk on edit); **content-identical touch ⇒ zero reindex**; **the whole delta is one SQLite transaction** so a crash mid-window leaves either old-consistent or new-consistent state, never half.

### 1.3 Native memory store

Notes are markdown with YAML frontmatter. Canonical source is **git-committed + MegaSync-portable** with root-relative paths (ADR-006 storage model). Proposed native layout (confirm in open items): per-project notes at `<project>/.koden-memory/*.md`; global notes at `~/.koden/memory/*.md`. The derived FTS5 index is local-only (the same `index.sqlite3`).

```rust
// brain/memory/note.rs
#[derive(Debug, Clone)]
pub struct MemoryNote {
    pub id: String,            // stable slug: blake3(root + '\0' + rel_path)[..16] (deterministic, portable)
    pub scope: MemoryScope,    // Global | Project { project: String }
    pub rel_path: String,      // root-relative, '/'-normalized
    pub title: String,
    pub status: NoteStatus,    // Active | Superseded | Draft  (default Active)
    pub archived: bool,        // default false
    pub provenance: Provenance,// Curated | Imported { from: String } | Inferred (default Curated)
    pub created: Option<String>,
    pub updated: Option<String>,
    pub supersedes: Vec<String>,
    pub body: String,          // markdown after the frontmatter
    /// LOSSLESS preservation: the full parsed frontmatter as an ordered map, so
    /// unknown/extra keys round-trip byte-for-byte on rewrite. Never dropped.
    pub raw_frontmatter: serde_yaml::Mapping,
}
```

**Tolerant frontmatter parsing** (port of Conductr `utils/frontmatter.ts` + `parseFrontmatter`, frontmatter.ts:134): split on the leading `---\n...\n---\n` fence; parse the YAML with `serde_yaml::from_str::<serde_yaml::Mapping>` so we keep a *raw map* first, then project known keys off it. Mirror Conductr's null-stripping (frontmatter.ts:156: bare `key:` parses to null → drop) and its lineWidth-disabled dump on rewrite (`serde_yaml` default already does not wrap; verify and pin). Parse failures **must not** be silently swallowed — return `Err` with the file path, exactly like frontmatter.ts:149 (`Failed to parse frontmatter in {filePath}`). A note that fails to parse is surfaced as a doctor finding, not dropped.

```rust
// brain/memory/note.rs
pub fn parse_note(scope: MemoryScope, root: &Path, rel_path: &str, content: &str) -> Result<MemoryNote, String>;
/// Rewrite preserving raw_frontmatter ordering + unknown keys (lossless round-trip).
pub fn render_note(note: &MemoryNote) -> String;
```

**SQL: scope/provenance/archived/status are columns pushed into WHERE** (not body text), so the pane can filter without scanning:

```sql
-- brain/migrations/0003_memory.sql
CREATE TABLE IF NOT EXISTS memory_note (
  id          TEXT PRIMARY KEY,
  scope       TEXT NOT NULL,           -- 'global' | 'project'
  project     TEXT,                    -- non-null when scope='project'
  rel_path    TEXT NOT NULL,
  title       TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'active',
  archived    INTEGER NOT NULL DEFAULT 0,
  provenance  TEXT NOT NULL DEFAULT 'curated',
  created     TEXT, updated TEXT,
  blake3      TEXT NOT NULL,           -- of the file, ties memory freshness to the same signal
  raw_yaml    TEXT NOT NULL,           -- serialized raw_frontmatter for lossless rewrite
  body        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_note_scope ON memory_note(scope, project, archived, status);

-- Notes share the SAME FTS5 virtual table as code (P0), distinguished by a 'kind' column,
-- and are tokenized with the IDENTICAL ported tokenizer (brain/lexical.rs). See P0 §FTS5.
-- Insert: docs(kind='note', doc_id=note.id, path=rel_path, title, body) tokenized via the
-- external/pre-tokenization pass P0 established. Memory cards default-filter:
--   WHERE archived=0 AND status!='superseded'
```

```rust
// brain/memory/store.rs
pub fn upsert_note(conn: &Connection, note: &MemoryNote) -> Result<(), String>; // also (re)tokenizes into FTS5
pub fn delete_note(conn: &Connection, id: &str) -> Result<(), String>;
pub fn list_notes(conn: &Connection, filter: NoteFilter) -> Result<Vec<MemoryNote>, String>;
pub struct NoteFilter { pub scope: Option<MemoryScope>, pub include_archived: bool, pub include_superseded: bool }
```

Memory note files are also folded into the §1.2 watcher/delta path so an out-of-band edit to a `.koden-memory/*.md` reindexes within one debounce, same as code.

### 1.4 Native-notes seed importer

Imports existing curated notes from the three CLI homes so the brain is non-empty on first boot. Lossless round-trip is mandatory (mirror Conductr `claudecode-memory.ts` / `codexcli-memory.ts` / `rulesync-memory.ts` semantics, but read-only on the sources — we *copy into* `~/.koden/memory/` and `<project>/.koden-memory/`, never mutate the originals).

Sources scanned (all optional; absence is fine):
- `~/.claude/memory/*.md`, and `~/.claude/CLAUDE.md` (treated as one global note)
- `~/.codex/memory/*.md` (and AGENTS-style memory if present)
- `~/.gemini/memory/*.md`

```rust
// brain/memory/seed.rs
pub struct SeedSource { pub label: String, pub home: PathBuf, pub glob: &'static str }
pub struct SeedResult { pub imported: usize, pub skipped: usize, pub by_source: Vec<(String, usize)> }

/// Read each source note, parse_note (lossless raw_frontmatter), stamp
/// provenance = Imported { from: label }, write into the native global memory dir
/// via the safe writer (§1.6), and upsert into the store. Idempotent: re-running
/// matches on id (blake3 of source path) and only overwrites if the source blake3 changed.
pub fn seed_native_notes(conn: &Connection, sources: &[SeedSource], dest_root: &Path) -> Result<SeedResult, String>;
```

**Verify-count-or-fail-loud:** if *any* source directory exists and contains at least one `.md` but `SeedResult.imported == 0`, return `Err("seed importer found N source notes but imported 0 — refusing to claim an empty brain is seeded")`. This is the explicit "verify count>0 or fail loud" guard from the task; a silently-empty seed is a bug, not a valid outcome. (If no source dirs exist at all, `imported == 0` is legal and returns `Ok` with `by_source` empty.) Round-trip is asserted in tests by `render_note(parse_note(x)) == x` byte-for-byte over fixtures (§1.7).

### 1.5 MemoryProposal queue + deterministic doctor

**One** proposal type, ported from Conductr `reflect-proposals.ts::MemoryProposal` (reflect-proposals.ts:41) minus the LLM-only fields (those arrive in P4):

```rust
// brain/proposals.rs
pub enum ProposalScope { Global, Project { project: String } }
pub enum ProposalAction { Create, Update, Supersede, Archive } // mirror reflect-proposals.ts ProposalAction
pub enum ProposalApplyOp { Manual, Backfill { target: String } } // P1 subset of ADR-016 ops

pub struct MemoryProposal {
    pub id: String,                 // deterministic sig (see below)
    pub title: String,
    pub scope: ProposalScope,
    pub project: Option<String>,
    pub action: ProposalAction,
    pub reason: String,
    pub confidence: Confidence,     // Low | Medium | High
    pub body: String,               // markdown the human pastes/adapts
    pub risk: String,               // blast radius + manual step
    pub source: ProposalSource,     // P1: always Deterministic; P4 adds Llm
    pub apply: ProposalApplyOp,
}

/// Deterministic signature = blake3(scope|action|title|project)[..16].
/// Mirrors Conductr proposal-store.ts::rejectSignature (proposal-store.ts:225) so a
/// REJECTED proposal that the doctor would re-emit is suppressed (recurring-rejected).
pub fn signature(p: &MemoryProposal) -> String;
```

**Queue = append-only JSONL at `~/.koden/memory-proposals.jsonl`** (mirrors the durable-JSONL-tail pattern Koden already uses in `subagentBus.ts` / `AgentBusBridge.tsx` — tolerant line-by-line parse, skip a corrupt trailing partial line, never throw the whole file away). Plus a **SQLite mirror** for fast pane queries:

```sql
-- brain/migrations/0004_proposals.sql
CREATE TABLE IF NOT EXISTS proposal (
  id          TEXT PRIMARY KEY,       -- signature
  state       TEXT NOT NULL DEFAULT 'pending', -- pending | approved | rejected | edited
  json        TEXT NOT NULL,          -- the full MemoryProposal
  created_at  INTEGER NOT NULL,
  decided_at  INTEGER
);
-- Rejected sigs persist so the doctor's recurring-rejected suppression works (Conductr parity).
```

```rust
// brain/proposals.rs
pub fn enqueue(proposals: &[MemoryProposal]) -> Result<usize, String>; // append JSONL + mirror, de-dup by sig
pub fn read_queue() -> Result<Vec<(MemoryProposal, ProposalState)>, String>; // tolerant tail parse
pub fn set_state(conn: &Connection, sig: &str, state: ProposalState) -> Result<(), String>;
pub fn rejected_signatures(conn: &Connection) -> Result<HashSet<String>, String>;
```

**Deterministic doctor — pure, no LLM** (port of Conductr `doctor.ts::runMemoryDoctor`, doctor.ts:132; the command shell memory-doctor.ts:39 is explicitly "READ-ONLY, changes NOTHING, a human acts"). Findings → proposals via `proposalForFinding` (Conductr reflect-proposals.ts:160). P1 checks (the deterministic subset; AST-anchor checks defer to P2):

```rust
// brain/doctor.rs
pub enum DoctorCheck { Duplicate, BrokenLink, SupersededPresent, DanglingSupersede,
                       CyclicSupersede, MissingTemporal, Stale, Stub, ParseError }
pub enum Severity { Error, Warn, Info }
pub struct DoctorFinding { pub check: DoctorCheck, pub severity: Severity, pub scope: MemoryScope,
                           pub project: Option<String>, pub note_id: String, pub rel_path: String,
                           pub detail: String, pub related: Option<String> }
pub struct DoctorReport { pub generated_at: i64, pub notes: usize, pub findings: Vec<DoctorFinding>, pub debt_score: u32 }

/// PURE: takes the in-memory note set + now-ms + thresholds, returns findings. No I/O, no LLM.
/// Mirrors doctor.ts: duplicate (Jaccard >= dup_threshold default 0.85, doctor.ts:70),
/// broken [[link]] (checkLinks doctor.ts:217), superseded-but-present (checkSupersession
/// doctor.ts:232), missing created/updated (checkProvenance doctor.ts:283),
/// stale (> stale_days default 180, doctor.ts:303), stub (isStub doctor.ts:212).
pub fn run_doctor(notes: &[MemoryNote], now_ms: i64, opts: DoctorOptions) -> DoctorReport;

/// Debt score = deterministic weighted sum: error*5 + warn*2 + info*1, summed over findings.
/// PURE function of the report; identical input -> identical score (asserted in tests).
pub fn debt_score(findings: &[DoctorFinding]) -> u32;

pub fn findings_to_proposals(report: &DoctorReport, rejected: &HashSet<String>) -> Vec<MemoryProposal>;
```

`findings_to_proposals` filters out any proposal whose `signature()` is in `rejected` (recurring-rejected suppression, exactly the two-pass logic in memory-doctor.ts:62-79, but done in one pass here since rejected sigs are already in SQLite).

### 1.6 Review-inbox UI (approve / edit / reject) + safe writer

The Brain pane (P0) gains a **Review Inbox** tab listing `proposal` rows where `state='pending'`, newest first, grouped by scope/project, each card showing title, reason, confidence, risk, and a body preview. Three actions:

- **Approve** → for `apply.op == Backfill`, the brain performs the deterministic mechanical edit (e.g. inject `created`/`updated` derived from the file's git/mtime — mtime-only in P1, no git) through the **atomic safe writer**; for `apply.op == Manual`, approve just marks the proposal `approved` and opens the target note in the editor (no auto-edit). Then `set_state(sig, Approved)`.
- **Edit** → opens the proposal body in an inline editor; on save, the edited body is written to the target note via the safe writer and the proposal is marked `edited`.
- **Reject** → `set_state(sig, Rejected)`; the sig joins the rejected set so the doctor won't re-surface it.

**Atomic safe writer** (port the temp-file-then-rename pattern already proven in `usage/poll.rs::write_window_stamp`, poll.rs:355-363):

```rust
// brain/safe_write.rs
/// Write `content` to `path` atomically: write to `path.with_extension("<ext>.koden-tmp")`,
/// fsync, then std::fs::rename over the target; remove the tmp on rename failure.
/// Same shape as poll.rs:355. Every memory-note mutation (seed, approve, edit) goes
/// through here so a crash never leaves a half-written note. Returns the new blake3.
pub fn atomic_write_note(path: &Path, content: &str) -> Result<String, String>;
```

New Tauri commands (registered in the `invoke_handler!` block, lib.rs:178, alongside the P0 `brain_*` commands):

```rust
brain_list_proposals(filter) -> Vec<ProposalView>
brain_decide_proposal(sig: String, decision: Decision /* Approve|Reject */) -> Result<(), String>
brain_edit_proposal(sig: String, edited_body: String) -> Result<(), String>
brain_run_doctor(scope: Option<MemoryScope>) -> DoctorReport     // pure, on demand; enqueues new proposals
brain_list_notes(filter: NoteFilter) -> Vec<MemoryNoteView>      // memory cards
```

### 1.7 First-boot wizard (3 steps)

Net-new dep `tauri-plugin-dialog` (register in `lib.rs` `.plugin(...)` block at lib.rs:119-139). The wizard is shown once when the registry has no brain-tracked roots; it is **idempotent and partial-repair**: re-running after a partial first boot picks up where it left off (a root already watched is shown checked; seeding already done is shown as a count, not re-run unless source hashes changed).

Steps:

1. **Folder picker** — `tauri_plugin_dialog`'s `FileDialogBuilder::pick_folder` selects the workspace **root** (the MegaSync-synced parent that holds projects). The chosen root is `registry.authorize(root)` (workspace.rs:26) so all subsequent brain ops are inside an authorized root. The `<root>/.koden-brain/` registry dir is created (proposed name — confirm in open items).
2. **Project checklist** — under the root, run the `ignore::WalkBuilder` (depth-1, SKIP_DIRS-pruned) to list candidate project dirs (those containing a `package.json`, `Cargo.toml`, `.git`, etc.). User checks which to track. Each checked dir → `registry.authorize` + `brain_watcher.watch_root` + queued for cold population.
3. **Seed preview** — call a dry-run of `seed_native_notes` (scan only) and show "Found N notes in ~/.claude (M), ~/.codex (K), ~/.gemini (J) — import into Koden Brain?" with the count. On confirm, run the real seed through the safe writer; on the verify-count guard tripping (§1.4), show the loud error rather than a false success.

```rust
// brain/wizard.rs
pub struct WizardState { pub step: u8, pub root: Option<PathBuf>, pub candidates: Vec<ProjectCandidate>,
                         pub seed_preview: Option<SeedResult> }
pub fn wizard_status() -> WizardState;                 // for idempotent resume
pub fn wizard_pick_root(path: PathBuf) -> Result<Vec<ProjectCandidate>, String>;
pub fn wizard_select_projects(roots: Vec<PathBuf>) -> Result<(), String>; // authorize + watch + populate
pub fn wizard_seed(confirm: bool) -> Result<SeedResult, String>;
pub fn wizard_complete() -> Result<(), String>;        // marks first-boot done; safe to re-run
```

Frontend commands: `brain_wizard_status`, `brain_wizard_pick_root`, `brain_wizard_select_projects`, `brain_wizard_seed`, `brain_wizard_complete`.

### 1.8 Tests

Rust unit/integration tests live next to each module (`#[cfg(test)] mod tests`), Tauri-command-level tests use an in-memory `rusqlite` connection. Frontend uses the existing vitest harness.

Fingerprint / delta (`brain/fingerprint.rs`, `brain/delta.rs`):
- `fingerprint_aggregate_is_order_independent`
- `fingerprint_aggregate_is_byte_stable_across_runs`
- `delta_unchanged_file_touch_does_not_reindex` (mtime bump, same content → 0 FTS writes)
- `delta_only_touched_files_are_hashed` (assert hash_file call count == changed paths)
- `delta_added_changed_removed_classified_correctly`
- `delta_whole_window_is_one_transaction` (inject a panic mid-window → DB unchanged)
- `delta_removed_dir_reconciles_only_subtree_orphans`

Watcher (`brain/watcher.rs`):
- `watcher_skips_skip_dirs` (write into `node_modules` → no event)
- `watcher_buckets_paths_by_root`
- `watcher_debounce_collapses_burst_within_window` (timing parity with fs/watch.rs)
- `watcher_watch_root_is_idempotent`
- `watcher_linux_budget_guard_falls_back_to_poll` (`cfg(target_os="linux")`)

Memory note / store (`brain/memory/note.rs`, `store.rs`):
- `note_frontmatter_lossless_round_trip` (render(parse(x)) == x byte-for-byte over fixtures incl. unknown keys)
- `note_parse_failure_returns_err_with_path` (no silent swallow)
- `note_null_bare_key_is_stripped` (Conductr parity)
- `store_filters_archived_and_superseded_in_sql`
- `store_note_edit_reindexes_fts5`

Seed (`brain/memory/seed.rs`):
- `seed_imports_all_three_homes_lossless`
- `seed_is_idempotent_on_unchanged_source`
- `seed_fails_loud_when_sources_present_but_zero_imported`
- `seed_ok_empty_when_no_source_dirs`

Proposals / doctor (`brain/proposals.rs`, `brain/doctor.rs`):
- `proposal_signature_is_deterministic`
- `doctor_run_is_pure_same_input_same_findings`
- `doctor_debt_score_is_deterministic`
- `doctor_flags_duplicate_broken_link_superseded_stale_stub_missing_temporal`
- `doctor_no_llm_called` (no network/keyring access — assert by construction; doctor takes no client)
- `proposals_recurring_rejected_are_suppressed`
- `proposals_queue_jsonl_tolerates_corrupt_trailing_line`

Safe writer (`brain/safe_write.rs`):
- `atomic_write_leaves_no_tmp_on_success`
- `atomic_write_failure_removes_tmp_preserves_original`

Wizard (`brain/wizard.rs`):
- `wizard_is_idempotent_partial_repair` (re-run after step-2 only → step-3 still pending, no re-watch)
- `wizard_authorizes_selected_roots`

Frontend (vitest): `ReviewInbox.approve.test.tsx`, `ReviewInbox.reject_suppresses_proposal.test.tsx`, `SetupWizard.threeStep.test.tsx`.

### 1.9 Acceptance gate (P1)

All must pass to close P1 (extends the ADR-006 P1 gate row):

1. **Out-of-band freshness:** edit one tracked file out-of-band; within one debounce window (≤ ~1.15 s: 150 ms quiet-gap, 1 s max) `brain_index_status` reports the project fresh and a `brain_search` for new content returns it — and the delta hashed *only the touched file(s)* (verified by instrumentation/log: no full-root rehash on edit). A content-identical touch triggers **zero** FTS writes.
2. **Atomic delta:** a simulated crash mid-window leaves the DB in the pre-window state (no half-indexed file); recovery re-applies cleanly on next event. (`delta_whole_window_is_one_transaction`.)
3. **Seeded corpus searchable zero-token:** after the wizard seed, `brain_search` returns seeded notes with **zero network calls and zero tokens spent** (no keyring read, no reqwest). The seed verify-count guard fails loud if sources existed but nothing imported.
4. **Lossless memory:** every fixture note survives `render(parse(x)) == x` byte-for-byte including unknown frontmatter keys.
5. **Doctor → proposal → human decision:** `brain_run_doctor` produces ≥1 finding on a seeded corpus with a planted defect (e.g. a stub + a missing-temporal note); each maps to a `pending` proposal; approving applies via the atomic safe writer (or opens for manual ops); rejecting suppresses re-emission on the next doctor run. The doctor is **provably LLM-free** (takes no client, makes no network/keyring call).
6. **Wizard idempotent:** killing the app mid-wizard and relaunching resumes at the correct step with already-authorized roots checked and already-imported notes not re-imported.
7. **No double-watch regression:** with the brain watcher active, `fs/watch.rs` still emits `fs:changed` for the viewport unchanged; on Linux the watch-budget guard prevents inotify exhaustion on a >25k-file root (falls back to poll, logged once).
8. CI: `cargo test -p koden` green for all P1 test names above; `cargo clippy` clean; frontend vitest green.


---

## Phase 2 — tree-sitter AST graph (XL, the marquee differentiator)

This phase replaces the brain's lexical-only relationship view with a real syntactic graph. Conductr's graph builder (`Conductr/src/lib/code/graph.ts:1-18`, `graph.ts:48-54`) derives `imports`/`references`/`declares` edges with regexes (`IMPORT_FROM_RE`, `REQUIRE_RE`, `COMMAND_LITERAL_RE` at `graph.ts:49-54`) over the lexical index — it never sees scopes, methods, re-exports, arrow consts, or call sites. P2 ports the *node/edge model and the BFS impact shape* (`graph.ts:700-800`, `graph-types.ts:25-67`) but produces the edges from tree-sitter parse trees instead, and persists forward+reverse adjacency so incremental relink is O(neighbors), not a full edge rescan.

Prereq: P0 store (`modules/brain/store/` — the unified SQLite file under `app_local_data_dir()/koden/brain/index.db`) and P1 freshness (blake3 per-file hash manifest + recursive notify watcher) are landed. P2 is additive: AST tables are new; lexical FTS5 search keeps working if AST is absent (fail-open).

### 2.0 Module layout

New files under `src-tauri/src/modules/brain/ast/`:

```
ast/
  mod.rs            // pub re-exports; AstGraph facade; LANGUAGE_VERSION
  lang.rs           // Lang enum, grammar registration, parser pool
  queries.rs        // embedded .scm strings + compiled tree_sitter::Query cache
  queries/
    ts.scm  tsx.scm  js.scm  rust.scm   // include_str!()'d at build time
  extract.rs        // tree walk -> RawDef/RawImport/RawRef/RawCall + ScopeTable
  scope.rs          // per-file lexical scope/binding resolution
  resolve.rs        // module resolution (tsconfig/package.json/Cargo)
  model.rs          // AstNode, AstEdge, NodeKind, EdgeKind, NodeId
  graph.rs          // forward+reverse HashMap adjacency, BFS, impact, neighbors
  persist.rs        // ast_nodes / ast_edges / ast_files DDL + load/store
  incremental.rs    // re-parse one file + O(neighbors) relink
  anchors.rs        // AST-validated memory anchors (ports anchors.ts)
```

`mod.rs` exposes `pub struct AstGraph` held inside `BrainState` (added in P0 via `.manage(BrainState)` analogous to `lib.rs:162-169`). All P2 work runs on the existing brain worker thread (the `spawn_poller` clone — `usage/poll.rs:384`); parsing is CPU-bound and must NOT run on the Tauri command thread, so `brain_code_*` commands read the in-memory `AstGraph` under an `RwLock` and never parse synchronously.

### 2.1 Crates + grammar version pinning

`LANGUAGE_VERSION` is the tree-sitter ABI the core links. Pin to the `tree-sitter` 0.24.x line (ABI 14/15) and pin each grammar to a release that declares the SAME ABI range. Exact versions in `src-tauri/Cargo.toml`:

```toml
# Brain AST (P2). All grammars MUST resolve against tree-sitter 0.24's ABI range.
tree-sitter            = "=0.24.7"
tree-sitter-typescript = "=0.23.2"   # provides language_typescript() + language_tsx()
tree-sitter-javascript = "=0.23.1"
tree-sitter-rust       = "=0.23.2"
```

`=` (exact) pins, not caret — grammar ABI drift is risk #3 in ADR-006 Consequences. `mod.rs` exports the linked ABI as a constant and a runtime assert:

```rust
// modules/brain/ast/mod.rs
pub const LANGUAGE_VERSION: usize = tree_sitter::LANGUAGE_VERSION; // 15 for 0.24.x

/// Called once at parser-pool init. Fails-open (logs, disables AST) on mismatch
/// so a bad grammar bump can never crash the GUI.
pub fn assert_grammar_abi() -> Result<(), String> {
    for (name, lang) in lang::all_languages() {
        let v = lang.abi_version(); // tree_sitter::Language::abi_version()
        if !(tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION).contains(&v) {
            return Err(format!("grammar {name} ABI {v} outside [{}, {}]",
                tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION, LANGUAGE_VERSION));
        }
    }
    Ok(())
}
```

`Cargo.toml` profile note: tree-sitter grammars are C; LTO-fat is the existing release profile. Add `strip = "symbols"` (binary-size risk #2). Compile-time mitigation: only 3 grammars in v1; Python/Go are gated behind a future `ast-extra` feature (no-op now).

**CI smoke-parse gate (`ci/brain-ast-smoke`):** one fixture per language under `src-tauri/tests/fixtures/ast/{ts,tsx,js,rust}/sample.*`. Test `ast_smoke_parse_all_grammars` (in `ast/lang.rs` `#[cfg(test)]`) parses each fixture, asserts `tree.root_node().has_error() == false` AND that at least one expected def is extracted. This is the canary for ABI drift — it fails the build the moment a grammar bump breaks the parse or the queries.

### 2.2 Per-language `.scm` queries

Queries are `include_str!()`'d into `queries.rs`, compiled once into `tree_sitter::Query` and cached per `Lang`. Each capture name maps to a `RawDef`/`RawImport`/`RawRef` kind in `extract.rs`. Capture naming convention: `@def.<kind>`, `@def.name`, `@import.spec`, `@ref.name`, `@call.callee`.

**`queries/ts.scm`** (TS/TSX share this base; tsx adds JSX element refs). Covers methods, default exports, re-exports, arrow consts — the four things Conductr's regex misses (`indexer.ts:21-27`):

```scheme
; --- function / class / interface / type / enum defs ---
(function_declaration name: (identifier) @def.name) @def.function
(class_declaration name: (type_identifier) @def.name) @def.class
(interface_declaration name: (type_identifier) @def.name) @def.interface
(type_alias_declaration name: (type_identifier) @def.name) @def.type
(enum_declaration name: (identifier) @def.name) @def.enum

; --- methods (Conductr regex misses these) ---
(method_definition name: (property_identifier) @def.name) @def.method

; --- arrow / function consts: export const foo = () => {} ---
(lexical_declaration
  (variable_declarator
    name: (identifier) @def.name
    value: [(arrow_function) (function_expression)])) @def.const_fn

; --- default export: export default function / export default Name ---
(export_statement
  (function_declaration name: (identifier) @def.name)) @def.default_export
(export_statement
  value: (identifier) @def.name) @def.default_export

; --- re-exports: export { A as B } from "./x"  /  export * from "./x" ---
(export_statement
  (export_clause (export_specifier name: (identifier) @reexport.name))
  source: (string (string_fragment) @import.spec)) @reexport
(export_statement
  source: (string (string_fragment) @import.spec)) @reexport.star

; --- imports ---
(import_statement source: (string (string_fragment) @import.spec)) @import
(call_expression
  function: (identifier) @_req (#eq? @_req "require")
  arguments: (arguments (string (string_fragment) @import.spec))) @import.require

; --- call sites (for refs/calls edges) ---
(call_expression function: (identifier) @call.callee) @call
(call_expression
  function: (member_expression property: (property_identifier) @call.callee)) @call.method
```

**`queries/rust.scm`** (defs incl methods, `use` imports, calls):

```scheme
(function_item name: (identifier) @def.name) @def.function
(struct_item name: (type_identifier) @def.name) @def.struct
(enum_item name: (type_identifier) @def.name) @def.enum
(trait_item name: (type_identifier) @def.name) @def.trait
(impl_item (declaration_list (function_item name: (identifier) @def.name) @def.method))
(mod_item name: (identifier) @def.name) @def.mod
(const_item name: (identifier) @def.name) @def.const

; pub use / re-export
(use_declaration (use_as_clause path: (_) @import.spec alias: (identifier) @reexport.name)) @reexport
(use_declaration argument: (_) @import.spec) @import

; call sites
(call_expression function: (identifier) @call.callee) @call
(call_expression function: (scoped_identifier name: (identifier) @call.callee)) @call.scoped
(macro_invocation macro: (identifier) @call.callee) @call.macro
```

`queries/js.scm` = `ts.scm` minus the type/interface/enum/type_alias captures. `queries/tsx.scm` = `ts.scm` plus `(jsx_opening_element name: (identifier) @ref.name)` so component usage becomes a reference edge.

### 2.3 Per-file scope/binding tables (`scope.rs`)

Before edges can be bound, each file's *local* bindings must be resolved so a `@call.callee` of `foo` is linked to the right `foo` (local def, imported symbol, or unresolved). This is what makes the graph better than name-matching.

```rust
// modules/brain/ast/scope.rs
pub struct ScopeTable {
    /// Symbol name -> binding, innermost wins. Built by a pre-order walk that
    /// pushes a frame on function/class/block nodes and pops on exit.
    frames: Vec<HashMap<String, Binding>>,
}

pub enum Binding {
    LocalDef { node_id: NodeId },          // resolves to a def in THIS file
    Imported { spec: String, name: String }, // resolves later via resolve.rs
    Param,                                  // function parameter: not a graph edge
    Unresolved,
}

impl ScopeTable {
    /// O(1) innermost-first lookup.
    pub fn resolve(&self, name: &str) -> Option<&Binding> {
        self.frames.iter().rev().find_map(|f| f.get(name))
    }
}
```

A `@call.callee` is emitted as a `calls` edge ONLY when `resolve()` returns `LocalDef` or `Imported`; `Param`/`Unresolved` callees are dropped (or, when the name still matches a known def elsewhere, demoted to a *lexical-candidate* — see tiering in 2.7). This is the precision/recall split Conductr can't express.

### 2.4 Module resolution (`resolve.rs`)

Turns an `@import.spec` string into a target file `NodeId` (or a `package:` node for externals). Conductr only resolves relative paths (`graph.ts` "Resolve a relative import specifier" helper at `graph.ts:117+`). P2 adds the real resolver chain, evaluated in order, first hit wins:

1. **Relative** (`./`, `../`): join + extension fallback `["", ".ts", ".tsx", ".js", ".jsx", ".d.ts", "/index.ts", "/index.tsx", "/index.js"]`. (Rust: `mod x;` -> `x.rs` or `x/mod.rs`.)
2. **tsconfig `paths` + `baseUrl`**: parse the nearest `tsconfig.json` up the tree once per project, cache `{ baseUrl, paths: Vec<(glob, Vec<target>)> }`. Match longest-prefix alias, substitute `*`, then apply extension fallback. (Comments/trailing commas: parse with `jsonc-parser` semantics — use `serde_json` after a cheap comment-strip pass, fail-open to "external" on parse error.)
3. **package.json `exports`/`imports` map**: for a bare specifier `pkg` or `pkg/sub`, if it resolves to a workspace member (monorepo) bind to that member's file; otherwise emit `package:<pkg>` (the `depends-on` edge, ported from `graph-types.ts:54` `"depends-on"`).
4. **Cargo workspace members**: parse root `Cargo.toml` `[workspace].members`; `use crate_name::...` where `crate_name` is a member -> bind to that crate's `lib.rs`/`main.rs`. External crates -> `package:<crate>`.
5. **Unresolvable** -> dropped with a debug log; never an edge.

Resolution caches (`ResolverCtx`) are per-project and invalidated when `tsconfig.json`/`package.json`/`Cargo.toml` themselves change (the watcher already reports these; freshness manifest keys them).

### 2.5 Typed node/edge model + adjacency (`model.rs`, `graph.rs`)

Node ids stay string-compatible with Conductr's scheme (`graph-types.ts:7-19`) so the FTS5 layer and gist code can reference the same ids:

```rust
// modules/brain/ast/model.rs
pub type NodeId = String; // "file:<relpath>" | "symbol:<relpath>#<name>" | "package:<name>"

#[derive(Clone)]
pub struct AstNode { pub id: NodeId, pub kind: NodeKind, pub name: String,
                     pub path: Option<String>, pub symbol_kind: Option<String> }

pub enum NodeKind { File, Symbol, Package }

pub enum EdgeKind {
    Declares,   // file -> symbol (was lexical; now AST-exact incl methods)
    Imports,    // file -> file (resolved)
    DependsOn,  // file -> package
    Calls,      // symbol -> symbol (scope-resolved; the NEW high-precision edge)
    References, // file -> symbol (name-match fallback, lower confidence)
    Reexports,  // file -> symbol/file (re-export passthrough)
}

#[derive(Clone)]
pub struct AstEdge { pub from: NodeId, pub to: NodeId, pub kind: EdgeKind, pub confident: bool }
```

`confident=true` for scope-resolved `Calls`/`Imports`/`Declares`; `false` for `References` name-match fallbacks. This single bool drives the tiered impact API (2.7).

Adjacency held in memory for query speed; persisted lazily:

```rust
// modules/brain/ast/graph.rs
pub struct AstGraph {
    nodes: HashMap<NodeId, AstNode>,
    fwd: HashMap<NodeId, Vec<EdgeRef>>,   // from -> outgoing
    rev: HashMap<NodeId, Vec<EdgeRef>>,   // to   -> incoming   (THE relink enabler)
    by_file: HashMap<String, Vec<NodeId>>, // relpath -> nodes it owns (for fast file delete)
}
struct EdgeRef { other: NodeId, kind: EdgeKind, confident: bool }
```

Maintaining `rev` is the explicit answer to ADR-006 risk #4 (incremental relink correctness).

### 2.6 Persistence DDL (`persist.rs`)

Three new tables in the unified `index.db` (same file as FTS5 + memory + manifest + ledger). Local-only, rebuildable (ADR-006 storage model):

```sql
CREATE TABLE IF NOT EXISTS ast_files (
  project   TEXT NOT NULL,
  relpath   TEXT NOT NULL,
  lang      TEXT NOT NULL,
  blake3    TEXT NOT NULL,          -- ties to P1 freshness manifest; skip re-parse if unchanged
  parsed_at INTEGER NOT NULL,
  PRIMARY KEY (project, relpath)
);
CREATE TABLE IF NOT EXISTS ast_nodes (
  project     TEXT NOT NULL,
  id          TEXT NOT NULL,        -- "symbol:src/foo.ts#bar"
  kind        TEXT NOT NULL,        -- file|symbol|package
  name        TEXT NOT NULL,
  relpath     TEXT,                 -- owning file (NULL for package nodes)
  symbol_kind TEXT,                 -- function|class|method|const_fn|...
  PRIMARY KEY (project, id)
);
CREATE TABLE IF NOT EXISTS ast_edges (
  project   TEXT NOT NULL,
  from_id   TEXT NOT NULL,
  to_id     TEXT NOT NULL,
  kind      TEXT NOT NULL,          -- declares|imports|depends_on|calls|references|reexports
  confident INTEGER NOT NULL,       -- 1 AST-confident, 0 lexical-candidate
  src_file  TEXT NOT NULL           -- the file whose parse PRODUCED this edge (delete key)
);
CREATE INDEX IF NOT EXISTS idx_ast_edges_to    ON ast_edges(project, to_id);
CREATE INDEX IF NOT EXISTS idx_ast_edges_from  ON ast_edges(project, from_id);
CREATE INDEX IF NOT EXISTS idx_ast_edges_src   ON ast_edges(project, src_file);
CREATE INDEX IF NOT EXISTS idx_ast_nodes_file  ON ast_nodes(project, relpath);
```

`src_file` is the linchpin of incremental: every edge records WHICH file's parse emitted it, so a re-parse deletes exactly `WHERE src_file = ?` and re-inserts. `to_id` index makes reverse-edge load O(log n).

### 2.7 Incremental re-parse + relink (`incremental.rs`) — the correctness core

Driven by the P1 watcher (recursive notify, SKIP_DIRS + 150ms/1000ms debounce reused from `fs/watch.rs:14-19`). For a debounced batch of changed/created/deleted files:

```
reindex_file(project, relpath, new_content_or_None):
  # 1. freshness gate
  h = blake3(new_content)                       # None content => delete
  if ast_files[relpath].blake3 == h: return     # no AST change

  # 2. tear down THIS file's outgoing contribution (O(edges_of_file))
  for nid in graph.by_file[relpath]:            # symbols + the file node
      remove node nid from nodes, and for each EdgeRef in fwd[nid]/rev[nid]:
          drop the mirrored entry in the partner's rev/fwd
  drop all edges WHERE src_file == relpath      # outbound imports/calls/refs/declares
  # NOTE: inbound edges from OTHER files (their imports OF us) are NOT touched here.

  if deleted: persist; return

  # 3. parse + extract (O(file size))
  tree = parser.parse(new_content)
  defs, imports, refs, calls = extract(tree, scope_table(tree))

  # 4. (re)create nodes for this file
  add file node + symbol nodes; record by_file[relpath]

  # 5. forward bind THIS file's edges (O(symbols+imports))
  for imp in imports: to = resolve(imp); add Imports/DependsOn edge (src_file=relpath)
  for call in calls:  to = scope-resolve; add Calls(confident) or References(!confident)
  for def in defs:    add Declares edge

  # 6. REBIND INBOUND edges from other files in O(neighbors), NOT O(repo):
  #    other files may import/call a symbol THIS file just (re)declared.
  for nid in graph.by_file[relpath]:            # each (re)created node
      for eref in rev_pending(nid):             # see below
          re-point the partner's stale edge to the fresh node id
```

The O(neighbors) inbound rebind avoids re-scanning the whole repo. Two cases:

- **Edge target id is stable** (`symbol:rel#name` unchanged): the partner's `Imports`/`Calls` edge in `ast_edges(to_id=...)` already points at the right id; nothing to do — it was never deleted because we only delete by `src_file`. The `rev` adjacency entry survives. This is the common case and is genuinely O(1) per surviving edge.
- **Edge target id changed/vanished** (symbol renamed/removed): the surviving inbound edge now dangles. We resolve it by querying `rev[old_id]` (the in-memory reverse map gives us exactly the neighbor files that imported/called the old symbol), and for each we mark it *unresolved -> demote to lexical-candidate* or, if a same-name symbol reappeared, re-point. Either way the cost is `O(|rev[old_id]|)` = number of inbound neighbors, never the repo.

Because inbound neighbors are reached through `rev` (built and persisted via `idx_ast_edges_to`), no neighbor file is re-parsed. That is the formal meaning of "rebinds inbound edges from OTHER files in O(neighbors)."

**Property test `ast_incremental_equals_full_rebuild`** (`incremental.rs` `#[cfg(test)]`, `proptest` crate):
- Generate a small random TS/Rust project (3-8 files, random import/call/rename/delete mutations) via a `proptest` strategy `arb_project_mutation_seq()`.
- Build graph A by `reindex_file` applied incrementally across the mutation sequence.
- Build graph B by full rebuild of the final state.
- Assert canonicalized equality: sort nodes by id, edges by `(from,to,kind,confident)`; `assert_eq!(canon(A), canon(B))`.
- Shrinking is on; any mismatch yields the minimal failing mutation sequence. This is the P2 GATE for relink correctness.

Companion deterministic unit tests:
- `ast_reparse_only_changed_file` — touching `a.ts` re-parses `a.ts` only (assert a parse-counter stays 1 for untouched `b.ts`).
- `ast_delete_file_drops_owned_nodes_and_outbound_edges` — file delete removes its nodes + `src_file` edges; inbound edges from others survive but flip to unresolved.
- `ast_rename_symbol_rebinds_inbound_in_o_neighbors` — rename `foo`->`bar` in `a.ts`; the import edge from `b.ts` is demoted/repointed without re-parsing `b.ts`.

### 2.8 Commands (`graph.rs` Tauri commands; registered in `lib.rs` `generate_handler!` at `lib.rs:178`)

```rust
#[tauri::command] pub fn brain_code_graph(
    state: State<BrainState>, project: String, node: String, depth: Option<usize>
) -> Result<GraphResult, String>;

#[tauri::command] pub fn brain_code_impact(
    state: State<BrainState>, project: String, target: String
) -> Result<ImpactResult, String>;

#[tauri::command] pub fn brain_neighbors(
    state: State<BrainState>, project: String, node: String
) -> Result<NeighborsResult, String>;
```

- **`brain_code_graph`** — BFS over `fwd` adjacency to `depth` (default 1), ports `findRelated` (`graph.ts:647`). Returns nodes + edges within the ball. Read-locks `AstGraph`; never parses.
- **`brain_neighbors`** — direct `fwd[node]` + `rev[node]` in one shot (depth-0 incoming + outgoing), the cheap "what's adjacent" call for the pane.
- **`brain_code_impact`** — ports `computeImpact` (`graph.ts:700-800`) reverse-BFS over `rev` `Imports` edges, but **TIERED**:

```rust
pub struct ImpactResult {
    /// confident=true closure: reverse Imports + scope-resolved Calls. High precision.
    pub ast_confident: Vec<AstNode>,
    /// confident=false: name-match References + demoted unresolved callees. Candidates.
    pub lexical_candidates: Vec<AstNode>,
    /// additive views ported from Conductr (graph.ts ComputeImpactResult, graph.ts:84-110)
    pub impacted_tests: Vec<AstNode>,
    pub impacted_docs: Vec<AstNode>,
    pub impacted_packages: Vec<AstNode>,
}
```

The tiering directly implements ADR-006 P2 gate "tiered AST-confident vs lexical-candidate." `ast_confident` is the reverse closure restricted to `confident` edges; `lexical_candidates` is `(name-match referencers ∪ unresolved callees) \ ast_confident`. The frontend renders confident edges solid, candidates dashed.

### 2.9 AST-validated memory anchors (`anchors.rs`)

Ports `Conductr/src/lib/code/anchors.ts:114` `deriveAnchors`, but the validation source is the AST graph instead of the regex graph. Anchor kinds reuse Conductr's enum (`anchors.ts:19-27`): `references-path`, `references-symbol`, `references-command`, `belongs-to-area`, `unsupported-by-code`, `contradicted-by-code`, `stale-attached`.

Upgrade: a note claiming "see `validateInput()` in `auth.ts`" was, in Conductr, confirmed by a name regex over the file. In Koden it is confirmed only if `ast_nodes` contains `symbol:src/auth.ts#validateInput` (def actually exists). If the symbol is gone -> `contradicted-by-code` (was a real symbol per a prior manifest) or `unsupported-by-code` (never existed) with `confidence: high` because AST is authoritative. Pure function, no I/O, returns `Vec<Anchor>` exactly like `anchors.ts` (`anchors.ts:104-122`); the brain worker feeds it `&AstGraph` + the `MemoryNote` corpus and surfaces results as P1 `MemoryProposal`s (human-gated) — never auto-writes (ADR-006 "only ever PROPOSES").

Test `ast_anchor_contradicted_when_symbol_removed` and `ast_anchor_high_confidence_only_with_ast` assert the AST-vs-lexical confidence difference.

### 2.10 Phase gate (acceptance)

P2 ships only when ALL pass:

1. `cargo test -p koden brain::ast` green, including the property test `ast_incremental_equals_full_rebuild` (1000 cases default).
2. CI `brain-ast-smoke` parses the per-language fixtures with zero parse errors and ≥1 extracted def each.
3. `brain_code_impact` on a fixture repo returns a non-empty `ast_confident` reverse-import+call closure that is a strict superset relationship correctness vs the `lexical_candidates` tier (test `impact_confident_superset_of_lexical_for_known_target`).
4. Out-of-band edit -> watcher debounce -> `reindex_file` re-parses ONLY the changed file (`ast_reparse_only_changed_file`).
5. Manual: rename a widely-imported symbol; impact view updates within one debounce window without a full rebuild and stays equal to a forced full rebuild.
6. Binary size delta and cold compile time recorded; no regression beyond the ADR-006 risk-#2 budget (note the measured numbers in the PR).


---

## Phase 3 — Gist assembly + cache-stable injection (the payoff)

> **Goal (ADR-006 P3):** the same warm query path that powers the Brain pane (P0) now feeds *every agent pane* a token-bounded "gist" via the **existing** `~/.koden/agent-<id>.txt` + `--append-system-prompt` channel — built from local index data with **zero tokens spent**. The non-negotiable gate: **a relaunch on unchanged code + notes produces a byte-identical file**, so the gist lives safely in the cacheable prompt prefix and does not bust prompt cache (~90% input-cost penalty per ADR-006 "Top risks" #1).
>
> **Dependencies:** P0 (warm lexical `SearchIndex`, `brain_search`), P1 (`MemoryNote` store + recursive watcher + blake3 fingerprint manifest), and the freshness line from P1. P2 (AST graph) is **optional input** here: graph-neighbor layer degrades to empty when `brain_code_graph` is unbuilt, exactly like Conductr's `findRelated` degrades to `[]` (`context-pack.ts:406-428`). Phase 3 must compile and pass its gate **with or without P2**.

### 3.0 Module layout

All new code under the established tree `src-tauri/src/modules/brain/`:

| File | Responsibility |
|---|---|
| `brain/gist/pack.rs` | `ContextPack` struct + `assemble_pack()` (port of `context-pack.ts` layered fail-open + caps + proportional trim). |
| `brain/gist/budget.rs` | `estimate_tokens()` calibrated chars/type heuristic + `trim_to_budget()`. |
| `brain/gist/synthesis.rs` | `synthesize_query()` cold-start ambient-signal → query/intent. |
| `brain/gist/render.rs` | `render_gist()` — `ContextPack` → the deterministic, byte-stable Markdown string. |
| `brain/gist/fingerprint.rs` | `gist_fingerprint()` (blake3 over code state + notes + query) + the on-disk cache map. |
| `brain/gist/gate.rs` | `confidence_gate()` (thin/empty pack when ambient signal weak). |
| `brain/commands.rs` (extend) | `brain_build_gist`, `brain_write_gist` Tauri commands. |
| `brain/gist/tests/` | unit + integration tests (named below). |

The query-planner is ported verbatim from `query-planner.ts:82-205` into `brain/gist/planner.rs` (pure, deterministic, never-throws — it already has no I/O, so it is a 1:1 Rust translation of the regex matcher table + intent profiles). It is shared between P0 search ranking biases and P3 synthesis.

---

### 3.1 ContextPack assembly (port of `context-pack.ts`)

Port the **layered, fail-open** composition from `buildContextPack` (`context-pack.ts:771-1364`). Every layer is independently bounded and degrades to empty on any error — the assembled pack **never** fails the launch (fail-open is the ADR-006 worker-wide rule).

```rust
// brain/gist/pack.rs
pub struct ContextPack {
    pub query: String,
    pub project: String,
    pub intent: QueryIntent,
    pub freshness: FreshnessLine,            // ALWAYS present, never trimmed (see §3.1.2)
    pub code_files: Vec<ContextCodeFile>,    // skeleton + top snippets
    pub graph_neighbors: Vec<ContextNeighbor>, // [] when P2 absent
    pub memory_notes: Vec<ContextMemoryNote>,
    pub confidence: Confidence,              // Low | Medium | High
    pub token_estimate: u32,
}

pub fn assemble_pack(p: AssembleParams<'_>) -> ContextPack
```

`AssembleParams` carries `&dyn SearchIndex` (the P0 trait), the resolved `project` + `project_root`, the synthesized `query`/`intent`, the P1 `FreshnessLine`, and the per-layer caps. **No closures-of-I/O DI** like Conductr's `ContextPackDeps` (`context-pack.ts:97-173`) — in-process Rust calls the trait directly; testability comes from a fake `SearchIndex` impl, not injected fns.

#### 3.1.1 Layers + per-layer caps

Ordered the same as Conductr's composition (`context-pack.ts:7-12`), each wrapped so an `Err`/panic-free failure yields an empty layer:

| Layer | Source | Cap (const) | Conductr origin |
|---|---|---|---|
| **freshness line** | P1 `FreshnessLine` | always 1 line, never dropped | `context-pack.ts:733` ("freshness is intentionally KEPT") |
| **code skeleton** | `SearchIndex::search(query, CODE)` top hits → def signatures (P2) or first-N lines (P0 fallback) | `MAX_CODE_FILES = 6`, `SNIPPET_MAX_CHARS = 300` | `context-pack.ts:563,572,1007-1014` |
| **top snippets** | same hits, BM25/RRF-ranked | folded into code_files | `context-pack.ts:932-1014` |
| **graph neighbors** | `brain_code_graph` reverse/forward adjacency on top file ids | `MAX_GRAPH_NEIGHBORS = 12` | `context-pack.ts:566,1034-1049` |
| **top memory notes** | `SearchIndex::search(query, NOTES)` | `MAX_MEMORY_NOTES = 8`, snippet 300 | `context-pack.ts:562,873-890` |

Constants mirror `context-pack.ts:562-572` exactly so the calibration carries over. We **drop** these v5/v6 Conductr layers in v1 P3 (they need git/temporal infra not in scope and add bytes that hurt cache stability): `temporalWarnings`, `staleConflictWarnings`, `recentActivity`, `suggestedTests`, `evidenceNotes`, `ledger`, `debug`. The freshness line already carries the "what changed" signal via P1's blake3 digest, which is the one temporal signal we keep.

#### 3.1.2 Proportional trim (port of `trimPackToTokenBudget`)

Port `trimPackToTokenBudget` (`context-pack.ts:599-753`) with the **freshness-always-kept** invariant intact (`context-pack.ts:733-748` clears every optional section but never `freshness`). Trim order:

1. Progressively shorten snippets `[200, 120, 60]` chars (`context-pack.ts:606`).
2. Cap note/file counts (`slice(0,3)`/`slice(0,2)`) with 60-char snippets (`context-pack.ts:638-654`).
3. Last-resort: truncate every path/name/query string to `max(4, budget_chars/8)` (`context-pack.ts:676`).
4. Hard floor: drop all layers **except the freshness line** (`context-pack.ts:734-748`).

The trim must be **deterministic** (same pack + same budget → same output) — it already is in Conductr (pure transforms over sorted inputs); the Rust port keeps stable iteration order (`Vec`, not `HashMap`, for all pack fields) so this property holds for the byte-identical gate.

---

### 3.2 Token budget — calibrated chars/type heuristic

There is **no accurate cross-vendor tokenizer** available in-process (Claude/Codex/Gemini/GLM all tokenize differently; bundling tiktoken-rs would still be wrong for 3 of 4 vendors and adds weight). We therefore use a **calibrated chars-per-token heuristic**, the same family as Conductr's `CHARS_PER_TOKEN = 4` (`context-pack.ts:499,536-550`), but split by content class because code tokenizes denser than prose.

```rust
// brain/gist/budget.rs
const CHARS_PER_TOKEN_PROSE: f32 = 4.0;  // memory notes, freshness line, headings
const CHARS_PER_TOKEN_CODE:  f32 = 3.0;  // code snippets/skeletons (denser: punctuation, identifiers)
const DEFAULT_MAX_TOKENS: u32 = 2_000;   // gist budget: deliberately << Conductr's 8000 pane budget

pub fn estimate_tokens(pack: &ContextPack) -> u32
```

**Why 2000, not 8000:** the gist sits in the cacheable system-prompt prefix of *every turn*, not a one-shot pane answer. The budget is sized to be a cheap, durable map ("here's the lay of the land, go search"), not a content dump. This is the measurement-backed default; the calibration section (§3.6) can tune it.

**Calibration procedure (must be documented in the PR, not hand-waved):**
1. Build the gist for 20 representative `(project, query)` pairs across TS, Rust, and notes-heavy projects.
2. Send each rendered gist string through each vendor's **official** token counter offline (Anthropic `count_tokens` API for Claude; tiktoken `o200k_base` for Codex; published Gemini counter) — done **once at calibration time, never at runtime**.
3. Fit `chars/token` per class; pick the constant that makes `estimate_tokens()` a **conservative over-estimate** (we'd rather trim slightly early than overflow). Record the measured vs. estimated table in the PR.
4. Acceptance: `estimate_tokens()` is within +15% / −0% of the worst-case (densest) real vendor count across the corpus. (Over-estimate is safe; under-estimate risks overflow and is rejected.)

The estimate is what the toast and the budget gate read; it is **not** part of the byte-identical fingerprint (the rendered bytes are — see §3.4).

---

### 3.3 Cold-start query synthesis from ambient signals

The agent pane has **no user query** at launch — Conductr's `buildContextPack` always receives one. Phase 3's novel piece is synthesizing a query + intent purely from ambient signals captured by the GUI-resident worker.

```rust
// brain/gist/synthesis.rs
pub struct AmbientSignals {
    pub session_id: u32,             // KODEN_SESSION (session.rs:137)
    pub project: Option<ResolvedProject>, // pty->cwd->project (PTY leaf map + registry root-prefix)
    pub agent_name: Option<String>,  // AgentSignal.agent from "started" (agent_detect.rs:37-47)
    pub git_head: Option<String>,    // optional fast-path subprocess (ADR-006: git HEAD optional only)
    pub changed_files: Vec<String>,  // blake3 manifest delta since last index (P1)
    pub recent_files: Vec<String>,   // PTY-cwd recent edits from the watcher (P1)
    pub top_notes: Vec<String>,      // titles of highest-BM25 project notes (P0/P1)
}

pub struct SynthResult { pub query: String, pub intent: QueryIntent, pub signal_strength: u8 }
pub fn synthesize_query(s: &AmbientSignals) -> SynthResult
```

**Algorithm (deterministic — pure fn of `AmbientSignals`, no clock, no rng):**
```
signal_strength = 0
terms = []
if s.changed_files non-empty:   terms += basenames(top 5 changed files); strength += 2   // strongest: "what am I working on"
if s.recent_files non-empty:    terms += basenames(top 3 recent);        strength += 1
if s.top_notes non-empty:       terms += note title head-tokens (top 2);  strength += 1
intent =
    if s.agent_name == "claude"/"codex"/... AND no change/recent signal -> "prepare-agent-context"  // fresh onboarding
    else if s.changed_files non-empty -> "what-changed"     // mid-task relaunch
    else -> planRetrieval(query).intent                     // reuse ported planner (query-planner.ts:257)
query = terms (deduped, tokenizer-normalized via the P0 ported tokenizer lexical.ts:61) joined by space
```
Agent name maps to **intent**, never to query terms (the agent's identity is not a search term). `git_head` is used **only** as a cache-key salt (§3.4) and to fetch `changed_files` via the optional fast-path; if git is absent the blake3 manifest delta supplies `changed_files` (ADR-006 freshness rule: blake3 is primary, git optional).

`synthesize_query` is also reused by the Brain-pane manual path: a user-typed query bypasses synthesis and goes straight to `assemble_pack` — same code path, so pane and gist stay unified (the ADR-006 "one engine" payoff).

---

### 3.4 CRITICAL — cache-stable gist (byte-identical relaunch)

The gist is appended to the cacheable prompt prefix. If it mutates between two launches over **unchanged** code + notes, it busts the agent's prompt cache (~90% input-cost / ~80% latency hit — ADR-006 risk #1). The design guarantees byte-stability via a content-addressed fingerprint **and** a fully deterministic renderer.

#### 3.4.1 Fingerprint key

```rust
// brain/gist/fingerprint.rs
pub fn gist_fingerprint(p: &FingerprintInput) -> [u8; 32] // blake3
```
The key hashes, in fixed order, **only** content that should change the gist:
1. **Code state:** the P1 **sorted aggregate blake3 manifest** for the project (already the ADR-006 PRIMARY freshness signal — reuse it; do not recompute). Optionally salted with `git_head` when present (changes when HEAD moves even if working tree is byte-equal — desirable).
2. **Notes state:** sorted aggregate blake3 of the project's `MemoryNote` files (P1 store).
3. **Synthesized query string** (the exact normalized string from §3.3).
4. **`embedderId` header value** (deferred-semantic seam from ADR-006 — `"none"` in v1; included now so enabling semantic later auto-invalidates the cache).
5. **`GIST_SCHEMA_VERSION`** const (bumped whenever the renderer format changes, forcing rebuild).

**Excluded from the key (must be):** wall-clock time, `generatedAt`, `token_estimate`, `session_id`, the agent-`<id>` filename, any iteration over a `HashMap`. (Conductr's pack carries `generatedAt: new Date().toISOString()` at `context-pack.ts:1212` — that field is **dropped entirely** from the rendered gist; a timestamp in the prefix would bust cache every launch.)

#### 3.4.2 Deterministic renderer

```rust
// brain/gist/render.rs
pub fn render_gist(pack: &ContextPack) -> String
```
Rules enforced (each has a test in §3.7):
- All collections rendered in a **stable total order**: code files by `(−score, path)`, notes by `(−score, path)`, neighbors by `(kind, name, id)`. Ties broken by id (ADR-006 "deterministic id tie-break").
- **No timestamps, no absolute paths** (root-relative only — matches ADR-006 MegaSync portability), no run-specific ids, no float formatting drift (scores **not** rendered; ordering only).
- Fixed line endings (`\n`), fixed heading text, fixed separators.
- The freshness line is rendered from P1's blake3 digest, which is itself stable for unchanged content.

#### 3.4.3 Cache map + write path

`brain_build_gist` computes `fp = gist_fingerprint(...)`, looks it up in the local cache map at `app_local_data_dir()/koden/brain/gist-cache.sqlite` (one row: `fingerprint BLOB PRIMARY KEY, rendered TEXT, token_estimate INT, files INT, notes INT`). On hit → return the **stored bytes verbatim** (guarantees byte-identity even across schema-compatible code changes). On miss → assemble + render + store, then return.

`brain_write_gist` writes the rendered string to `~/.koden/agent-<id>.txt` via the existing `native.writeFile` path. **Idempotent write:** read the existing file first; if its bytes already equal the rendered string, skip the write entirely (no mtime churn — belt-and-suspenders for cache stability and for the watcher not to self-trigger).

---

### 3.5 Wiring — extend `App.tsx` + `agentCommand.ts`

The injection channel already exists and is vendor-agnostic; Phase 3 only changes **what string** gets written to `agent-<id>.txt`, not the launch mechanics.

**Current flow (verified):** `handleSpawnTerminalAgent` (`App.tsx:878-932`) writes the worker system prompt to `${dir}/agent-${req.agentId}.txt` (`App.tsx:909-910`) and launches with `--append-system-prompt "$(Get-Content -Raw <promptPath>)"` (`App.tsx:911,918`). `getAgentCommandWithArgs()` (`agentCommand.ts:49`) ensures flags survive the `cm` wrapper.

**Change:** before the write at `App.tsx:910`, call the brain to obtain the gist and **prepend** it to the worker prompt so the file content becomes `gist + "\n\n" + workerPrompt`:

```ts
// App.tsx handleSpawnTerminalAgent, before native.writeFile(promptPath, workerPrompt)
const gist = await invoke<GistResult | null>("brain_build_gist", {
  sessionId: req.sessionId,        // KODEN_SESSION for this leaf
  agentName: name,                 // -> intent
  projectRoot: inheritedCwdForNewTab(),
}).catch(() => null);              // FAIL-OPEN: null -> no gist, launch unchanged
const promptBody = gist ? `${gist.text}\n\n${workerPrompt}` : workerPrompt;
await native.writeFile(promptPath, promptBody);
if (gist) toast.info(`Gist injected: ${gist.files} files, ${gist.notes} notes, ~${Math.round(gist.tokens/1000)}k tokens`);
```

Ordering matters for cache: the **gist goes first** (most-cacheable, content-keyed) and the per-agent worker prompt second. The worker prompt is itself stable per role, so the concatenation is stable when the gist is. Manual/non-orchestration agent launches (the user typing `claude` themselves) are out of scope for v1 auto-injection — they can pull a gist via the Brain pane's "Copy gist" action.

`agentCommand.ts` needs **no change** — it already forwards `--append-system-prompt` correctly. We add **one** comment noting the file may now contain a brain gist prefix, and (optional) a typed `GistResult` export shared with `App.tsx`. The `kodenFunctionsPs1`/Director path (`App.tsx:1392-1396`) is left as-is for v1; Director gist injection is a P3-follow-on (the Director is one agent; the win is the subagents).

---

### 3.6 Confidence gate — never a speculative distractor

A wrong gist is worse than no gist: it pollutes the cache prefix and misleads the agent. Port the confidence heuristic (`context-pack.ts:578-588`) and add a **hard gate** on the synthesized signal strength.

```rust
// brain/gist/gate.rs
pub enum GateOutcome { Full(ContextPack), Thin(ContextPack), Empty } // Empty -> no file injected
pub fn confidence_gate(synth: &SynthResult, pack: &ContextPack) -> GateOutcome
```
Rules:
- `synth.signal_strength == 0` (no changed/recent/notes signal, only an agent name) → **`Empty`**: `brain_build_gist` returns `null`, the launch proceeds with the plain worker prompt, **no `agent-<id>.txt` brain content**, no toast. This is the "fresh project, cold start, nothing to say" case — we refuse to speculate.
- `signal_strength == 1` OR `computeConfidence(...) == Low` → **`Thin`**: freshness line + at most the top 1 changed/recent file's skeleton, no notes, no neighbors. A minimal "here's where things moved" hint.
- otherwise → **`Full`** pack within budget.

`computeConfidence` ported verbatim: `evidence = memoryCount + codeCount + neighborCount; 0→low; ≥4 or (mem≥1 && code≥1)→high; else medium` (`context-pack.ts:582-588`).

---

### 3.7 Tests + acceptance gates

**Unit / property tests (`brain/gist/tests/`):**

| Test name | Asserts |
|---|---|
| `gist_byte_identical_on_unchanged_relaunch` | **(P3 GATE)** assemble→render twice over the same fake index/notes/manifest → `bytes_a == bytes_b`; then mutate one note → bytes differ. |
| `gist_cache_hit_returns_stored_bytes` | second `brain_build_gist` with same fingerprint returns the row verbatim without re-rendering (mock render to panic on 2nd call). |
| `gist_fingerprint_ignores_clock_and_session` | same content, different `generatedAt`/`session_id`/agent filename → identical fingerprint. |
| `gist_fingerprint_changes_on_code_or_notes` | flip one byte in manifest, then in a note, then in query → 3 distinct fingerprints. |
| `confidence_gate_empty_on_weak_signal` | `signal_strength==0` → `GateOutcome::Empty` → command returns `None`. |
| `confidence_gate_thin_on_single_signal` | one changed file only → Thin pack: freshness + ≤1 code file, 0 notes, 0 neighbors. |
| `synthesize_query_deterministic` | same `AmbientSignals` → identical `SynthResult` across 1000 runs. |
| `synthesize_agent_name_sets_intent_not_terms` | agent name never appears in `query`; intent set correctly. |
| `assemble_pack_failopen_per_layer` | each layer's source returns Err in turn → pack still built, that layer empty, others intact (mirrors `context-pack.ts` per-layer try/catch). |
| `trim_keeps_freshness_line` | budget=1 token → only the freshness line survives (port of `context-pack.ts:734-748`). |
| `trim_deterministic` | same pack+budget twice → identical trimmed bytes. |
| `estimate_tokens_conservative` | over the calibration corpus, estimate ≥ real worst-case vendor count (never under). |
| `render_stable_order_under_shuffled_input` | shuffle input vecs → identical rendered bytes (total-order sort proof). |
| `write_gist_idempotent_skips_unchanged` | second `brain_write_gist` with equal bytes performs no write (mock fs records 0 writes). |
| `gist_no_absolute_paths` | rendered gist contains no path matching the OS abs-path pattern (root-relative only). |

**Integration / acceptance gate (P3 exit criteria, ADR-006 P3 row):**
1. **Functional:** a freshly spawned orchestration agent pane gets a relevant, ≤2k-token gist via `agent-<id>.txt`, assembled with **zero network/tokens** (assert no `reqwest` call; reflect is P4).
2. **Cache-safety (the gate):** launch agent → capture file bytes → exit → relaunch with no fs change → capture again → **assert byte-identical**. Then `touch`+edit one project file → relaunch → assert bytes differ. (`gist_byte_identical_on_unchanged_relaunch` is the unit proxy; this is the end-to-end proof.)
3. **Latency:** `brain_build_gist` returns < 50ms on a warm index (it is index reads + render, no walk); never blocks `whenSessionReady`.
4. **Fail-open:** with the brain disabled / index missing, `brain_build_gist` returns `null` and the launch is **byte-identical to today's** (`App.tsx:910` unchanged when gist is null).

---

### 3.8 Measurement plan — PROVE net token savings (skeptical)

The gist **only** saves tokens if the agent then **searches the brain** (cheap `brain_search`) instead of blindly re-reading files (expensive). It is entirely possible for a gist to *add* prefix tokens and change nothing. We measure, we don't assume.

**A/B protocol (run before declaring P3 a win):**
- **Control (gist OFF):** N=20 representative tasks per project, agent launched with worker prompt only.
- **Treatment (gist ON):** same N tasks, same seeds/models, gist prefix injected.
- **Instrument:** parse each agent's transcript for (a) tool-call counts of `Read`/`Glob`/`Grep` vs. `brain_search`, (b) total input tokens billed (from the usage poller's own accounting — reuse `usage/poll.rs` telemetry), (c) prompt-cache hit ratio on the system prefix (the cache-safety proof from §3.7 gate 2 is the prerequisite — if the prefix isn't stable, this whole comparison is invalid).

**Win conditions (all three required):**
1. Treatment shows **fewer redundant `Read` calls on already-summarized files** (the gist's skeleton should preempt re-reads).
2. Treatment's **net** input tokens (gist prefix cost included) are **lower or equal** at the same task success rate — i.e., the prefix pays for itself.
3. Prompt-cache hit ratio on the prefix ≥ 0.9 across relaunches (otherwise the prefix cost is paid in full every turn and the gist is a net loss — **kill the feature** rather than ship a cost regression).

**Honest reporting:** report the measured per-task delta with the negative-control (gist OFF) average alongside; if the gist does not move re-read behavior, report that and gate the feature default-OFF rather than claiming a vanity win. The toast (`Gist injected: N files, M notes, ~Xk tokens`) is a UX affordance, **not** evidence of savings — savings come only from the A/B above.


---

## Phase 4 — Budgeted LLM reflect + crash-resume

P4 adds the **only** token-spending path in Koden Brain (an opt-in, default-$0, budgeted `reflect` call) and the first durability surface (per-pane resume journals). Both are guarded extras layered on the keyless P0–P3 brain; if either is disabled or fails, the brain degrades to the deterministic doctor (P1) and cold-rehydrated tabs respectively. Nothing in P4 ever blocks first paint, ever writes memory without human approval, or ever spends a token without passing a hard pre-flight budget gate.

### 4.0 Module layout

```
src-tauri/src/modules/brain/
  reflect/
    mod.rs          // public: reflect_once(), ReflectOutcome
    budget.rs       // BudgetLedger: check → reserve → reconcile (crash-safe ordering)
    digest.rs       // bounded corpus digest (60 notes × 200 chars)
    llm.rs          // reqwest+rustls Anthropic call, block_on
    schema.rs       // serde structs + validation for the model's JSON output
    proposal.rs     // map validated output → MemoryProposal (reuse P1 queue type)
  resume/
    mod.rs          // public: ResumeJournal, record_event(), recover_all()
    journal.rs      // append-only JSONL writer (one file per sessionKey)
    sessionkey.rs   // SessionKey::derive(cwd, agent, pane_uuid)
    cursor.rs       // tail-cursor read-back (ported from AgentBusBridge tick loop)
    tier2.rs        // claude --resume capture + launch rewrite (feature-gated on capture)
```

`reflect` and `resume` are driven from the existing P0 worker thread (`brain::worker`, cloned from `usage::poll::spawn_poller` at `src-tauri/src/modules/usage/poll.rs:384`). The worker already owns the `BrainEvent` spine and `app.listen("koden:agent-signal")`; P4 adds two consumers of that spine — `resume::record_event` (synchronous, every signal) and a `reflect` trigger (manual-only in v1, never on a timer).

---

### 4.1 Budgeted LLM reflect

#### 4.1.1 What it is and where the idea comes from

Ported from Conductr's `src/lib/memory/reflect-llm.ts` (`C:/Users/Snorlax/Snorlax/Products/Conductr/src/lib/memory/reflect-llm.ts:1`). Conductr's locked constants — `MAX_NOTES = 60`, `MAX_NOTE_CHARS = 200`, `MAX_PROPOSALS = 8` (`reflect-llm.ts:28-30`) — and its core invariant (`reflect-llm.ts:18`: "The LLM NEVER writes anything. This function returns data only") carry over verbatim. The model proposes; a human approves via the same `MemoryProposal` queue built in P1. The output schema (`reflect-llm.ts:46-61`) becomes a serde-validated Rust struct.

The single deviation from Conductr: Koden calls the daemon's **own** Anthropic key from `secrets.rs` (keyring service `koden-ai`, mirroring how `secrets.rs` is consumed elsewhere — see `src-tauri/src/modules/secrets.rs:118` `secrets_get`), via `reqwest`+`rustls` and `tauri::async_runtime::block_on`, exactly like the usage poller's network call (`poll.rs:104` `build_client`, `poll.rs:422` `block_on(fetch_once(&client))`). No tokio `time` feature, one in-flight call at a time.

> **Provider facts (verified against the bundled `claude-api` skill, cutoff 2026-01):** model id `claude-opus-4-8` (or `claude-haiku-4-5` for the cheap reflect path — see budget note below). Endpoint `POST https://api.anthropic.com/v1/messages`, header `anthropic-version: 2023-06-01`, `x-api-key: <key>`. **Opus 4.8 / Haiku 4.5 take adaptive thinking only** — never send `thinking: {type:"enabled", budget_tokens:N}` (400) and never send `temperature`/`top_p` on Opus 4.8 (400). For strict JSON, use `output_config: {format: {type: "json_schema", schema: SCHEMA}}` — **not** assistant-prefill (prefill 400s on 4.8) and not the deprecated top-level `output_format`. Pricing for budget math: Opus 4.8 = $5/$25 per MTok in/out; Haiku 4.5 = $1/$5 per MTok.

#### 4.1.2 Signatures

```rust
// reflect/mod.rs
pub struct ReflectOutcome {
    pub proposals: Vec<MemoryProposal>, // [] on any fail-open path
    pub spent_usd: f64,                 // 0.0 when no call was made
    pub reason: ReflectReason,          // why we returned what we returned
}

pub enum ReflectReason {
    Ok,
    Disabled,            // budget ceiling == 0.0 (default)
    NoKey,               // keyring koden-ai empty
    OverBudget,          // pre-flight reserve would exceed ceiling
    EmptyCorpus,         // < MIN_NOTES notes to digest
    CallFailed(String),  // network/HTTP/timeout — fail-open to []
    InvalidOutput,       // serde/validation rejected the model's JSON
}

/// Manual-trigger only in v1. Never called on a timer.
/// `block_on`s the network call; returns within ~one call's latency.
pub fn reflect_once(app: &AppHandle, state: &BrainState) -> ReflectOutcome;
```

```rust
// reflect/budget.rs — the crash-safe spend ordering
pub struct BudgetLedger;   // backed by a row in the unified SQLite file

impl BudgetLedger {
    /// Atomically: read ceiling + spent_total, verify (spent + est_cost <= ceiling),
    /// INSERT a reservation row (status='reserved', est_cost, ts). Returns its rowid.
    /// On over-budget returns Err(OverBudget) BEFORE any network call.
    pub fn check_and_reserve(&self, conn: &Connection, est_cost_usd: f64)
        -> Result<i64, ReflectReason>;

    /// After the call returns: UPDATE the reservation row to status='spent',
    /// actual_cost = (input_tokens * in_rate + output_tokens * out_rate),
    /// and fold actual into spent_total in the SAME transaction.
    pub fn reconcile(&self, conn: &Connection, reservation_id: i64, actual_cost_usd: f64)
        -> Result<(), String>;

    /// Boot-time sweep: any row still status='reserved' (= crash mid-call) is
    /// reconciled to its est_cost (the conservative assumption — we charge the
    /// estimate, never zero, so a crash can't leak free spend) then marked 'spent'.
    pub fn sweep_orphaned_reservations(&self, conn: &Connection) -> Result<(), String>;
}
```

#### 4.1.3 SQL DDL (in the unified `brain.sqlite`)

```sql
CREATE TABLE IF NOT EXISTS brain_budget (
  id            INTEGER PRIMARY KEY CHECK (id = 1),  -- singleton row
  ceiling_usd   REAL NOT NULL DEFAULT 0.0,           -- DEFAULT-OFF: $0 = reflect disabled
  spent_total_usd REAL NOT NULL DEFAULT 0.0,
  updated_at    INTEGER NOT NULL
);
INSERT OR IGNORE INTO brain_budget (id, ceiling_usd, spent_total_usd, updated_at)
  VALUES (1, 0.0, 0.0, strftime('%s','now'));

CREATE TABLE IF NOT EXISTS brain_budget_ledger (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  status       TEXT NOT NULL CHECK (status IN ('reserved','spent')),
  est_cost_usd REAL NOT NULL,
  actual_cost_usd REAL,                              -- NULL until reconcile
  model        TEXT NOT NULL,
  reserved_at  INTEGER NOT NULL,
  reconciled_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_ledger_reserved
  ON brain_budget_ledger (status) WHERE status = 'reserved';
```

#### 4.1.4 The check-reserve-call-reconcile ordering (and why it can't leak the spent counter)

The ordering is the whole point — a crash at any step must never let a real call go uncharged.

```
1. ledger.check_and_reserve(est)         -- ONE write txn:
                                         --   verify spent_total + est <= ceiling
                                         --   else return OverBudget (no call made)
                                         --   INSERT ledger row status='reserved', est_cost=est
                                         --   COMMIT  ← reservation is DURABLE before the call
2. block_on(call_anthropic(...))         -- network; may crash the process mid-flight
3. ledger.reconcile(rid, actual)         -- ONE write txn:
                                         --   UPDATE ledger row status='spent', actual_cost=actual
                                         --   UPDATE brain_budget.spent_total += actual
                                         --   COMMIT
```

Failure analysis:
- **Crash between 1 and 3** (the dangerous window — money may have been spent on the API side): the reservation row is already committed as `status='reserved'`. On next boot, `sweep_orphaned_reservations` charges it at `est_cost` (conservative — we assume the call happened and cost roughly the estimate) and marks it `spent`. The spent counter therefore *over*-counts a crashed call rather than under-counting it. There is no path where a committed reservation disappears, so **the spent counter can never silently reset or leak**.
- **Crash before 1 commits**: no reservation, no call attempted (we reserve before calling) → correctly zero.
- **Over-budget**: detected inside step 1's read, returns `OverBudget` before any network I/O → spends nothing.
- **Call fails after reserve**: `reconcile(rid, 0.0)`? No — we still mark `spent` with `actual=0.0` only if the HTTP layer confirms no tokens were billed (e.g. a connect-timeout before request send). For any response (even an error body) we charge the estimate, because a 200-with-error or a mid-stream failure may have billed input tokens. Default to charging on uncertainty.

`est_cost_usd` is computed pre-call from the digest's token estimate: `est = (digest_tokens + SYSTEM_PROMPT_TOKENS) * in_rate + MAX_OUTPUT_TOKENS * out_rate`, using the per-model rates above. Conservative by construction (assumes max output).

#### 4.1.5 The reflect call body

```rust
// reflect/llm.rs (shape; real strings inline)
let body = json!({
    "model": cfg.model,                       // "claude-haiku-4-5" default for cost; opus opt-in
    "max_tokens": MAX_OUTPUT_TOKENS,          // 2048 — bounds est + actual
    "thinking": {"type": "adaptive"},         // NEVER budget_tokens (400 on 4.8/haiku-adaptive)
    "system": SYSTEM_PROMPT,                  // ported from reflect-llm.ts:33-39
    "output_config": {"format": {"type": "json_schema", "schema": LLM_PROPOSALS_SCHEMA}},
    "messages": [{"role":"user","content": user_digest}],  // digest.rs output
});
// reqwest client built like poll.rs:104; block_on like poll.rs:422.
// On 2xx: parse usage.input_tokens / usage.output_tokens for reconcile().
```

`SYSTEM_PROMPT` is Conductr's verbatim (`reflect-llm.ts:33-39`): "conservative memory librarian … SMALL set of high-confidence proposals (cap: 8) … Respond ONLY with a single JSON object." `LLM_PROPOSALS_SCHEMA` is the Rust/serde mirror of `reflect-llm.ts:46-61` — `{proposals: [{kind: insight|should_remember|stale|conflict, title, detail, scope: global|project, project?, confidence: low|medium|high, evidence?, ...}]}`. Because we use `output_config.format` (not prompt-coaxed JSON), validation is belt-and-suspenders, but `schema.rs` still hard-validates: any item missing a required field, or `proposals.len() > MAX_PROPOSALS`, drops the whole response to `[]` (`InvalidOutput`). **Never** `throw "string"` — `serde_json::from_str` in a guarded match, fail-open to `Ok(ReflectOutcome { proposals: vec![], reason: InvalidOutput, .. })`.

#### 4.1.6 Digest bounds (digest.rs)

Mirror `reflect-llm.ts:89` `buildDigest(docs)`: take up to `MAX_NOTES=60` memory notes from the FTS5 store (most-recently-touched first), truncate each to `MAX_NOTE_CHARS=200`, join with the doctor-findings summary (`reflect-llm.ts:90` `buildFindingsSummary(report)` — the deterministic P1 doctor's output). This caps the input token count and therefore the pre-flight estimate.

#### 4.1.7 Wizard budget step copy

The 3-step setup wizard from P1 gains a budget step (step 3b, optional, skippable). Exact copy:

> **Brain reflection (optional, off by default)**
> Koden Brain can occasionally ask an AI model to review your memory notes and *propose* cleanups — merging duplicates, flagging stale entries. It never edits anything itself; every suggestion lands in your review inbox for you to approve or reject.
> This is the only feature that spends money, and it uses **your own** Anthropic API key. It is **off** until you set a monthly ceiling.
> Monthly ceiling: `[ $0.00 ]` (leave at $0 to keep reflection off) · API key: `[ paste · stored in your OS keychain ]`
> Typical run with Haiku ≈ $0.002. A $1/month ceiling covers ~500 reviews.

Setting ceiling > 0 with no key shows: "Set an Anthropic key to enable reflection, or leave the ceiling at $0." Writes go to `brain_budget.ceiling_usd` + keyring `koden-ai`. No timer is ever scheduled — reflect is manual-trigger-only in v1 (a "Review memory now" button in the Brain pane).

---

### 4.2 Crash-resume — Tier 1 (events-only per-pane journal)

#### 4.2.1 Mechanism

On every `BrainEvent` derived from `koden:agent-signal` (started/working/attention/finished/exited, carrying the agent name — see `src-tauri/src/modules/pty/agent_detect.rs:37` `AgentSignal { id, kind, agent }` emitted at `src-tauri/src/modules/pty/session.rs:227`), the worker appends one JSONL line to `~/.koden/resume/<sessionKey>.jsonl`. This reuses `~/.koden/` (already ensured + fs-authorized by `App.tsx:381`) and the durable-JSONL-tail pattern proven by the orchestration bus.

```rust
// resume/journal.rs
#[derive(Serialize, Deserialize)]
pub struct ResumeRecord {
    pub ts: i64,
    pub kind: String,         // "started"|"working"|"attention"|"finished"|"exited"
    pub agent: Option<String>,// from AgentSignal.agent
    pub cwd: String,          // resolved pty→cwd at signal time
    pub project: Option<String>, // resolved via workspace registry root-prefix match
    pub claude_session_id: Option<String>, // populated only if Tier-2 capture fires (4.3)
}

pub fn record_event(app: &AppHandle, key: &SessionKey, rec: &ResumeRecord) -> Result<(), String>;
// Append-only: open with O_APPEND, write serde_json::to_string(rec) + "\n", flush.
// Fail-open: a write error is logged at debug and dropped (resume is best-effort).
```

#### 4.2.2 sessionKey — the critical dependency

```rust
// resume/sessionkey.rs
pub struct SessionKey(String); // hash or "<cwd>|<agent>|<paneUuid>", filesystem-safe

impl SessionKey {
    pub fn derive(cwd: &str, agent: &str, pane_uuid: &str) -> Self;
}
```

**`sessionKey = cwd + agent + PERSISTED pane uuid` — NOT the ephemeral `KODEN_SESSION`.** `KODEN_SESSION` is the per-pane pty id stamped at `session.rs:137` (`cmd.env("KODEN_SESSION", id.to_string())`) — it is a `u32` that does not survive a restart, so keying on it would make every resume journal an orphan on the next boot.

> **⚠️ OPEN DEPENDENCY — verified, and it currently blocks Tier 1 as specified.** There is **no stable persisted pane uuid in `orchestrationStore` today.** The store is explicitly session-scoped and not persisted: `C:/Users/Snorlax/Snorlax/Products/terax-workspace/src/modules/orchestration/store/orchestrationStore.ts:256-261` ("Orchestration state is intentionally session-scoped … persisting them would just resurrect stale, dead-linked entries"), and its ids are minted from `Date.now()+random` (`orchestrationStore.ts:15` `uid()`) — ephemeral by design. A grep for `persist(`/`createJSONStorage` across `src/modules` returns no terminal/tab/pane store; terminal panes/layout are not persisted via zustand `persist`. **Therefore P4 must FIRST introduce a stable, restart-surviving pane uuid** (a `crypto.randomUUID()` minted when a pane is first created and persisted in a new gitignored layout store, e.g. `~/.koden/panes.json`), then thread it down to `session.rs::spawn` as a new env var (`KODEN_PANE_UUID`) alongside `KODEN_SESSION`. Until that lands, Tier 1 keys can only fall back to `cwd+agent` (collides when two panes run the same agent in the same dir). This is a **prerequisite task, not optional polish** — call it P4-a.

#### 4.2.3 Read-back on boot (reuse the bus tail-cursor + tolerant recovery)

Recovery reuses, almost verbatim, the read-then-cursor loop that `AgentBusBridge` uses for the agent bus:
- the **line cursor + reset-on-shrink** logic at `C:/Users/Snorlax/Snorlax/Products/terax-workspace/src/modules/orchestration/components/AgentBusBridge.tsx:76-81` (`complete = lines.length - 1`; if the file shrank, reset cursor to 0 and re-read from the top), and
- the **tolerant, dedup-keyed recovery** of `subagentBus.ts::extractSubagentStarts` (`C:/Users/Snorlax/Snorlax/Products/terax-workspace/src/modules/orchestration/lib/subagentBus.ts:71`), which scans raw text and skips malformed/duplicate fragments rather than trusting line framing.

```rust
// resume/cursor.rs
pub fn recover_all(resume_dir: &Path) -> Vec<RecoveredPane>;
// For each <sessionKey>.jsonl: read whole file, split on '\n', drop the trailing
// partial line (AgentBusBridge.tsx:76), JSON-parse each complete line in a
// guarded match (skip un-parseable fragments — subagentBus.ts tolerance), and
// fold into a RecoveredPane { key, last_kind, last_agent, cwd, project, claude_session_id }.
```

#### 4.2.4 Boot recovery cards

On `.setup()` (after the worker spawns, fail-open), `recover_all` produces one `RecoveredPane` per journal whose `last_kind` is not `exited`. The frontend renders a **recovery card next to each cold-rehydrated tab**: "Claude was working here in `<project>` when Koden last closed. [Resume] [Dismiss]". `[Resume]` triggers the launch path (Tier 2 if a `claude_session_id` was captured, else Tier 1's plain re-launch in the same cwd). `[Dismiss]` deletes the journal. Cards are deterministic (driven only by journal contents) so the same boot always shows the same cards.

---

### 4.3 Crash-resume — Tier 2 (`claude --resume <id>`)

When the captured agent name is `claude` **and** a Claude session id was captured for that pane, `[Resume]` rewrites the launch command to `claude --resume <id> …` (threading through the existing launch path that already appends `--append-system-prompt` at `App.tsx:918`/`App.tsx:1376`). Otherwise it falls back cleanly to Tier 1 (plain re-launch in the recovered cwd, no `--resume`).

> **⚠️ OPEN DEPENDENCY — Claude session id is NOT reachable today.** A grep for `session_id`/`--resume`/`.claude/projects` across the orchestration bus (`bus.ts`) and `App.tsx` returns nothing relevant: the agent bus carries pty id (`KODEN_SESSION`), agent status, and Task `tool_use_id`s only (`AgentBusBridge.tsx:90-101`, `subagentBus.ts:25-33`) — **never the Claude session id.** Tier 2 therefore requires a NEW capture path that does not exist yet. Two candidate sources, in order of preference:
> 1. A Claude Code status-hook line on the existing `~/.koden/agent-bus.jsonl` carrying `session_id` (cheapest — Koden already tails this file; add a `claude_session_id` field to the agent-status payload and persist it into the resume journal at `record_event`).
> 2. Scanning `~/.claude/projects/<encoded-cwd>/*.jsonl` for the most-recent session matching the recovered cwd (fragile; CWD-encoding-dependent).
> v1 should attempt (1) and degrade to Tier 1 if absent. Tier 2 is **feature-gated on capture succeeding** — never emit `--resume` with an unverified id.

```rust
// resume/tier2.rs
pub fn resume_command(rec: &RecoveredPane, base_launch: &str) -> ResumePlan;
pub enum ResumePlan {
    Tier2 { command: String },  // base_launch with `--resume <id>` spliced in
    Tier1 { cwd: String },      // plain re-launch; no resume id available
}
```

---

### 4.4 Journal / proposal rotation policy

- **Resume journals** (`~/.koden/resume/*.jsonl`): an `exited` record is the terminal marker. On `[Dismiss]`, or once a pane is successfully resumed, the journal is deleted. A boot-time GC removes any journal older than `RESUME_TTL_DAYS = 7` (mtime) or whose `sessionKey` no longer maps to a known project. A single journal is hard-capped at `RESUME_MAX_LINES = 2000`; on overflow it is compacted to the last 200 lines (the recovery read-back only needs the tail, mirroring the bus's "tail-cursor" design — earlier lines are never read on recovery). Compaction is write-to-temp + atomic rename (the same pattern as `poll.rs:360` `std::fs::rename(&tmp, &path)`).
- **Proposal queue** (the gitignored P1 `MemoryProposal` queue): reflect output is appended, deduped by `proposalSignature` (ported from Conductr `proposal-scorer.ts`) so re-running reflect on an unchanged corpus never enqueues a duplicate. Approved/rejected proposals are moved to a terminal state (not deleted, for audit); pending proposals older than `PROPOSAL_TTL_DAYS = 30` are auto-expired to `rejected(stale)`.
- **Budget ledger**: rows are never deleted (audit trail); `brain_budget_ledger` rows older than `LEDGER_TTL_DAYS = 90` and `status='spent'` may be pruned by a boot sweep, but `spent_total_usd` is the authoritative running total and is never recomputed from rows.

---

### 4.5 Tests + gates (P4)

**Reflect / budget (Rust unit + integration):**
- `reflect_disabled_when_ceiling_zero` — default `ceiling_usd=0.0` → `ReflectOutcome{ reason: Disabled, spent_usd: 0.0 }`, no network client built. (Mirrors the gate "No key → spends nothing".)
- `reflect_no_key_is_deterministic_noop` — ceiling>0 but keyring `koden-ai` empty → `NoKey`, spends nothing.
- `budget_overbudget_blocks_before_call` — `check_and_reserve` returns `OverBudget` and an injected fake HTTP client asserts **zero** requests were made.
- `budget_reserve_then_reconcile_updates_spent` — happy path; `spent_total` increases by `actual_cost` exactly once.
- `budget_crash_midcall_does_not_leak_counter` — reserve, then drop the process before reconcile (simulate by leaving a `status='reserved'` row); `sweep_orphaned_reservations` on next open charges `est_cost`, marks `spent`, and `spent_total` reflects it. Asserts the counter is **over**-counted, never reset to a smaller value.
- `reflect_invalid_json_fails_open_to_empty` — fake client returns malformed/over-cap JSON → `proposals == []`, `reason: InvalidOutput`, and (per ordering) the call is still charged.
- `reflect_digest_respects_bounds` — corpus of 200 notes → digest contains ≤ 60 notes, each ≤ 200 chars.
- `reflect_body_uses_adaptive_thinking_and_output_config` — snapshot-asserts the request body has `thinking.type == "adaptive"`, no `budget_tokens`/`temperature`, and `output_config.format.type == "json_schema"` (guards against the 4.8 400-class regressions).

**Resume (Rust unit + integration):**
- `sessionkey_excludes_ephemeral_koden_session` — `SessionKey::derive` output is byte-identical across two derivations with the same `(cwd, agent, pane_uuid)` and **differs** when only a simulated `KODEN_SESSION`/pty id changes.
- `resume_journal_appends_per_signal` — N `BrainEvent`s → N JSONL lines in the right file.
- `resume_recover_drops_trailing_partial_line` — a journal ending without a newline recovers the same set as one with a trailing newline (ports `AgentBusBridge.tsx:76` semantics).
- `resume_recover_tolerates_garbage_lines` — interleaved/truncated lines are skipped, not fatal (ports `subagentBus.ts` tolerance).
- `resume_recover_resets_on_shrink` — a rotated/compacted (shorter) journal re-reads from the top (ports `AgentBusBridge.tsx:78-81`).
- `resume_gc_expires_old_and_orphan_journals` — TTL and unknown-project journals are removed.
- `tier2_falls_back_to_tier1_without_capture` — `RecoveredPane` with `claude_session_id: None` → `ResumePlan::Tier1`; with a captured id and `agent=="claude"` → `Tier2` whose command contains `--resume <id>`.

**Acceptance gates (must all pass to close P4):**
1. **No key → spends nothing, deterministic-only.** With `ceiling_usd=0` or empty keyring, `reflect_once` makes zero network calls and the brain still produces deterministic doctor proposals (P1 path unaffected).
2. **With key + ceiling → call blocked pre-flight when over.** Set a tiny ceiling already exhausted; `reflect_once` returns `OverBudget` and the spy HTTP client records zero requests.
3. **Mid-call crash doesn't leak the spent counter.** The `budget_crash_midcall_does_not_leak_counter` integration test passes: a reserved-but-unreconciled row is conservatively charged on next boot; `spent_total_usd` is monotonic and never silently resets.
4. **Resume keys survive restart.** A journal written under `SessionKey(cwd, agent, pane_uuid)` is recovered after a simulated restart (new pty ids) and produces a recovery card next to the matching cold-rehydrated tab; the ephemeral `KODEN_SESSION` plays no part in the key.
5. **Tier 2 is safe-by-default.** `--resume` is only ever emitted when a Claude session id was actually captured and `agent=="claude"`; all other cases use Tier 1, verified by `tier2_falls_back_to_tier1_without_capture`.

---

## Phase 5 — Deferred semantic seams

P5 is **decision-deferred**: in v1 it ships only the trait seams and an `embedderId` header so the SQLite schema and search path never churn when semantic is later turned on. The embedding stack (`fastembed-rs` + `hnsw_rs`) sits behind a **default-OFF cargo feature `semantic` that does not compile into the v1 binary**. There is no functional semantic search in v1 — only the shape that lets it slot in.

### 5.1 What lands now (compiles in v1)

```rust
// brain/search/vector.rs — trait seams only; no impl compiled in v1
pub trait Embedder: Send + Sync {
    fn id(&self) -> &str;                 // e.g. "bge-small-en-v1.5" — written to embedderId header
    fn dims(&self) -> usize;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

pub trait VectorStore: Send + Sync {
    fn embedder_id(&self) -> &str;
    fn upsert(&self, ids: &[DocId], vectors: &[Vec<f32>]) -> Result<(), String>;
    fn query(&self, vector: &[f32], k: usize) -> Result<Vec<(DocId, f32)>, String>;
}
```

- The unified `SearchIndex` trait (from P0) gains a hybrid hook so semantic can later join the RRF fusion as one more weighted leg — but in v1 the only legs are FTS5 BM25 over code and notes (P0). No vector leg is registered.
- **`embedderId` header**: a single string column persisted once in `brain.sqlite` so a future semantic build can detect a model/dimension mismatch and rebuild the vector index rather than serving stale embeddings.

```sql
CREATE TABLE IF NOT EXISTS brain_semantic_meta (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  embedder_id TEXT NOT NULL DEFAULT '',   -- empty in v1 (no embedder); set when 'semantic' is enabled
  dims        INTEGER NOT NULL DEFAULT 0,
  built_at    INTEGER
);
INSERT OR IGNORE INTO brain_semantic_meta (id, embedder_id, dims) VALUES (1, '', 0);
```

No `brain_vectors` table is created in v1 (it would be dead weight); it is created lazily only when the `semantic` feature is compiled and first enabled.

### 5.2 What is gated behind `feature = "semantic"` (does NOT compile in v1)

```toml
# Cargo.toml
[features]
default = []                       # semantic NOT in default — absent from the shipped binary
semantic = ["dep:fastembed", "dep:hnsw_rs"]

[dependencies]
fastembed = { version = "...", optional = true }   # version pinned at enablement time
hnsw_rs   = { version = "...", optional = true }
```

```rust
#[cfg(feature = "semantic")]
mod fastembed_embedder;   // impl Embedder via fastembed-rs (ONNX, local, no key for embedding)

#[cfg(feature = "semantic")]
mod hnsw_store;           // impl VectorStore via hnsw_rs, persisted under app_local_data_dir()/koden/brain/
```

Per ADR-006: even when compiled, semantic is enabled "only with key + visible budget" — but that gating is out of scope for v1; v1 only guarantees the seams exist and the feature is absent from the binary.

### 5.3 Tests + gates (P5)

- `semantic_feature_absent_from_default_build` — a CI job runs `cargo build` (no `--features semantic`) and a test asserts `cfg!(feature = "semantic") == false`; the binary does not link `fastembed`/`hnsw_rs` (verify via `cargo tree -e features` in CI showing they're absent from the default feature set).
- `embedder_id_header_persisted_empty_in_v1` — fresh `brain.sqlite` has `brain_semantic_meta.embedder_id == ''` and `dims == 0`.
- `search_index_has_no_vector_leg_in_v1` — the registered RRF legs are exactly {FTS5-code, FTS5-notes}; no vector leg.
- `semantic_feature_compiles` — a **separate** CI job runs `cargo build --features semantic` to prove the gated code stays compilable (catches bit-rot) without ever shipping it in the default binary.

**Acceptance gates (P5):**
1. **Deferred by decision — no functional semantic search in v1.** The default build contains no embedding/vector code; `semantic_feature_absent_from_default_build` passes.
2. **Seams are real and stable.** `Embedder`/`VectorStore` traits + `embedderId` header exist and are persisted; turning on the feature later requires no v1 schema migration of the FTS5/AST/notes/ledger tables (the only new object is `brain_vectors`, created lazily).
3. **Gated code does not rot.** `cargo build --features semantic` succeeds in CI even though it never ships.


---

## Cross-Cutting Concerns: Testing, CI, Risk, Open Decisions & Peer-Review Gate

This section is engine-agnostic across phases. It defines how Koden Brain proves correctness, what CI enforces, where the project can fail, and what a reviewer must confirm before P0 implementation begins. All paths are relative to `src-tauri/` unless absolute. Tests live in `src-tauri/src/modules/brain/**/tests` (unit, `#[cfg(test)]` inline) and `src-tauri/tests/brain_*.rs` (integration), matching the existing crate layout consumed by `cargo nextest`.

---

### 1. Testing Strategy

The brain has a hard correctness asymmetry: the **derived SQLite index is rebuildable** (so we can tolerate corruption by rebuilding), but the **gist injection prefix is prompt-cache-load-bearing** (a single non-deterministic byte costs ~90% input price on relaunch). The test pyramid is therefore weighted toward two invariants: *incremental == full rebuild* (index correctness) and *byte-identical gist* (cache safety). Everything else is conventional.

All Rust tests run under `cargo nextest run --locked` (the existing runner; see `.github/workflows/ci.yml` `rust` job). Property tests use `proptest = "1"` (net-new dev-dependency). Benchmarks use `criterion = "0.5"` (net-new dev-dependency) under `[[bench]]` targets, run manually and in a dedicated CI job — never gating PRs on wall-clock in shared runners except via the relaxed budget below.

#### 1.1 Layers

| Layer | Crate/tool | Scope | Example test names |
|---|---|---|---|
| Unit | `#[cfg(test)] mod tests` | Tokenizer, BM25/IDF math, RRF fusion, blake3 manifest diff, BrainEvent folding, path portability | `tokenizer_keeps_whole_and_parts`, `tokenizer_additive_stem_both_forms`, `bm25_k1_b_matches_reference`, `rrf_weighted_legs`, `manifest_diff_detects_single_file_change` |
| Integration | `tests/brain_*.rs` | Full index build over a fixture repo, `brain_search` end-to-end through `BrainState`, watcher→reindex round-trip, wizard/registry persistence | `brain_search_returns_code_and_notes`, `watcher_reindexes_only_changed_file`, `cold_build_from_committed_source` |
| Property | `proptest` | incremental==full-rebuild (P2), tokenizer determinism, gist byte-stability under field reordering, RRF monotonicity | `prop_incremental_equals_full_rebuild`, `prop_gist_byte_identical_unchanged_inputs`, `prop_tokenize_deterministic` |
| Perf/bench | `criterion` | <150ms search budget, cold-warm time, incremental delta latency, gist build latency | `bench_search_p95_under_150ms`, `bench_cold_warm`, `bench_incremental_delta` |
| Fail-open | integration | Corrupt SQLite, truncated manifest, malformed resume JSONL, missing keyring entry, budget-exhausted | `corrupt_index_rebuilds_clean`, `truncated_manifest_triggers_full_rescan`, `garbage_jsonl_recovers_tolerant`, `no_key_spends_zero` |

#### 1.2 Scratch-HOME pattern (mandatory; never touch real `~/.koden` or `~/.claude`)

Every test that resolves a home-relative path (registry at `<root>/.koden-brain/`, gist at `~/.koden/agent-<id>.txt`, resume journals `~/.koden/resume/<sessionKey>.jsonl`, seed importers reading `~/.claude`/`~/.codex`/`~/.gemini`, keyring) MUST run inside an isolated scratch HOME. This mirrors the global rule *"Isolate HOME when testing deploy commands"* (the 2026-06-08 incident where `generate --global` clobbered the real `~/.claude`).

Implementation (shared test helper `tests/support/scratch_home.rs`):

```rust
/// Returns a TempDir whose path is installed as HOME/USERPROFILE/app_local_data_dir
/// for the duration of the test. Brain code MUST read its base dirs through a single
/// injectable `BrainPaths` struct — never call dirs::home_dir() directly — so the
/// scratch dir fully sandboxes it.
pub struct ScratchHome { _tmp: tempfile::TempDir, pub paths: BrainPaths }

pub fn scratch_home() -> ScratchHome { /* tempdir + BrainPaths::for_root(tmp) */ }
```

Design rule that makes this enforceable: **no module under `brain/` may call `dirs::home_dir()`, `std::env::var("HOME")`, or `app.path().app_local_data_dir()` directly.** All base-path resolution goes through one `BrainPaths` value threaded from `BrainState`. A clippy `disallowed-methods` lint (clippy.toml) bans the direct calls so the sandbox can't be bypassed. The keyring is sandboxed by gating any real `keyring::Entry` behind a `SecretStore` trait whose test impl is an in-memory map (the production impl wraps `secrets.rs::entry`, `secrets.rs:111`).

#### 1.3 The <150ms search budget benchmark

- **Fixture:** a synthesized repo of ~MAX_FILES (reuse `fs/search.rs` `MAX_SCANNED = 50_000` cap as the ceiling, `search.rs:30`; default fixture ~5,000 files / ~20 MB realistic TS+Rust mix) generated deterministically by a seeded generator so the corpus is reproducible across machines.
- **Measure:** p50 and p95 of `brain_search(query, k=20)` over a 50-query workload (mix of identifier, multi-word, and stemmed queries), warm index, single-threaded. criterion bench `bench_search_p95_under_150ms`.
- **Gate (PR-blocking, relaxed):** p95 < 150ms is the *design target*; the CI gate asserts p95 < 300ms (2× headroom) to absorb shared-runner noise, while the criterion report records the true number for trend-tracking. A regression > 25% vs the committed baseline (`benches/baseline.json`) fails the perf job.
- Rationale for two thresholds: the 150ms number is a product promise measured on dev hardware; a hard 150ms gate on `ubuntu-latest`/`windows-latest` would flake. The reviewer must confirm this split is acceptable (Open Decision OD-7).

#### 1.4 The byte-identical-gist test (P3 cache-safety gate)

This is the single most important behavioral test. The gist is written to `~/.koden/agent-<id>.txt` and consumed via `--append-system-prompt`; it lands in the **cacheable prompt prefix**.

```rust
// tests/brain_gist_stable.rs
#[test]
fn gist_byte_identical_on_unchanged_inputs() {
    let home = scratch_home();
    let fixture = build_fixture_project(&home);          // deterministic
    index_project(&home, &fixture);
    let g1 = build_gist(&home, &gist_inputs(&fixture));   // first launch
    let g2 = build_gist(&home, &gist_inputs(&fixture));   // relaunch, nothing changed
    assert_eq!(g1.as_bytes(), g2.as_bytes());            // byte-for-byte
}
```

Plus a property test `prop_gist_byte_identical_unchanged_inputs` that feeds the gist builder the same `GistInputs` with internal collections in shuffled iteration order, asserting output is byte-stable. This forces the implementation to **sort all collections deterministically** and to **key the gist on the fingerprint** (blake3 aggregate of the files+notes that fed it) so the build is a pure function of (fingerprint, intent, caps). Companion negative test `gist_changes_when_fingerprint_changes` asserts that touching one indexed file *does* change the gist (so we're not byte-stable by accidentally emitting a constant). A third test asserts no wall-clock timestamps, no HashMap iteration order, and no absolute machine paths leak into the bytes (`gist_contains_no_timestamps_or_abs_paths`).

#### 1.5 The incremental == full-rebuild property test (P2 graph correctness)

```rust
// proptest: apply a random sequence of file edits (add/modify/delete/rename),
// drive the incremental indexer event-by-event, then build a fresh full index
// from the final tree, and assert the two are observationally identical.
proptest! {
  #[test]
  fn prop_incremental_equals_full_rebuild(ops in edit_sequence_strategy()) {
    let inc = apply_incremental(ops.clone());   // watcher path
    let full = build_full(final_tree(&ops));     // ignore::WalkBuilder path
    prop_assert_eq!(normalize(inc.fts_postings()),  normalize(full.fts_postings()));
    prop_assert_eq!(normalize(inc.graph_forward()), normalize(full.graph_forward()));
    prop_assert_eq!(normalize(inc.graph_reverse()), normalize(full.graph_reverse())); // reverse adjacency is the bug-prone one
    prop_assert_eq!(inc.manifest(), full.manifest());
  }
}
```

`normalize()` sorts rows and drops volatile columns (rowids, mtimes). The reverse-adjacency equality is the crux — incremental relink that forgets to delete stale reverse edges on a rename is the classic correctness break (Risk R4). Begin this as a P0-lite version over FTS postings only (no graph) so the harness exists before P2 lands.

#### 1.6 Fail-open tests

Fail-open is an architectural promise ("never blocks first paint", ".setup() fail-open"). Each failure mode gets a test that asserts (a) no panic propagates to the worker/`.setup()`, (b) the brain degrades to a known-good state, (c) an error is logged once.

- `corrupt_index_rebuilds_clean` — write garbage bytes into the SQLite file, start brain → it detects (PRAGMA integrity_check or open error), renames the bad file to `*.corrupt-<ts>`, cold-rebuilds from committed source. `brain_index_status` reports `rebuilding`.
- `truncated_manifest_triggers_full_rescan` — half-written fingerprint manifest → treated as empty → full rescan, not a crash.
- `garbage_jsonl_recovers_tolerant` — resume journal with a torn last line → parse line-by-line, drop the unparseable tail, resume from last good record (port the tolerant-tail recovery from `subagentBus.ts` / `AgentBusBridge.tsx`).
- `no_key_spends_zero` and `budget_ceiling_blocks_preflight` — reflect with no keyring entry spends nothing and returns deterministic-only; with a ceiling already exceeded, the call is blocked **pre-flight** (before reqwest), and the spent counter is unchanged after a simulated mid-call crash (`crash_midcall_does_not_leak_counter`).
- `watcher_failure_falls_back_to_periodic` — if the recursive `notify` watcher fails to arm (e.g. inotify exhaustion, Risk R5), brain logs and falls back to a low-frequency `WalkBuilder` rescan instead of dying.

---

### 2. CI Additions

Extend the existing `.github/workflows/ci.yml`. The `rust` job already runs `cargo check --all-targets --locked`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo machete`, and `cargo nextest run --locked`; the `rust-platforms` matrix runs check+nextest on `windows-latest` + `macos-latest`; the `frontend` job runs `pnpm size` (size budget). New work plugs into this structure — do **not** invent a parallel pipeline.

#### 2.1 tree-sitter grammar smoke-parse job (lands with P2)

New job `brain-grammars` (or a step appended to `rust`):

```yaml
  brain-grammars:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
      - uses: swatinem/rust-cache@v2
        with: { workspaces: ./src-tauri -> target }
      - uses: taiki-e/install-action@v2
        with: { tool: cargo-nextest }
      - name: Grammar smoke-parse (TS/JS/Rust)
        working-directory: src-tauri
        run: cargo nextest run --locked --features brain-grammar-smoke grammar_smoke_
```

Backed by a `#[test]` per language (`grammar_smoke_typescript`, `grammar_smoke_tsx`, `grammar_smoke_javascript`, `grammar_smoke_rust`) that loads the pinned grammar, parses a checked-in fixture under `src-tauri/tests/fixtures/grammars/<lang>/`, and asserts (a) the parse tree has **zero ERROR nodes**, (b) the language ABI version is within the core `LANGUAGE_VERSION` range (catches ABI drift, Risk R3), and (c) the `.scm` def/import/ref/call queries each match the expected count. This is the early-warning for grammar bumps.

#### 2.2 Binary-size budget check (lands with P0; tightens at P2)

tree-sitter C grammars + bundled SQLite on the release/LTO profile are the size risk (Risk R2). New step in a release-profile job:

```yaml
      - name: Binary size budget
        working-directory: src-tauri
        run: cargo build --release --locked && \
             scripts/check-binary-size.sh target/release/koden  # exits 1 if > BUDGET
```

`scripts/check-binary-size.sh` reads a committed budget (`src-tauri/.size-budget` → bytes) and compares the **stripped** release binary. Record both pre-P2 and post-P2 baselines; the post-P2 delta attributable to grammars+SQLite must be reported in the PR. Budget guidance: cap the brain-attributable growth at +12 MB stripped (SQLite ~1.5 MB + 3 grammars ~3–6 MB + tree-sitter core), with the absolute budget set from the measured P0 baseline + headroom. Mirror this with a frontend `pnpm size` line if any brain UI ships JS.

#### 2.3 clippy -D warnings (already enforced — extend, don't weaken)

The `rust` job already runs `cargo clippy --all-targets --locked -- -D warnings`. The brain code must pass it with **no `#[allow(...)]` escape hatches** added to silence the new lint config. Add a `clippy.toml` `disallowed-methods` entry for `dirs::home_dir`, `std::env::var` on `HOME`/`USERPROFILE`, and direct `keyring::Entry::new` outside `secrets.rs`/the `SecretStore` impl, so the scratch-HOME sandbox (§1.2) is statically enforced and any bypass fails CI.

#### 2.4 Property/perf placement

- Property tests run inside the normal `cargo nextest` set (proptest cases capped via `PROPTEST_CASES` env in CI to keep runtime bounded; default 256 locally, 64 in CI).
- The criterion perf benches run in a **separate, advisory-by-default** job `brain-perf` (not in `needs:` of merge gating except for the relaxed 300ms p95 assertion + 25%-regression check from §1.3). Baseline JSON committed under `src-tauri/benches/baseline.json`; the job posts the delta but only the two relaxed assertions block.

---

### 3. Risk Register

Ordered by severity. Owner = the role accountable, not a named person (single-maintainer project; "Lead" = Kosta).

| # | Risk | Phase | Mitigation | Owner |
|---|---|---|---|---|
| **R1** | **Prompt-cache busting** — the gist sits in the cacheable prompt prefix via `--append-system-prompt`; any per-launch byte drift (timestamps, HashMap order, abs paths, float formatting) invalidates the cache → ~90% input-cost / ~80% latency penalty on every relaunch. | **P3** | Gist build is a **pure function of (blake3 fingerprint, intent, caps)**; all collections sorted; no clocks, no abs paths, fixed number formatting. **Fingerprint-keyed**: same fingerprint → cached prior bytes reused, never rebuilt. Enforced by `gist_byte_identical_on_unchanged_inputs` (§1.4) + property test + the no-timestamps/no-abs-paths test. P3 gate is hard-blocking. | Lead |
| **R2** | **Binary size / compile time** — bundled SQLite + 3 tree-sitter C grammars on an LTO-fat release profile inflate the binary and slow `cargo build`. | P0→P2 | Chose SQLite over tantivy and 3 grammars (not N). Strip release binary. CI size-budget gate (§2.2) with committed baselines + per-PR delta reporting. Grammars behind no extra features but kept minimal; defer Python/Go. | Lead |
| **R3** | **tree-sitter ABI drift** — grammar crate bumps can move the ABI/node-type surface, silently breaking `.scm` queries. | P2 | Pin every grammar crate to an exact version in `Cargo.toml` (`=x.y.z`) inside `--locked` CI; pin to core `LANGUAGE_VERSION` range; grammar smoke-parse job (§2.1) asserts zero ERROR nodes + ABI-in-range + query match counts on every PR. | Lead |
| **R4** | **Incremental relink correctness** — incremental graph updates that fail to delete stale **reverse** adjacency on rename/delete diverge from a full rebuild, corrupting `code_impact`. | P2 | Maintain reverse adjacency explicitly; delete-then-insert on every file event; `prop_incremental_equals_full_rebuild` (§1.5) asserts forward AND reverse equality vs full rebuild over random edit sequences (incl. renames). Harness seeded in P0 over FTS postings. | Lead |
| **R5** | **Two-watcher inotify exhaustion** — existing `fs/watch.rs` (NonRecursive, per-open-dir; `watch.rs:186`) plus the new recursive brain watcher can double-watch and exhaust Linux inotify handles. | P1 | Clear ownership split: `fs/watch.rs` keeps editor-open-dir duties only; brain owns project-root recursive watching. Reuse SKIP_DIRS (`watch.rs:19`) to prune. Detect arm-failure and fall back to periodic `WalkBuilder` rescan (`watcher_failure_falls_back_to_periodic`, §1.6). Document the split in `brain/README.md`. | Lead |
| **R6** | **Own-key budget overshoot** — the reflect path spends real money on the daemon's key; a race or crash could over-spend or leak the counter. | P4 | Strict **check → reserve → call → reconcile** ordering: reserve budget before reqwest, reconcile after. Pre-flight block when over ceiling (before any network). Default ceiling $0 (off). Tests `budget_ceiling_blocks_preflight`, `crash_midcall_does_not_leak_counter`, `no_key_spends_zero`. Reflect "only ever PROPOSES". | Lead |
| **R7** | **Registry path portability (MegaSync)** — absolute paths in the committed registry/notes break on machine #2 (the global rule: store root-relative). | P0/P1 | Registry + notes store **root-relative portable paths only**; resolve to absolute at load against the current root. Derived SQLite stays local under `app_local_data_dir()/koden/brain/` and never travels. Unit test `registry_paths_are_root_relative` + `cold_build_from_committed_source` on a moved-root fixture. | Lead |
| **R8** | **Resume sessionKey durability** — Tier-1 resume keys on `cwd + agent + persisted pane uuid`; if the pane uuid isn't actually durable in `orchestrationStore`, resume silently misbinds or loses sessions. | P4 | Confirm a stable persisted pane uuid exists (OD-3) before building Tier-1. Journal is append-only JSONL with tolerant-tail recovery (port from `subagentBus.ts`). Test `resume_rebinds_same_session_after_restart` + `garbage_jsonl_recovers_tolerant`. | Lead |
| **R9** | **Freshness blind spots** — blake3 manifest can miss changes if the watcher drops events (mass git checkout, OS event coalescing) or if mtime-only shortcuts are taken. | P1 | blake3 **content** hash is primary (not mtime); periodic reconciliation rescan via `WalkBuilder` re-hashes and repairs drift on an interval and on focus/startup. Git HEAD is an *optional* fast-path hint only, never the source of truth. Test `mass_change_reconciled_by_periodic_rescan`. | Lead |

---

### 4. Open Decisions

Non-blocking for P0 unless marked. Each needs a recorded answer in ADR-006 before the dependent phase.

- **OD-1 (folder names):** Confirm canonical names — proposed `<root>/.koden-brain/` (registry) + `<project>/.koden-memory/*.md` (notes). Native naming, no `.conductr`/`.rulesync`. *Blocks P0 registry layout.*
- **OD-2 (projects outside root):** May registered projects live outside the workspace root? The registry already authorizes home + launch dir, so technically yes — decide whether to expose this in the wizard. *Affects P0 registry + path portability (R7).*
- **OD-3 (resume sessionKey source):** Is there a stable persisted pane uuid in `orchestrationStore`? Is the Claude session-id reachable from the bus for Tier-2 (`claude --resume`)? *Blocks P4 resume (R8).*
- **OD-4 (worker listen path):** Does the Rust brain worker `app.listen()` `koden:agent-signal` directly, or is it webview-forwarded? Direct preferred (matches `spawn_poller`, `poll.rs:384`). *Affects P0 worker wiring.*
- **OD-5 (external daemon consumers):** Does any external process ever consume the `brain_*` command API, or is it strictly GUI-resident in-process? Default: strictly in-process. *Affects command surface design.*
- **OD-6 (reflect cadence/budget defaults):** Default-OFF + manual-trigger-only recommended; confirm cadence and ceiling defaults. *Blocks P4.*
- **OD-7 (perf gate thresholds):** Accept the 150ms design-target vs 300ms-CI-gate + 25%-regression split (§1.3)? *Affects CI before P0 perf job.*
- **OD-8 (FTS5 weighting + tokenizer integration):** **This is a real unresolved design fork that the reviewer must resolve before P0 codes search.** FTS5 `bm25()` has **fixed k1=1.2/b=0.75** (matching our target — good) but exposes only **per-column weights**, not arbitrary BM25 reweighting. Two viable paths, pick one and record it:
  - **(A) FTS5-native:** columns `path`, `symbols`, `content`; call `bm25(fts, 3.0, 1.5, 1.0)` to get path-3x weighting; integrate the ported tokenizer as a **pre-tokenization pass** (we tokenize in Rust and store the expanded token stream into a shadow column indexed with FTS5's `unicode61` removing extra splitting), since writing an external FTS5 tokenizer in Rust via rusqlite is awkward. Pro: BM25 is C-side and fast. Con: per-column granularity only; pre-tokenization means the stored column is the expanded form (must store both display + indexed forms).
  - **(B) Manual BM25 over FTS5 postings:** use FTS5 only as the inverted index / candidate generator, then compute BM25 (K1=1.2/B=0.75, our IDF) + path-3x in Rust over the postings, giving full control + exact RRF weighting. Pro: total control, matches the borrowed ranking spec exactly, custom tokenizer trivially applied identically to code+notes. Con: more Rust, slower than C bm25 (must still hit <150ms — validate in bench).
  Recommendation to the reviewer: **(B)** for ranking fidelity (the spec demands exact K1/B/IDF + weighted RRF, which FTS5's fixed bm25 + column-weights cannot fully express), with the ported tokenizer applied as a pre-tokenization pass feeding a `unicode61(tokenchars '')`-style FTS5 column used purely as the postings store. *Blocks P0 search core.*
- **OD-9 (gist caching store):** Where is the prior-gist cache keyed by fingerprint stored — in the SQLite ledger table or alongside `~/.koden/agent-<id>.txt`? *Affects P3 + R1.*

---

### 5. Peer-Review Checklist

A reviewer must verify each item against the real tree before approving the plan. Citations are the load-bearing primitives.

**Primitive reuse (confirm line refs still hold):**
- [ ] `usage/poll.rs::spawn_poller` (`poll.rs:384`) is the worker template; the brain worker is a named `std::thread`, fail-open, started from `lib.rs .setup()` **after** the usage poller. Confirm `installed_cli_version()` (`poll.rs:58`) pattern is mirrored only if needed.
- [ ] `fs/watch.rs` constants reused: `DEBOUNCE = 150ms` (`watch.rs:14`), `MAX_WINDOW = 1000ms` (`watch.rs:15`), `SKIP_DIRS` (`watch.rs:19`). Confirm existing watcher is `RecursiveMode::NonRecursive` (`watch.rs:186`) so the brain's recursive watcher is genuinely net-new and the ownership split (R5) is real.
- [ ] `fs/search.rs` bounds reused for initial `WalkBuilder` population: `MAX_SCANNED = 50_000` (`search.rs:30`), and the 256 KB-ish per-file ceiling. Confirm the brain honors these.
- [ ] `secrets.rs` keyring service is reachable via `entry(service, account)` (`secrets.rs:111`) with service `koden-ai`; the brain wraps it behind a `SecretStore` trait so tests use an in-memory impl.
- [ ] `pty/agent_detect.rs` `Transition` enum (`agent_detect.rs:28`) emits `started/working/attention/finished/exited` with the agent name carried on `Started` (`agent_detect.rs:46`); the brain folds `koden:agent-signal` into its `BrainEvent` spine.
- [ ] Tolerant JSONL tail recovery is portable from `orchestration/lib/subagentBus.ts` + `orchestration/components/AgentBusBridge.tsx` for resume journals.
- [ ] CI structure matches `ci.yml`: `rust` job already does clippy `-D warnings`, nextest, machete; `rust-platforms` matrix is windows+macos; `frontend` does `pnpm size`. New jobs plug in, not replace.

**Conductr mechanism fidelity (confirm the port matches source):**
- [ ] Tokenizer matches `Conductr src/lib/search/lexical.ts:61` `tokenize()` — lowercase, `[A-Za-z0-9]+` split, `splitCamel` keeps whole token AND parts, additive `stemLight` both-forms (`lexical.ts:101-126`: `ation→ate`, `ated→ate`, `ion→base`, `ed→`, `ied→y`), 50-word stoplist, drop len<2. Applied **identically to code and notes**.
- [ ] AST graph is the upgrade over Conductr's regex extraction (`indexer.ts:21-27`: `TS_JS_EXPORT_RE`, `BARE_FUNCTION_CLASS_RE`, etc.) — confirm tree-sitter captures real defs incl. methods/re-exports/arrow-consts, real imports/refs/calls, not regex.
- [ ] RRF drops Conductr's duplicate-the-list hack and uses a **first-class per-leg weight param** with k=60.
- [ ] BM25 K1=1.2/B=0.75 + IDF=`log(1+(N-df+0.5)/(df+0.5))` + multiplicative recency re-rank + deterministic id tie-break.

**Questions to answer before P0 starts:**
- [ ] **OD-8 resolved?** FTS5-native (A) vs manual-BM25-over-postings (B) — the entire search core depends on this. Has someone validated path (B) hits <150ms on the bench fixture, or confirmed (A)'s column-weight approximation is acceptable?
- [ ] **OD-1 resolved?** Folder names locked so P0 registry path code isn't churned.
- [ ] **OD-4 resolved?** Direct `app.listen()` in the Rust worker confirmed reachable (vs webview-forward).
- [ ] Is the scratch-HOME `BrainPaths` injection + clippy `disallowed-methods` ban agreed, so no test can touch real `~/.koden`/`~/.claude`?
- [ ] Is the perf gate split (150ms target / 300ms CI / 25% regression) accepted (OD-7)?
- [ ] Is the binary-size budget number + baseline-capture method (§2.2) agreed before grammars land?
- [ ] Is the P0-lite incremental==full-rebuild harness (FTS postings only) committed in P0, so the property test isn't deferred whole to P2?
- [ ] Confirm net-new deps + pins: `rusqlite` (bundled+FTS5), `tree-sitter` + exactly-pinned TS/JS/Rust grammar crates, `blake3`, `serde_yaml`, `tauri-plugin-dialog`, dev-deps `proptest`/`criterion`/`tempfile`. All under `--locked`.


---

## Author-flagged open items (v1)

- FTS5 availability via rusqlite 'bundled': ADR-006 asserts bundled SQLite includes FTS5, but the bundled amalgamation must be compiled with SQLITE_ENABLE_FTS5 — verify with a build-time smoke test (CREATE VIRTUAL TABLE ... USING fts5) before locking P0; if absent, add the 'bundled-full' feature or a build flag. This is a peer-review blocker.
- Live foreground cwd: Session (pty/session.rs:114) stores only the spawn cwd, not the leaf process's live cwd. An agent that cd's into a subproject resolves to the spawn dir in v1. Confirm 'spawn cwd is sufficient for v1' with reviewer, or scope a platform-specific leaf-cwd reader (/proc on Linux, GetProcess on Win) as a P1+ enhancement.
- app.listen from a std::thread worker: confirm AppHandle::listen is callable off the main thread in Tauri 2 and that the callback runs on a runtime thread (not the worker), so the callback only sends on the mpsc channel. If listen must be registered on the main thread, register it in .setup() and capture the Sender there.
- Canonical folder names (<root>/.koden-brain/ registry + <project>/.koden-memory/*.md) are PROPOSED in ADR-006, not locked. Confirm before P1 writes anything to disk.
- BrainEvent::Agent uses spawn cwd via PtyState; the brain reads PtyState across module boundary — confirm BrainState can obtain tauri::State<PtyState> from the worker's AppHandle (app.state::<PtyState>()) the same way poller_loop does app.state::<UsageState>() (poll.rs:409).
- Exact crate versions (rusqlite 0.31, tree-sitter 0.22, grammars 0.21) are best-known as of plan date; pin against the actual Cargo.lock and tree-sitter LANGUAGE_VERSION range during P0/P2 setup — grammar ABI must match the core crate.
- Whether any external daemon ever consumes the brain_* command API (ADR-006 open Q). Plan assumes strictly GUI-resident in-process; flag if a headless consumer is wanted later (would change the command surface to a lib API).
- FTS5 external-tokenizer integration: Conductr's additive tokenizer emits whole-token + camel/Pascal parts + light stem as synonyms at overlapping positions; the port must set FTS5_TOKEN_COLOCATED on the part/stem forms. This is the trickiest single integration point and needs a focused spike/test before P0 is gated.
- IDF discrepancy: FTS5 bm25() uses log((N-df+0.5)/(df+0.5)) while ADR-006 specifies log(1+(N-df+0.5)/(df+0.5)). Accepted for the FTS5 leg (negligible ranking-order impact, exact k1/b match); the ADR formula applies only to the AST-symbol leg scored over our own postings (P2). Reviewer should confirm this is acceptable rather than forcing a custom ranking function.
- Crate version pins are best-current-stable as of 2026-06; verify against lockfile at implementation time, especially tree-sitter 0.24 + grammar 0.23 ABI compatibility (LANGUAGE_VERSION range). CI must smoke-parse one fixture per grammar.
- serde_yaml is in maintenance mode (0.9 final line); evaluate serde_yaml_ng fork if an advisory lands.
- tree-sitter grammar compile-time override ([profile.release.package."tree-sitter-*"] opt-level=0) is proposed to cut codegen time — measure actual binary-size/compile delta on CI before committing the override.
- Canonical folder names .koden-brain/ (registry) and .koden-memory/ (notes) are ADR-006 PROPOSED names pending Kosta's confirmation.
- MemoryType case-insensitive deserialize: choose between #[serde(alias)] enumerating both cases for all 15 types vs a custom lowercasing Deserialize impl.
- MAX_FILE_BYTES (256KB) is a brain-local addition — fs/search.rs caps by entry count (MAX_SCANNED=50_000) not bytes; confirm 256KB is the intended per-file body cap for FTS indexing.
- FTS5 bm25() fixed weights are applied at QUERY time via bm25(code_fts, 3.0, 2.0, 1.0); confirm path=3.0/symbols=2.0/body=1.0 weighting matches the intended 'path-3x' from ADR-006 (which only specifies path 3x, not symbol weight).
- rusqlite 0.32 + bundled FTS5: confirm FTS5 is enabled by default for the pinned version; if not, add bundled-full feature or -DSQLITE_ENABLE_FTS5, and verify against workspace MSRV.
- Confirm bundled SQLite FTS5 bm25() uses k1=1.2/b=0.75 defaults for the pinned version; if a future SQLite changes them, fall back to manual BM25 over fts5vocab postings (formula from lexical.ts:204-211).
- Canonical folder names still 'proposed' in ADR-006: <root>/.koden-brain/registry.json + <project>/.koden-memory/*.md — confirm before hardcoding the registry path in registry.rs.
- P0 registry seeding: spec assumes reuse of workspace::bootstrap_registry (workspace.rs:118) for launch-dir+home; the interactive add-project + tauri-plugin-dialog folder picker is P1's wizard. Confirm an empty registry on first run is acceptable (search returns []).
- Verify app.path().app_local_data_dir() resolves to the same base the rest of Koden uses for local state (the usage poller uses window_stamp_path — confirm directory convention matches koden/brain/).
- splitCamel: JS uses four chained overlapping regex replaces; the Rust hand-written boundary scanner must be golden-tested against JS output to guarantee identical tokenization across code AND notes.
- FTS5 contentless table (content='') row management by rowid: confirm upsert/delete semantics for the external-content config; alternative is a normal (non-contentless) FTS5 table at a storage cost — decide based on delete-on-reindex (P1) needs.
- Recency half-life (30d) and floor (0.5) are proposed defaults, not specified in ADR-006 — confirm or make configurable.
- BrainState.init returns Result and is .manage()'d from inside .setup() (not the chained .manage at lib.rs:162) to fail-open; confirm app.manage() inside setup is acceptable in the Tauri 2 version used.
- Confirm canonical folder names: ADR-006 proposes <root>/.koden-brain/ for the registry and <project>/.koden-memory/*.md for notes, with global notes at ~/.koden/memory/. These are PROPOSED, not locked — peer reviewer should ratify before P1 code hard-codes them.
- Verify the exact MAX_FILE_BYTES constant name/location in fs/search.rs or fs/file.rs (ADR cites 256KB). Grep showed bounds (MAX_SCANNED=50_000, limits) but I did not see a literal MAX_FILE_BYTES symbol — confirm it exists or define it canonically in brain/fingerprint.rs.
- serde_yaml 0.9.34 is the final release and is in maintenance/soft-deprecated. Decide whether to pin it or switch to an alternative (e.g. yaml-rust2 / a serde_yaml fork). Lossless ordered-map round-trip is the hard requirement; confirm serde_yaml::Mapping preserves insertion order and that dump does not reorder keys.
- P0 dependency assumptions: this section assumes P0 already defines brain/mod.rs BrainState, the SearchIndex trait, the unified index.sqlite3 under app_local_data_dir()/koden/brain/, brain/lexical.rs tokenizer, the worker thread + BrainEvent enum, and the FTS5 docs table with a 'kind' column. If P0's actual shape differs (e.g. separate tables for code vs notes), §1.3's shared-FTS5 assumption must be revised.
- Confirm whether brain memory-note edits should also bump git-committed source (the canonical store is git+MegaSync) — i.e. does the safe writer write into the synced source tree, and is there any commit/staging step, or is that left fully manual to the user? P1 assumes the writer touches the working tree and the user commits.
- The wizard project-candidate detection heuristic (package.json/Cargo.toml/.git at depth 1) needs confirmation for monorepos where projects are nested deeper than depth-1 under the root.
- Linux inotify watch-budget guard reads /proc/sys/fs/inotify/max_user_watches and the 50%-of-remaining threshold is a proposed heuristic — needs a real number/validation on a large monorepo before locking.
- rejected-signature persistence: I put rejected state in the SQLite proposal table (state='rejected') rather than a separate rejected.json like Conductr (proposal-store.ts). Confirm this is acceptable, or whether a portable git-committed rejected list is wanted so rejections travel via MegaSync like the notes do.
- LANGUAGE_VERSION exact value: ADR-006 says pin grammars to 'core LANGUAGE_VERSION range' but tree-sitter 0.24.x exact patch and the matching grammar release ABIs must be verified at implementation time against crates.io (the =0.24.7 / grammar pins here are plausible-but-unverified; confirm ABI 14 vs 15 compatibility window before committing).
- tree-sitter-typescript exposes both language_typescript() and language_tsx() from one crate — confirm the 0.23.x API shape (function vs LANGUAGE const) hasn't changed in the pinned version.
- Conductr graph.ts:117 relative-import resolver helper line number is approximate (the resolveRelativeSpecifier helper sits just after resolveNodeId at 286 / before buildCodeGraph at 340 region) — re-grep exact line when porting.
- Calls edges are symbol->symbol but require the CALLER's enclosing symbol to be known; for top-level/module-scope calls the 'from' is the file node, not a symbol — decide whether module-level calls attach to file: or to a synthetic symbol:<rel>#<module> node. Proposed: attach to file node; flag for reviewer.
- proptest is a net-new dev-dependency not in ADR-006's listed deps; confirm it's acceptable as dev-only (it does not ship in the binary).
- Confidence-tier set algebra for lexical_candidates uses set-difference against ast_confident; confirm whether a node appearing in BOTH a confident and a candidate edge should be deduped to confident-only (proposed: yes, confident wins).
- tsconfig 'extends' chains and project references are not handled in v1 resolver — only the nearest tsconfig's paths/baseUrl. Note as a known v1 limitation; Conductr also did not resolve these.
- Token calibration constants (CHARS_PER_TOKEN_CODE=3.0, PROSE=4.0) and DEFAULT_MAX_TOKENS=2000 are proposed defaults — must be empirically fit per §3.2 calibration procedure before merge; record the measured vs estimated table in the PR.
- AgentSignal.agent is only populated on the 'started' transition (agent_detect.rs:46-47) and defaults to 'claude' when OSC133 carries no agent (agent_detect.rs:213). Synthesis must tolerate a missing/defaulted agent name — confirm the worker captures the started-signal agent before building the gist, or the intent mapping silently falls back.
- SpawnTerminalRequest does not currently expose sessionId/KODEN_SESSION to App.tsx (App.tsx:878-923 reads req.agentId, not the PTY session id). Wiring brain_build_gist needs the leaf's KODEN_SESSION (session.rs:137) threaded through — confirm the PTY leaf->session id is reachable at spawn time or pass project_root + leafId instead and resolve session in Rust.
- Director auto-injection deferred to a P3 follow-on (App.tsx:1392-1396 kodenFunctionsPs1 path left unchanged); confirm that's acceptable for the P3 gate or whether the Director must also receive a gist.
- Whether git_head should salt the fingerprint by default: salting invalidates the cache on every commit even when the working tree is byte-equal (e.g. amend/rebase with no content change). Proposed default = include git_head only when present; flag if the team prefers content-only keying for max cache longevity.
- Idempotent-write skip (§3.4.3) reads agent-<id>.txt before writing; confirm this does not race the recursive brain watcher (the watcher must SKIP ~/.koden per the ownership split, else writing the gist self-triggers a reindex).
- embedderId='none' header is included in the fingerprint now (semantic seam) — confirm the exact header string with the P5 VectorStore/Embedder trait seam so enabling semantic later invalidates cleanly.
- P4-a (BLOCKING prerequisite for Tier 1 as specified): there is no stable persisted pane uuid today — orchestrationStore is intentionally session-scoped and not persisted (orchestrationStore.ts:256-261) and its ids are ephemeral Date.now()+random (orchestrationStore.ts:15); no terminal/tab/pane zustand persist store exists. Must mint a crypto.randomUUID() per pane, persist it (e.g. ~/.koden/panes.json), and thread it to session.rs::spawn as a new KODEN_PANE_UUID env var before sessionKey can be restart-stable. Until then Tier 1 keys degrade to cwd+agent (collides for same-agent-same-dir panes).
- Tier 2 (claude --resume) requires a NEW Claude-session-id capture path that does not exist today: the agent bus carries pty id + status + Task tool_use_ids only (AgentBusBridge.tsx:90-101, subagentBus.ts:25-33), never the Claude session id. Recommended: add a claude_session_id field to the Claude Code status-hook payload on ~/.koden/agent-bus.jsonl and persist it into the resume journal at record_event; degrade to Tier 1 if absent. Confirm the installed Claude Code status hook can emit session_id.
- Reflect model choice (cost vs quality): plan defaults the reflect call to claude-haiku-4-5 ($1/$5 per MTok) for budget reasons with opus-4-8 as opt-in. Confirm Kosta wants Haiku default, or whether reflect should always use Opus on the daemon key.
- Crash-charging policy is conservative-by-design: a reserved-but-unreconciled call is charged at est_cost on boot sweep (over-counts rather than under-counts). Confirm this is the desired bias (it guarantees the spent counter can't leak, at the cost of occasionally over-charging a call that may have failed before billing).
- est_cost token estimate for pre-flight reserve depends on a local token counter for the digest; the claude-api skill says do NOT use tiktoken for Claude. v1 can use a cheap heuristic (chars/4) for the RESERVE estimate (it only needs to be an upper-ish bound) and reconcile with the API's real usage.input_tokens/output_tokens — confirm a rough heuristic for the reserve is acceptable since reconcile corrects it.
- RESUME_TTL_DAYS=7, RESUME_MAX_LINES=2000/compact-to-200, PROPOSAL_TTL_DAYS=30, LEDGER_TTL_DAYS=90 are proposed defaults — confirm or tune.
- fastembed-rs and hnsw_rs exact versions are intentionally left unpinned in P5 (pin at enablement time, not v1) since the feature does not compile into the shipped binary; confirm this deferral is acceptable vs pinning now for the CI semantic-compiles job.
- OD-8 (FTS5 ranking fork) is a genuine unresolved design decision — recommended path B (manual BM25 over FTS5 postings) for ranking fidelity, but it MUST be validated against the <150ms bench before P0 codes the search core. Reviewer must pick A or B.
- agentCommand.ts gist-injection path (~/.koden/agent-<id>.txt + --append-system-prompt) was not found by grep in the terax-workspace tree at the expected location — could not confirm exact file:line for the injection channel. Verify the file path/name before P3.
- Binary-size absolute budget number is left as guidance (+12MB stripped brain-attributable) pending an actual P0 baseline measurement on a release build.
- Resume sessionKey durability (OD-3/R8) depends on whether orchestrationStore actually persists a stable pane uuid and whether the Claude session-id is reachable from the bus for Tier-2 — unverified; needs confirmation before P4.
- Perf gate thresholds (150ms target vs 300ms CI gate vs 25% regression, OD-7) are proposed, not confirmed acceptable by the maintainer.


---

## Appendix: original adversarial review (v1)

*Resolved by Section 0; retained for context.*


**Verdict:** REQUEST CHANGES before P0. The plan is unusually rigorous in its self-flagged open items and its Conductr/LLM-fidelity claims are well-grounded (BM25/IDF in lexical.ts:205, the RRF 5-list hack in hybrid-search.ts:263, context-pack caps/trim at 562-733, and the full P4 Claude API surface all verified correct). But three load-bearing primitives the architecture rests on are factually wrong or unverifiable against the real tree, and they sit under P0/P3/P4 acceptance gates. They must be resolved before any P0 code lands. Citation paths are also systematically wrong (App.tsx is at src/app/App.tsx; hybrid-search.ts and context-pack.ts are not at the cited dirs), which undermines the 'all line refs verified 2026-06-20' assurance.


### Gaps

- SESSION CWD IS NOT STORED (keystone gap). Plan section 2.5 claims 'Session carries the spawn cwd (session.rs:114)'. The verified Session struct (session.rs:43-62) holds only _job, shell_pid, killer, writer, master -- there is NO cwd field. Line 114 is the cwd parameter to spawn(), consumed into build_command and never retained. The entire pty->cwd->project resolution that gist injection (P3 synthesis), agent-signal project tagging (2.4), and resume-project tagging (P4) depend on cannot read cwd from PtyState as the plan describes. A path exists (session.rs:58 exposes shell_pid -> a /proc or ToolHelp live-cwd read), but the stated v1 mechanism ('use spawn cwd from Session') is unimplementable as written. Must be resolved before resolve.rs/registry.rs are committed.
- AgentSignal is Serialize-only, not Deserialize. agent_detect.rs:36 derives only #[derive(Clone, serde::Serialize)] and kind is &'static str. Section 2.4 says the worker app.listen("koden:agent-signal") and 'deserializes AgentSignal' -- that requires a Deserialize impl that does not exist, and kind: &'static str cannot deserialize into an owned field. P0 must add Deserialize + change kind to String/Cow, or define a separate owned DTO. Not flagged anywhere.
- No Linux keyring. Cargo.toml has keyring only under [target.cfg(target_os=macos)] and [target.cfg(target_os=windows)] -- no Linux entry. P4 reflect reads the key via secrets.rs (service 'koden-ai'). On Linux that path is absent, so P4 reflect cannot fetch a key on Linux as specified, yet the crate table (4.1) and 2.2 list keyring as a cross-platform EXISTING dep without flagging this. Linux is in scope (R5/1.2 inotify guard is Linux-specific).
- app.listen() is unproven in this codebase. Grep for '.listen(' across the entire Rust tree returns ZERO hits -- Koden today only emits (app.emit), never listens. The plan's central 'Decision: the Rust worker app.listen()s directly (ADR-006 direct preferred)' (2.4) and the open item asking to 'confirm AppHandle::listen is callable off the main thread' are leaning on an API path with no precedent in the repo. Needs a verified spike (thread-safety of the callback + that it only sends on the mpsc channel) before P0 wires the event spine.
- MAX_FILES indexing cap is mis-sourced. Plan 0.9 cites MAX_FILES=2000 from 'fs/search.rs:164 DEFAULT_LIMIT'. Verified: search.rs:164 DEFAULT_LIMIT is the SEARCH-RESULT cap, not an indexing-corpus cap; MAX_SCANNED=50_000 (search.rs:30) is the walk ceiling. Capping the warm index at 2000 files would silently under-index any project >2000 files while the perf gate fixture is '~2,000 files' -- the cap and the gate are coincidentally equal, hiding the problem. Decide the real per-project index ceiling (likely MAX_SCANNED=50_000) explicitly.
- No MAX_FILE_BYTES symbol exists. The plan's own open item suspects this; confirmed -- search.rs has no MAX_FILE_BYTES (only MAX_SCANNED and DEFAULT_LIMIT). The 256KB per-file body cap is a brain-local invention. Fine to define, but every citation that attributes it to fs/search.rs is wrong.
- SpawnTerminalRequest does not expose a PTY session id to App.tsx. Verified App.tsx (at src/app/App.tsx, not src/App.tsx) line ~909 writes agent-${req.agentId}.txt -- it has req.agentId, not KODEN_SESSION/the pty leaf id. P3's brain_build_gist needs the leaf KODEN_SESSION (session.rs:137) to resolve project/intent. The plan's own open item flags this; it is a real blocker for P3 wiring and needs the leafId/project_root threaded through or resolved Rust-side.

### Inconsistencies

- WorkspaceRegistry self-contradiction. Section 4.6 asserts 'WorkspaceRegistry / ProjectEntry ... Net-new in Koden (verified: no existing workspace-registry struct in src-tauri/src/modules -- grep for registry hits only file-explorer code).' FALSE: workspace.rs:20 defines `pub struct WorkspaceRegistry` with authorize() (26), is_authorized() (33-36), bootstrap_registry() (118). The plan ELSEWHERE correctly relies on this exact struct (2.5 cites WorkspaceRegistry::is_authorized at workspace.rs:33-36; 0.4 cites workspace.rs:20-36 and bootstrap_registry at :118). So 4.6 both invents a net-new WorkspaceRegistry AND the rest of the plan reuses the existing one of the same name -- a naming collision that will shadow or conflict at compile time. Pick a distinct name (the plan also calls it KodenBrainRegistry in 2.2/2.5/4.6 -- unify on that).
- Citation paths are systematically wrong despite the '2026-06-20 verified' claim. App.tsx is at src/app/App.tsx (every App.tsx:NNN cite drops the app/ dir). Conductr hybrid-search.ts is at src/lib/code/ not the bare path cited at 'hybrid-search.ts:263'. Conductr context-pack.ts is at src/lib/brain/ not the cited src/lib/context/. The content at those lines matches (5-list hack at code/hybrid-search.ts ~263; CHARS_PER_TOKEN=4 at brain/context-pack.ts:499; freshness-kept at :733), so the ports are sound -- but the path errors mean an implementer following the cites verbatim will fail to open the files, and the blanket 'all line references verified' assurance is not trustworthy.
- P0 deps list contradicts the master crate table. 0.1 adds rusqlite 0.32 with features ['bundled','blob'] (no fts5 feature) and a comment 'FTS5 ships in bundled... enabled by default >=0.31'. The master table (4.1) lists rusqlite 0.32 with features ['bundled','fts5','blob','functions']. rusqlite DOES expose an explicit `fts5` cargo feature -- omitting it in 0.1 while 0.2/0.5 create FTS5 virtual tables is the exact failure the plan's own 'FTS5 availability' blocker warns about. Reconcile: include the fts5 feature explicitly rather than relying on bundled defaults.
- Two contradictory FTS5/tokenizer integration DECISIONS coexist. 4.2 DECIDES an FTS5 EXTERNAL tokenizer (koden_tok via create_module/fts5_api, with FTS5_TOKEN_COLOCATED for additive forms) and calls it 'the single trickiest integration point'. 0.6 DECIDES the OPPOSITE: a pre-tokenization pass with a trivial ascii pass-through tokenizer, explicitly rejecting an external FTS5 tokenizer as 'brittle unsafe FFI'. Both are presented as locked decisions in the same plan. OD-8 then re-opens the whole question. The reviewer cannot tell which path P0 actually builds. Must collapse to one before P0.
- BM25 weighting decision is internally inconsistent on whether the symbols column exists in P0. 4.2 uses bm25(code_fts, 3.0, 2.0, 1.0) over three columns (path/symbols/body) -- but symbols is AST-derived (P2). 0.5/0.7 use a two-column doc_fts (path/body) with bm25(doc_fts, 3.0, 0.0) / (1.0,1.0). The schema shape and the weighting call differ between the data-model section and the P0 section.
- Recency re-rank constants are asserted as 'ADR-006' in one place and 'proposed defaults, not in ADR-006' in another. 0.8 hardcodes HALF_LIFE_MS=30d, FLOOR=0.5 as if specified; the open items list says '30d half-life and 0.5 floor are proposed defaults, not specified in ADR-006 -- confirm or make configurable.' Decide and state once.

### Technical concerns

- FTS5 contentless table (content='') + delete-on-reindex is under-specified and risky. P1's incremental delta does 'delete FTS5 rows for doc, re-tokenize, insert' every changed file. With content='' (external-content/contentless) FTS5, you MUST manage the rowid lifecycle yourself with the special 'delete' command syntax (INSERT INTO fts(fts, rowid, ...) VALUES('delete', rowid, ...)) supplying the ORIGINAL column values, or the index corrupts silently. The plan never specifies this; a plain DELETE leaves dangling postings. The plan's own open item flags 'confirm upsert/delete semantics for external-content config' -- treat this as a P0 spike, because P1's atomic-delta gate depends on it.
- External FTS5 tokenizer + FTS5_TOKEN_COLOCATED (the 4.2 path) is genuinely hard and the plan rates it correctly. The colocated flag is for synonyms at the SAME position with byte offsets into the INPUT; Conductr's tokenizer emits synthetic stem/part tokens that are NOT substrings of the input, so offsets are fabricated. FTS5 tolerates colocated tokens but snippet()/highlight() and phrase queries behave oddly with synthetic tokens. If you keep the 4.2 external-tokenizer decision, phrase/NEAR queries over stemmed forms are a latent correctness hazard. The 0.6 pre-tokenization path sidesteps this but loses snippet offsets (0.6 acknowledges this). This trade is the single biggest unmade decision in the plan (OD-8).
- FTS5 bm25() IDF differs from the spec and the plan accepts it for the FTS5 leg only -- but the RRF then fuses an FTS5-bm25 leg (IDF log((N-df+.5)/(df+.5))) with a future AST-symbol leg scored under the ADR IDF (log(1+...)). Mixing two IDF formulas across RRF legs is defensible (RRF only uses rank position, not raw score) BUT the plan elsewhere multiplies recency into the FUSED score and sorts by it -- so the absolute bm25 magnitude does leak in via the per-leg ordering feeding RRF. Confirm RRF consumes only rank ordinals (it does in 0.8: rank=idx+1), in which case the IDF discrepancy is truly cosmetic; if any code path uses raw bm25 magnitude, it is not.
- Prompt-cache-stable gist claim is sound IN PRINCIPLE and the plan's controls are the right ones (blake3-fingerprint key, deterministic sorted render, no clock/abs-paths/HashMap, cache-map returns stored bytes verbatim). Two residual risks: (1) the gist is PREPENDED to the per-agent worker prompt (App.tsx flow: gist + worker prompt). Cache prefix stability requires the WHOLE prefix stable -- if the worker prompt or any tool/system content rendered BEFORE the gist ever varies, the gist's own stability is moot. Per the claude-api skill, render order is tools->system->messages and any byte change in the prefix invalidates everything after. The plan must confirm the gist sits at the very FRONT of the appended-system-prompt and that nothing variable precedes it in the actual launch command. (2) git_head salting (3.4.1) optionally varies the key on every commit even when the working tree is byte-identical (amend/rebase) -- the plan flags this; recommend content-only keying by default for max cache longevity.
- SQLite WAL concurrency model is correct but the writer-connection assumption needs a guard. Plan says single writer on the worker thread, readers open SQLITE_OPEN_READONLY from command threads, WAL allows concurrent. True -- but: (a) readers on a separate connection will NOT see the writer's uncommitted txn (fine) and WAL readers can block the checkpointer, not the writer (fine). (b) The real hazard: the plan opens ONE writer Connection inside a Mutex<Connection> in P0 (0.5: 'reads also take the lock in P0') then switches to a ReaderPool in 2.6. If any command thread ever takes the P0 writer Mutex while the worker holds it across a multi-statement txn, commands block the UI exactly as the no-block guarantee forbids. The migration from 0.5's single-mutex model to 2.6's reader-pool model is a real refactor, not a detail -- and busy_timeout/PRAGMA is never set, so a reader hitting a checkpointer can SQLITE_BUSY. Set busy_timeout and specify the reader-pool from P0.
- Binary size: the plan's +12MB stripped brain-attributable budget is plausible but optimistic on the LTO-fat/opt-level=s/panic=abort profile (verified Cargo.toml:96-101). bundled SQLite amalgamation under codegen-units=1 + lto=fat is ~1.5-2.5MB and a large compile-time hit; 3 tree-sitter C grammars under the same profile commonly land 2-6MB combined once generated parse tables are LTO'd in. The proposed [profile.release.package.tree-sitter-*] opt-level=0 override helps COMPILE time but can INCREASE size (less optimization). The plan flags 'measure before committing the override' -- good; just don't assume the override is free on size. panic=abort is already set, so unwinding tables aren't a factor.
- tree-sitter incremental relink correctness: the O(neighbors) inbound-rebind argument (2.7) is the right design and the property test ast_incremental_equals_full_rebuild is the correct gate. One real hole: the plan deletes edges by src_file and keeps inbound edges 'because we only delete by src_file' -- but a RENAMED symbol changes the node id (symbol:rel#name), and inbound edges point at the OLD id. The plan handles this via rev[old_id] lookup, but rev is an in-MEMORY map (graph.rs AstGraph) that is rebuilt from ast_edges on load. After a restart mid-rename, the in-memory rev map is reconstructed from persisted edges that may already be half-updated -- the property test must include a SIMULATED RESTART between mutations (load graph from SQLite, then continue incremental), not just in-process mutation sequences, or it won't catch persisted-vs-memory divergence. Add restart to arb_project_mutation_seq.
- tree-sitter ABI/version pinning is contradictory across sections (tree-sitter 0.22 + grammars 0.21 in section 2.2; 0.24.7 + 0.23.x in section 2.1/4.1) and the LANGUAGE_VERSION constant (14 vs 15) is guessed. This is correctly flagged as an open item, but the two cited version sets are far enough apart (0.22 ABI 14 vs 0.24 ABI 15) that the .scm queries and the language_typescript()/language_tsx() API shape may differ between them. The CI smoke-parse gate is the right mitigation; just pin ONE set before P2 and delete the other from the plan to avoid an implementer wiring 0.22.
- Reflect budget crash-safety (P4) is well-designed (reserve-before-call, sweep-orphaned-on-boot charges est_cost). One concern: est_cost is computed from a chars/4 heuristic for the digest, but the conservative bound assumes max_tokens output. If the model returns MORE input tokens than estimated (the heuristic under-counts code-dense notes, which the plan ACK's by using chars/4 for prose and 3 for code), the reserve can be lower than actual and a single call could exceed ceiling by the estimation error. Reconcile corrects the LEDGER but the pre-flight gate already let the call through. For a hard ceiling, the reserve must be a strict UPPER bound (use chars/3 or lower divisor for the reserve regardless of content class). The plan's reserve heuristic is an approximation, not an upper bound -- state that the ceiling is best-effort, not hard.
- P4-a (persisted pane uuid) and Tier-2 Claude-session-id capture are correctly identified as BLOCKING prerequisites with no current implementation (orchestrationStore is session-scoped/ephemeral, verified at orchestrationStore.ts ~256 and uid() at ~16; no zustand persist store for panes). Good catch by the authors. The concern is sequencing: P4-a requires a NEW persisted layout store + a NEW KODEN_PANE_UUID env var threaded into session.rs::spawn -- that is frontend + Rust + a new env var, i.e. a mini-feature, not 'polish'. It should be its own phase gate BEFORE P4, and the plan should not present Tier-1 resume as P4-internal.

## Koden Brain — Tightened Pre-Merge Reviewer Checklist

### BLOCKERS — resolve before any P0 code lands
- [ ] **Session cwd**: confirm how pty→cwd resolves. `Session` (session.rs:43-62) stores **no cwd**, only `shell_pid`. Pick: (a) read live cwd via `shell_pid` (/proc on Linux, ToolHelp/NtQuery on Win), or (b) add a `cwd` field to `Session` at spawn. Update §2.5 — the cited "Session carries spawn cwd (session.rs:114)" is false.
- [ ] **AgentSignal deserialization**: `agent_detect.rs:36` is `Serialize`-only and `kind: &'static str`. Add `Deserialize` + change `kind` to `String`/`Cow`, **or** define an owned listen-DTO. (§2.4)
- [ ] **app.listen() spike**: zero `.listen(` call sites exist in the repo today. Prove `AppHandle::listen` is callable off the worker thread and the callback only `send`s on the mpsc channel, before wiring the event spine. (§2.4, OD-4)
- [ ] **WorkspaceRegistry name collision**: `workspace.rs:20` ALREADY defines `WorkspaceRegistry`. §4.6 wrongly says it's net-new. Rename the brain's registry to `KodenBrainRegistry` everywhere and stop redefining `WorkspaceRegistry`.
- [ ] **FTS5 tokenizer decision (OD-8)**: §4.2 (external `koden_tok` + `FTS5_TOKEN_COLOCATED`) and §0.6 (pre-tokenization + ascii pass-through) are contradictory locked decisions. Collapse to ONE before P0.
- [ ] **FTS5 build proof**: add `features=["bundled","fts5"]` explicitly (don't rely on bundled default). CI `CREATE VIRTUAL TABLE ... USING fts5` smoke test must be green before P0 locks.
- [ ] **Contentless FTS5 delete semantics**: specify the `INSERT INTO fts(fts,rowid,...) VALUES('delete',...)` lifecycle (with original column values) for P1's delete-on-reindex, or switch to a normal (non-contentless) FTS5 table. A plain DELETE corrupts the index.

### Primitive reuse — re-verify line refs (paths are wrong in the plan)
- [ ] `usage/poll.rs:384` spawn_poller ✓; `:409` app.state ✓; `:355-364` atomic temp+rename ✓; `:418-421` in_flight guard ✓.
- [ ] `fs/watch.rs:14/15` DEBOUNCE/MAX_WINDOW ✓; `:186` RecursiveMode::NonRecursive ✓.
- [ ] `fs/search.rs:30` MAX_SCANNED=50_000 ✓; `:164` DEFAULT_LIMIT=2000 is the **result** cap, NOT an index cap — fix §0.9.
- [ ] **No MAX_FILE_BYTES** exists in fs/search.rs — define it brain-local, stop citing fs/search.rs for it.
- [ ] `pty/agent_detect.rs:37-52` AgentSignal/Transition ✓ (but Serialize-only — see blocker).
- [ ] `pty/session.rs:137` KODEN_SESSION ✓; `:43` Session struct ✓ (no cwd — see blocker).
- [ ] App.tsx is at **`src/app/App.tsx`** (every `App.tsx:NNN` cite is missing `app/`). Injection write at ~909 uses `req.agentId`, not a session id.
- [ ] Conductr cites: `hybrid-search.ts` is at `src/lib/code/`; `context-pack.ts` is at `src/lib/brain/`. Content matches (~263 5-list hack; :499 CHARS_PER_TOKEN; :733 freshness-kept) but paths in the plan are wrong.
- [ ] `lib.rs:159` spawn_poller, `:162-177` manage, `:178` generate_handler ✓.

### Platform / dependency
- [ ] **Linux keyring**: Cargo.toml has macOS+Windows keyring only. Add a Linux target or scope P4 reflect as non-Linux, and stop listing keyring as cross-platform EXISTING.
- [ ] **tree-sitter pins**: §2.2 says 0.22/0.21, §2.1/4.1 say 0.24.7/0.23.x. Pick ONE, delete the other, verify ABI (14 vs 15) and `language_typescript()`/`language_tsx()` API shape against crates.io. CI smoke-parse per grammar.
- [ ] Confirm `panic="abort"` (Cargo.toml:100) is compatible with every `.expect()`-on-poisoned-lock path (a poisoned lock under panic=abort aborts the process, not just the thread — acceptable for fail-open? state it).
- [ ] Binary-size: measure P0 baseline + post-P2 delta on the real LTO-fat/opt-level=s profile before locking the +12MB budget; the tree-sitter `opt-level=0` override may grow size.

### Search / ranking
- [ ] Confirm RRF consumes only rank ordinals (0.8 uses rank=idx+1) so the FTS5-vs-ADR IDF discrepancy is cosmetic; reject any path that fuses raw bm25 magnitude.
- [ ] Unify the bm25 column set: §4.2 uses 3 columns incl. AST `symbols` (P2-only); §0.5/0.7 use 2 columns. P0 must be 2-column.
- [ ] State recency HALF_LIFE/FLOOR once (proposed, not ADR) and make configurable.
- [ ] golden-test splitCamel against the JS output (overlapping-regex semantics) — applied identically to code AND notes.

### SQLite concurrency
- [ ] Reconcile §0.5 (single `Mutex<Connection>`, reads take the lock) with §2.6 (ReaderPool + RO conns). Build the reader-pool from P0 or commands WILL block the worker.
- [ ] Set `PRAGMA busy_timeout` (and confirm WAL checkpoint policy) — RO readers can hit SQLITE_BUSY against the checkpointer; the plan never sets it.

### Gist cache-stability (P3)
- [ ] Confirm the gist is at the **front** of the appended system prompt and NOTHING variable renders before it (tools→system→messages prefix rule). A stable gist behind a varying worker prompt still busts cache.
- [ ] Default to **content-only** fingerprint (no git_head salt) for max cache longevity; flag amend/rebase busts if salting kept.
- [ ] Confirm the brain watcher SKIPS `~/.koden` so writing `agent-<id>.txt` does not self-trigger a reindex (3.4.3 idempotent-write race).

### tree-sitter incremental (P2)
- [ ] `ast_incremental_equals_full_rebuild` property test MUST include a **simulated restart** (reload graph from SQLite mid-mutation) — the rev-adjacency rebind correctness lives at the persisted/in-memory boundary, not just in-process.

### P4 budget / resume
- [ ] State the ceiling is **best-effort, not hard**: the chars/4 reserve heuristic is an approximation, not a strict upper bound. Use a conservative divisor for the reserve if a hard ceiling is required.
- [ ] Make **P4-a (persisted pane uuid + KODEN_PANE_UUID env var)** its own phase gate BEFORE Tier-1 resume — it's a frontend+Rust+env mini-feature, not internal polish. orchestrationStore is verified ephemeral (no persist store exists).
- [ ] Confirm the installed Claude Code status hook can emit `session_id` on `~/.koden/agent-bus.jsonl` before promising Tier-2 `--resume`.

### Claude API (P4) — VERIFIED CORRECT against claude-api skill, keep as-is
- [x] Opus 4.8 $5/$25, Haiku 4.5 $1/$5 per MTok ✓
- [x] temperature/top_p → 400 on 4.8 ✓; thinking budget_tokens → 400, use {type:"adaptive"} ✓
- [x] output_config:{format:{type:"json_schema",schema}} (not output_format, not prefill) ✓
- [x] anthropic-version: 2023-06-01, x-api-key ✓; count_tokens endpoint exists, no tiktoken ✓
- [ ] Note: adaptive thinking is OFF by default on 4.8 — the plan's explicit `{type:"adaptive"}` is correct; keep it explicit.