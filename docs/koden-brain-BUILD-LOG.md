# Koden Brain — Build Log

Running, timestamped audit trail of the autonomous Koden Brain build (per
`.memory/plans/koden-brain-BUILD-PROMPT.md`). Append-only; newest at bottom.

Branch: `feat/koden-brain` (off `main` @ `f00a360`). Sibling `feat/koden-cli`
workstream is independent and merges later.

Legend: ✅ done+verified · ⏳ in progress · ⏭ deferred (with reason) · ❌ failed (with fix)

---

## 2026-06-20 — Session 1 (orchestrator bootstrap)

### Recon & grounding (§3, §4)
- Read in full: `ADR-006` (architecture of record), `koden-brain-CONCEPT.md`
  (flows + 30 `[DP-n]` + secrets §7.1 + acceptance §12), `koden-brain-EXECUTION_PLAN.md`
  §0 (authoritative corrections) + §1–§2.8 (module tree, worker lifecycle, event
  spine, resolution, concurrency, fail-open).
- Toolchain verified on this Windows dev box: `cargo 1.94.1`, `rustc 1.94.1`,
  `node v24.13.0`, `pnpm 11.5.0`.
- `src-tauri/src/modules/brain/` does **not** exist — greenfield.
- Base selection: `main` = `f00a360` ("Koden — terminal-first…"); `feat/koden-cli`
  = main + the CLI sibling (touches lib.rs/pty/mod.rs/Cargo.lock + adds cli/, bin/,
  pty/headless.rs, docs/koden-cli-*). Branched `feat/koden-brain` off **main** per
  mandate §8. On main, `session.rs`/`agent_detect.rs`/`modules/mod.rs` are
  byte-identical to what §0 cited, so the verified blocker analysis holds.

### B1–B7 blocker verification (against real source on this branch)
- **B1** CONFIRMED — `Session` (`pty/session.rs:43-62`) has no `cwd`; `spawn()` takes
  `cwd` (:114) and moves it into `build_command` (:132), then drops it. → add `pub cwd`.
- **B2** CONFIRMED + RESOLVED-BY-DESIGN — `AgentSignal` (`pty/agent_detect.rs:36-41`)
  is `Serialize`-only, `kind: &'static str`, and on `main` `agent_detect` is a
  **private** module. Decision: the brain does **not** touch `agent_detect.rs`. It
  `app.listen("koden:agent-signal", …)` and deserializes the already-JSON payload
  into its **own** `AgentSignalPayload { id, kind: String, agent: Option<String> }`
  in `brain/events.rs`. Zero wire change, zero pty-surface widening, fully decoupled
  — strictly cleaner than §0/B2's two suggested options (add Deserialize / mpsc).
- **B3** CONFIRMED — `PtyState.sessions` (`pty/mod.rs:23`) is private. → add a `pub`
  cwd accessor (consumes B1). Used by P3 gist resolution; added now as pre-work.
- **B4** CONFIRMED — `SpawnTerminalRequest` (`DirectorView.tsx`) carries no `leafId`.
  ⏭ Deferred to P3 (gist project resolution is the only consumer); tracked.
- **B5** CONFIRMED — bus filename split (`agent.rs` writes `director-bus.jsonl`;
  `App.tsx` reads `agent-bus.jsonl`). ⏭ Deferred to P4 (Tier-2 resume is the only
  consumer); pre-existing known issue, tracked.
- **B6** CONFIRMED — `WorkspaceRegistry` exists + managed (`lib.rs`). Brain registry
  named distinctly `KodenBrainRegistry`. Cleared via naming on module add.
- **B7** CONFIRMED — `lib.rs` `use modules::{…}` list + `modules/mod.rs` both need
  `brain`. Cleared on module add (+ handler registration).

### Decisions (decide-and-document, §1)
- D1: B2 via brain-owned payload struct (above).
- D2: P0 Cargo additions limited to `rusqlite` (bundled+FTS5) + `blake3`. `notify`,
  `ignore`, `reqwest/rustls`, `keyring`, `serde` already in-tree. `tree-sitter` (P2),
  `serde_yaml` (P1), `tauri-plugin-dialog` (P1) added at their phases.
- D3: Worker mirrors `usage::poll::spawn_poller` (`poll.rs:384`) exactly — named
  `std::thread`, fail-open, spawned from `.setup()` after the usage poller.
- D4: `rusqlite` pinned to **0.31** (not the resolver's latest 0.40.1). 0.40 →
  `libsqlite3-sys 0.38.1` whose build script uses the unstable `cfg_select!`
  feature, which fails on the pinned `rustc 1.94.1`. 0.31's bundled SQLite ships
  FTS5 (proven by the `fts5_is_available` test) and predates it. Revisit on a
  rustc bump. ❌→✅ (caught by a real `cargo check`, not assumed.)
- D5 ([DP-9]/[DP-2]): BM25 = FTS5 built-in `bm25()` (hardcoded k1=1.2/b=0.75 ==
  Conductr) with first-class per-column weights (path 3× / symbols 1.5× / content)
  — replaces Conductr's path-string-repetition hack. Fusion = weighted RRF with
  first-class per-leg weights — replaces Conductr's list-duplication hack. Leg
  weights: identity 1.5 > content 1.0 so a **filename match outranks a body-only
  mention** (CONCEPT [DP-2]); I initially mis-imported Conductr's content-heavy NL
  bridge, the `path_match_outranks_body_only` test caught it. Weights are
  provisional pending benchmark calibration.
- D6 (secrets, hard gate): file denylist + regex-free content redaction (provider
  prefixes + Shannon-entropy) run before any tokenize/store. Conservative P0
  baseline; the Auditor hardens it against the planted-secrets fixture.

### Milestone M0 — P0 warm-lexical-brain backend ✅ (green, committed)
Built (orchestrator-authored; the interdependent foundation per §13.5):
`brain/{mod,events,worker,registry,rank,tokenize,secrets,commands}.rs`,
`brain/store/{mod,schema,migrate,sqlite}.rs`, `brain/freshness/{mod,hash,walk}.rs`;
wired into `lib.rs` (worker spawn + `.manage` + 3 commands) + `modules/mod.rs`.
Blockers cleared: B1 (Session.cwd), B2 (brain-owned `app.listen` payload),
B3 (PtyState::session_cwd), B6 (`KodenBrainRegistry`), B7 (module wiring).

Evidence (this machine, Windows, `feat/koden-brain`):
- `cargo check --all-targets` → clean.
- `cargo clippy --all-targets --locked -- -D warnings` → clean (fixed a needless
  `mut` + ported the pre-existing Windows-only `window` unused-var cfg_attr that
  `main` lacked but `koden-cli` had).
- `cargo test --locked` → **217 passed, 1 failed**. The single failure is the
  documented pre-existing `authorize_spawn_cwd_blocks_symlink_escape`
  (`workspace.rs:829`, Windows symlink-creation privilege — untouched by this
  workstream, a known non-regression).
- 25 brain unit tests pass: tokenizer port (camel/stem/stoplist/query-doc
  symmetry), weighted RRF, secrets (denylist/prefix/entropy/no-false-positive),
  FTS5 availability + index/search/path-priority/reindex/no-op-skip, freshness
  aggregate, registry idempotency + longest-prefix resolve, migration idempotency.

⏭ Still in P0 (next, via fan-out): minimal Brain pane (frontend); the §6.5
sandbox + fixture repos + adversarial Auditor proof of the secrets gate from a
real run; benchmark harness (labeled ground-truth + negative control). Watcher
freshness is P1.

### Adversarial verification round (mandatory, §2/§13.6) ✅
12-agent workflow (8 reviewers + 3 designers + synthesis) over the committed P0
code; ~1.27M tokens. Surfaced 32 findings (3 CRITICAL, 3 HIGH). Triaged and
**must-fix items fixed this session**, re-verified green:

- **CRITICAL — secrets gate leaks** (the hard gate): the redactor's 20-char floor,
  `has_letter && has_digit` AND-gate, and separator-splitting let short/all-letter/
  punctuation-split secrets reach the index. Rewrote `secrets.rs` with three
  detectors (prefix scan · secret-named `key=value` whole-value redaction ·
  FP-aware high-entropy) + expanded denylist. Added the **planted-secrets real-run
  proof** (`planted_secrets_never_reach_the_index`: redact→index_file→search shows
  no secret retrievable, while a git-SHA control + surrounding identifiers stay).
  Documented residual gap honestly (bare in-code pure-hex / `/`-split secret not in
  a secret-named assignment).
- **HIGH — Windows pty→project resolution always failed**: `\\?\` verbatim prefix
  wasn't stripped from registry roots. Routed `registry` through `fs::to_canon`
  (canonicalize both root and cwd).
- **MEDIUM — reconcile never deleted**: removed/moved files matched forever. Added
  `SqliteIndex::{existing_paths,remove_file}` + prune pass in `index_project`
  (+ `remove_file_prunes_index` test).
- **MEDIUM — walk didn't prune**: added `.filter_entry` so non-gitignored
  `node_modules` is never descended.
- **MEDIUM — FTS5 tokenizer**: `unicode61`→`ascii` (passthrough; matches §0.6,
  avoids index/query desync). UNIQUE `files_fts_rowid` index.
- **MEDIUM — readonly conns lacked `busy_timeout`**: added 5s (no silent empty on
  transient lock).
- **MEDIUM — seed used `std::env::current_dir()`**: could index an install dir /
  fs root on a packaged build. Now seeds from the authorized launch dir, falling
  back to cwd only if it has a project marker and is sane (not fs-root/home).
- **LOW — fail-open + dead code**: tick-thread spawn no longer `.expect`s (logs +
  continues); wired `brain_rescan` command (was dead `tx` plumbing); softened two
  over-claiming doc comments (IDF, tie-break) and the stoplist count.

Deferred (lower-risk / later phase, logged): non-ASCII identifier tokenization
(bundle with the ascii decision), per-file stat/tx micro-opt (P1 watcher), Hit
shape enrichment (later phase), Rescan event-loop starvation (superseded by the P1
incremental watcher).

Re-green: `clippy --all-targets --locked -D warnings` clean · `cargo test --locked`
**220 passed, 1 pre-existing** (symlink) · 28 brain tests.

### P0 §6.5 offline sandbox (real-run proof) ✅
Extracted the indexing pipeline into an `AppHandle`-free seam
`brain::worker::index_dir(index, project_id, root) -> IndexStats` (worker calls it;
tests drive it) + `SqliteIndex::project_fingerprint`. Added `tests/brain_sandbox.rs`
— 4 integration tests over materialized fixture repos in a scratch TempDir,
exercising the **real** walk→sniff→blake3→redact→index→reconcile pipeline:
- real source indexed; generated/`node_modules`/`dist`/binary(NUL)/oversized(>1MB)
  all excluded (verified via per-file sentinel tokens);
- **secrets gate proven from a real walk**: a denylisted `.env` is never read and an
  inline `sk-…` key is redacted — none retrievable via search — while normal code
  stays searchable;
- **incremental == full rebuild**: after modify+add+delete on disk, a reconciled
  index has the same file count, byte-identical `project_fingerprint`, and same
  results as a fresh full rebuild (prune verified);
- fingerprint determinism across rebuilds (P3 cache-stability proxy).
Green: clippy clean · `cargo test --test brain_sandbox` 4/4.

⏭ P0 remaining: Brain pane (frontend) · benchmark harness (labeled ground-truth +
negative control) · larger fixture catalog + the `pnpm tauri dev` real-app run +
real-kill crash sim + real-key smoke (cross-phase DoD evidence).

### P0 relevance benchmark ✅ (commit 4e72984)
`tests/brain_bench.rs` — hermetic, offline, over a realistic mixed TS/Rust corpus
via the real `index_dir` pipeline. Three labeled bands (CONCEPT §12.2 / anti-gaming
§13.12): positives (recall@5, hard floor 0.80), negative control (must be empty,
hard gate), semantic-intent (P0-lexical expected to miss; reported not asserted).
Measured from a real run (`docs/koden-brain-BENCH.md`): positives **9/9=1.00**,
negative-control leaks **0/3**, semantic-intent **0/2** (the honest lexical gap
that P5 closes). Deliberately not a flat vanity 1.0 — the discriminating signal is
the negative control + the semantic band. _(P0-era figures; V2.2 grew the corpus to
16 files / 12 positives with confusers — see the V2.2 entry + current BENCH.md.)_

### P0 Brain pane (frontend) ✅
Recon mapped the conventions (invoke via `@tauri-apps/api/core`, `ExplorerSearch`
as the search-pane template, singleton-view registration, no i18n, `tsc`+biome
gates). Built `src/modules/brain/{lib/bindings.ts, BrainPane.tsx, index.ts}` —
typed command wrappers + a minimal pane (index-status line with a Rescan button,
optional project filter, debounced search with the alive-flag cancel idiom,
results list). Wired as a singleton view by extending `OrchestrationView` with
`"brain"` (smallest change): `useTabs` (type + title), `WorkspaceSurface` (render),
`TabBar` (search icon — not a brain icon, per the no-AI-iconography rule),
command palette ("Open Brain"), `App.tsx` (`openBrain` callback + deps), and
`serialize.ts` (persists across restart). Verified: `tsc --noEmit` clean, biome
lint clean (5 files), `serialize.test.ts` 7/7.
⏭ Runtime click-through in a live `pnpm tauri dev` is the remaining evidence
(needs a GUI session) — folds into the cross-phase real-app-run DoD item.

### P0 status: functionally complete + statically/offline-verified
Warm lexical brain (search/status/list/rescan) + secrets hard-gate + AST-less
graph deferred to P2 + the offline sandbox proof + the relevance benchmark + the
Brain pane. Remaining P0-adjacent evidence is the live `pnpm tauri dev` run (with
the fake-claude→agent-detect replay) + real-kill crash sim + real-key smoke —
all cross-phase, GUI/live-session items. **Next: P1** (recursive `notify` watcher
+ incremental freshness, native memory store + seed import, `MemoryProposal`
queue + doctor, 3-step setup wizard).

---

## 2026-06-20 — Session 2 (P1 start)

### P1 freshness: recursive watcher + incremental reindex ✅
`brain/freshness/watch.rs` — brain-owned RECURSIVE `notify` watcher over project
roots (the existing `fs/watch.rs` is NonRecursive). Debounced/coalesced with the
same 150ms/1000ms constants; `group_by_project` resolves each changed path to its
project by longest-prefix over canonical roots (unit-tested), feeding
`BrainEvent::Fs` to the worker. No new dep (`notify` already in-tree).
Worker: extracted `index_one_file` (shared by full + incremental); added
`index_changed` (reindex changed text files — hash-skip makes no-ops cheap — and
prune vanished ones); armed the watcher after warm-population and re-arm on
Rescan; `rel_path` now routes both sides through `fs::to_canon` so the full walk
and the watcher (native/`\\?\` absolute paths) produce the same rel.
**P1 gate proven** (`incremental_reindex_touches_only_changed_paths`): an
out-of-band modify+delete+add reindexes only the changed paths (modified
re-indexed, deleted pruned, added inserted), untouched files remain. Live-FS
watcher timing verification folds into the cross-phase real-app-run evidence
(a real-FS-event test would be timing-flaky — avoided per §13.21; the resolution
+ reindex LOGIC is tested deterministically).
Green: clippy `-D warnings` clean · `cargo test` 221 passed / 1 pre-existing ·
brain lib 29 · brain_sandbox 5.

### P1 native memory store ✅
Added `serde_yaml 0.9` (ADR-006 dep). `brain/memory/mod.rs`: `MemoryNote` model +
frontmatter parser (`---` YAML + body, BOM-tolerant, **null-stripping** to None for
Zod-parity per §0.3 [DP-10]), title from frontmatter→`# heading`→filename,
`scan_project_memory` over `<root>/.koden-memory/*.md`. Schema v2: a structured
`notes` table (id/type/status/title/scope/provenance/anchors-JSON/hash) +
`upsert_note`/`note_count`/`list_notes_readonly`. Notes are scanned during
warm-population and re-synced when a `.koden-memory/` file changes (incremental).
The note FILES stay lexically searchable via the code walk (one query path);
the table is the typed layer for cards/doctor/proposals. New `brain_notes` command
(+ registered). Tests: frontmatter parse incl. null-strip + malformed-degrades +
heading-title (4 unit) + `memory_notes_parsed_stored_and_searchable` (integration:
scanned → listed → searchable).
Green: clippy `-D warnings` clean · `cargo test` 225 passed / 1 pre-existing ·
brain lib 33 · brain_sandbox 6.

### P1 proposal queue + deterministic doctor ✅
Schema v3: `proposals` + `reject_signatures` tables (brain-owned, local-only,
rebuildable; never auto-applied — propose-only). `memory/proposal.rs`:
`ProposalAction` (create/update/supersede + archive apply-op, §0.3), the **two
distinct signature schemes** (plain-join `proposal_signature` PK vs djb2
`reject_signature`, §0.3). `memory/doctor.rs`: pure `check()` (deterministic,
clock-injected `now_date` per §13.21) with a code-grounded subset —
`missing_type`, `broken_supersession`, `stale_revalidate`, `broken_anchor`
(path-shaped only; AST anchors are P2) — plus `run_doctor` that queues findings as
proposals, skipping persisted reject-signatures. Single-writer respected: new
`Doctor`/`ResolveProposal` events; commands (`brain_doctor`/`brain_proposals`/
`brain_resolve_proposal`) enqueue onto the worker; structural doctor runs once
after warm-population to seed the inbox. **P1 gate proven**
(`doctor_queues_proposals_and_reject_sticks`): a finding → a proposal → reject
persists a signature → the proposal does not reappear, while un-rejected ones stay.
Green: clippy clean · `cargo test` 232 passed / 1 pre-existing · brain lib 40 ·
brain_sandbox 7.
⏭ The full 18-check port + `TYPED_CHECK_MAP` (§0.3) is tracked follow-up (subset
shipped); applying a proposal is human/agent work (Librarian never edits user files).

### P1 Brain pane: memory cards + review inbox ✅
Backend: `MemoryProposal` now carries its `project` (populated on read) so the UI
can resolve. Frontend: a Search/Memory mode toggle in the pane. Memory mode shows
the **review inbox** (pending proposals with action badge + detail + Approve/Reject,
plus a "Run doctor" button passing today's ISO date) and **note cards**
(type/status/path), over the `brain_notes`/`brain_proposals`/`brain_doctor`/
`brain_resolve_proposal` commands. Optimistic removal on resolve + a short
settle-then-refetch (the worker applies async). Verified: `tsc --noEmit` clean,
biome clean, Rust clippy clean, brain_sandbox 7/7. Runtime click-through pending a
live `pnpm tauri dev` (cross-phase evidence).

### `brain_add_project` + multi-project (commit 59ac8e7) ✅
`brain_add_project(path)` validates a dir, registers it, enqueues a reconcile-all
(indexes + re-arms watcher). Pane gains a "+ Add" path-input affordance. Folder
PICKER dialog deferred (needs tauri-plugin-dialog capability + live verification).

### P1 adversarial-verification round + hardening ✅
8-reviewer + synthesis fan-out (~946k tokens) over the committed P1 surfaces; 29
findings (2 CRITICAL, 5 HIGH). Verified each against code; fixed the must-fix set:

- **CRITICAL (one root cause, two symptoms) — Windows watcher dead + split DB keys**:
  `registry::normalize` swapped backslashes but did NOT strip the `\\?\` verbatim
  prefix, so the stored root was `//?/C:/…` while `to_canon` (used by the watcher's
  `group_by_project` and `rel_path`) yields `C:/…` → longest-prefix never matched →
  every FS event silently dropped (incremental watcher dead on Windows) AND full
  walk vs watcher produced divergent rel keys. **HONESTY NOTE:** my P0 hardening
  commit (ee2f4ae) *claimed* this registry fix but it never actually landed — I'd
  fixed `rel_path` but left `normalize` untouched. Now genuinely fixed: `normalize`
  routes through `fs::to_canon`. Added a Windows regression test (the gap existed
  because every prior test used Unix `/work/repo` literals with no verbatim-prefix
  control).
- **HIGH — one bad YAML field discarded the whole frontmatter**: replaced the
  all-or-nothing `from_str::<Frontmatter>` with a tolerant per-field projection over
  `serde_yaml::Value` (wrong-typed field → None for that field only; gray-matter
  parity). + closing-fence tolerates trailing whitespace.
- **HIGH — notes never pruned**: `scan_project_memory` now collects a seen-set and
  deletes vanished notes (`existing_note_ids`/`remove_note`), mirroring the files
  reconcile.
- **HIGH — zombie proposals**: `remove_note` deletes the note's dependent PENDING
  proposals in the same tx, so the doctor stops regenerating findings for a gone note.
- **HIGH — `index_changed` ignored directory events**: a vanished path now also
  prunes any indexed files under its prefix (covers dir delete/rename).
- **MEDIUM (safety) — notes bypassed the secrets gate**: the note title is now
  `secrets::redact`-ed before `upsert_note` (the notes table is a form of indexing).
- **MEDIUM — `broken_anchor` false positives**: symbol/line-suffixed anchors
  (`a/b.rs#sym`, `a/b.rs:42`, `./a/b.rs`, `mod::fn`) are normalized/skipped.
- **MEDIUM/LOW (frontend)**: doctor/resolve now poll-refetch (not a single 500ms
  shot); `today()` uses local date; unique note React keys; add-project errors
  surfaced. Watcher re-arm drops the old watcher first (no double-watch window).

Green: clippy `-D warnings` clean · `cargo test` 237 passed / 1 pre-existing ·
brain lib 45 · brain_sandbox 9 · tsc + biome clean.

⏭ P1 deferred (logged, not blocking P2): external seed importer; folder-picker
dialog; full 18-check doctor port; Rescan coalescing + honor-single-project-field
(perf; current behavior is a correct superset); applied-proposal audit retention
vs recurrence; reject_signature GC. **P1 is functionally complete + verified.**

---

## 2026-06-21 — Session 3 (P2 start: tree-sitter AST)

### P2.1 — grammars de-risked + symbols populated ✅
Added `tree-sitter 0.26.9` + `tree-sitter-rust 0.24.2` + `tree-sitter-typescript
0.23.2` (resolver-picked; modern `LANGUAGE: LanguageFn` API; `StreamingIterator`
re-exported by `tree_sitter`). `brain/ast/mod.rs`: extension→`Lang` mapping
(Rust; TS/TSX, with JS/JSX riding the TS/TSX superset), `parse`, and `extract_defs`
(definition-name capture queries per language — fail-open `[]` on any grammar/query
error). **Top ADR risk (grammar/ABI drift) retired** by smoke tests that prove the
grammars load + parse AND the queries match the real node kinds for both languages.
Wired into `index_file`: definition names now feed the FTS `symbols` column (left
empty in P0), weighted above content in the identity leg — derived from the
(redacted) content, computed only on changed files (after the hash-skip), zero
caller churn. Tests: 4 AST (ext map, grammar smoke, Rust defs, TS defs) +
`symbols_column_populated_from_ast`.
Green: clippy `-D warnings` clean · `cargo test` 242 passed / 1 pre-existing ·
brain lib 50 · brain_sandbox 9.

### P2.2 — AST graph: nodes, import edges, tiered impact + the gate ✅
Schema v4: `code_nodes` (defs), `code_imports` (raw per-file specs), `code_edges`
(resolved file→file). `ast::analyze` parses once → nodes (name/kind/line) + import
specs. `index_file` stores nodes + import specs per file (in its tx); `remove_file`
cleans them; **edges are rebuilt as a pure function of (imports, file set)**
(`rebuild_edges`, called once per project pass) — so incremental relink and a full
rebuild provably converge. Relative import resolution (extension + `/index`
fallback); tsconfig-paths/pkg-exports/Cargo-member resolution + Rust `use` edges
are a documented later refinement (Rust still gets nodes + lexical candidates).
`code_impact` = AST reverse-import BFS closure (tiered above the lexical
over-approximation, CONCEPT §4.1b); `get_symbol` = definition locations. Commands
`brain_get_symbol`/`brain_code_impact` registered.
**P2 gate proven** (`incremental_relink_equals_full_rebuild`): after a mutate +
single-file relink, nodes AND edges are byte-identical to a full rebuild over the
same final state. Plus `code_impact_reverse_import_closure` (a→b→c chain →
impact(alpha) dependents = {b, c}). The `symbols` FTS column is now AST-fed.
Green: clippy `-D warnings` clean · `cargo test` 243 passed / 1 pre-existing ·
brain lib 51 · brain_sandbox 11.

### P2 adversarial-verification round + hardening ✅
8-reviewer + synthesis fan-out (waved 4-at-a-time after the first run hit a transient
server rate-limit; ~1.1M tokens). 31 findings, severities adjudicated against code.
**The convergence gate held structurally** — no real incremental≠full break (the
dir-move-in case self-heals + a full pass is always correct); `analyze` is panic-safe
and `parent().kind()` derivation is robust. Fixed the must-fix set:
- **HIGH — `normalize_rel` root escape**: `../shared` from a root file collapsed to
  `shared` → false edge. Now returns `None` on escape (drops the edge). + guard test.
- **MEDIUM — getter/setter PK collision**: same-line accessors collided on
  `code_nodes` PK and one was dropped. Added `start_col` to the node + PK (schema v5).
  + `same_line_getter_setter_both_indexed` test.
- **MEDIUM — symbols backfill on upgrade**: restructured `migrate` to DROP the
  derived file tables on any version bump (preserving notes/proposals) so a warm pass
  rebuilds + backfills the AST-fed `symbols` column. + upgrade test.
- **MEDIUM — dir move-in**: `index_changed` now walks+indexes a moved-in directory's
  children (closes the one self-healing convergence gap).
- **MEDIUM — gate test strengthened**: added an ADD+DELETE+RENAME+MODIFY convergence
  test (not just one MODIFY), so the "proven" claim is honest.
- Cheap correctness: spec normalization (backslash + `?query`/`#hash`), self-loop
  edge skip, sorted `defined_in`, and a precise RRF doc note (RRF fuses by rank, so
  per-column bm25 weights order intra-leg while per-leg weights drive cross-leg — by
  design, not a dropped weight).
Green: clippy `-D warnings` clean · `cargo test` 244 passed / 1 pre-existing ·
brain lib 52 · brain_sandbox 13.

⏭ P2 deferred (logged, non-blocking — gate met): scoped def queries (drop
function-local const/object-method noise from get_symbol/impact); generators +
ambient/trait-signature/extern/private-method def coverage; module resolution
(tsconfig/pkg/Cargo) + Rust `use` edges; refs/calls edges + `brain_code_graph`/
`brain_neighbors`; rebuild_edges coalescing; a frontend impact view.
**P2's marquee (real AST graph + tiered impact), gate, and verification are done.**

### P3.1 — cache-stable gist core + the byte-stability gate ✅
`brain/gist/mod.rs`: `build_gist(db, project, name, intent, budget)` synthesizes a
token-bounded, **fingerprint-keyed** context bundle — always-kept freshness line +
relevant files (with their top AST symbols) + top memory notes — assembled with
per-layer caps + proportional char-budget trim (chars/4, [DP-21]). Key =
`blake3(project_fingerprint ‖ intent ‖ budget ‖ schema_version)`. Zero tokens to
build (pure index reads); secret-safe (draws only from the redacted index + scan-
redacted note titles). Determinism prerequisite fixed: search now has a secondary
`f.path` sort so bm25 ties order stably. New readonly helpers
(`project_fingerprint_readonly`, `symbols_for_path_readonly`) + `brain_build_gist`.
**P3 gate proven** (`gist_byte_identical_on_unchanged_relaunch`): two builds over an
unchanged index are byte-identical (and share the cache key); a content edit changes
the key. The thin-gist confidence behavior falls out (tiny budget / no hits → just
the freshness line).
Green: clippy `-D warnings` clean · `cargo test` 244 passed / 1 pre-existing ·
brain lib 52 · brain_sandbox 14.

### P3.2 — cold-start synthesis + gist write path ✅
`gist/synth.rs`: `synthesize_intent` (deterministic-given-state: project name +
sorted note titles; git/recent-files signal a documented refinement — must stay
deterministic to preserve byte-stability). `build_gist_auto` synthesizes when the
intent is blank; `write_gist` builds + writes the bytes to the agent file. Commands
`brain_build_gist` (now auto-synth) + `brain_write_gist` (→ `~/.koden/agent-<id>.txt`,
the existing `--append-system-prompt` channel). Tests: deterministic synth +
non-thin auto-synth gist + byte-stable + the write path lands the bytes.
Green: clippy `-D warnings` clean · `cargo test` 244 passed / 1 pre-existing ·
brain_sandbox 15.

### P3.3 — launcher wiring (gist injected at agent spawn) ✅
Recon mapped the flow: the launcher already feeds `~/.koden/agent-<agentId>.txt`
to `--append-system-prompt` (`App.tsx:914-916`), and `brain_write_gist` would
**clobber** that file. Fix (recon option a): in `handleSpawnTerminalAgent`, resolve
the pane's project (`resolveProjectForCwd` — longest-prefix match of the spawn cwd
against `brain_list_projects` roots), `brain_build_gist` (no write), and **prepend**
the gist as the cache-stable PREFIX of the worker prompt in the existing single
`native.writeFile` — one writer, gist first (prompt-cache-stable), launch command
untouched, fail-open, with a `toast.success` ("injected gist: N files"). New TS
bindings (`brainBuildGist`, `resolveProjectForCwd`, `Gist`). tsc + biome clean.
⏭ The ACTUAL injection (an agent launch picking up the gist) needs a live
`pnpm tauri dev` run — the cross-phase live evidence. Director-path gist + richer
confidence gate + snippet-text + git/recent-files synthesis remain refinements.

**P3 is functionally complete (backend + gate + launcher wiring).** Next: a P3
adversarial-verification pass, then P4 (budgeted reflect + crash-resume).

### P3.4 — adversarial verification + hardening ✅
8 reviewers (waved 4-at-a-time to dodge burst rate-limits) + synthesis, ~1M tokens,
across cache-stability, launcher-wiring, faithfulness, synth-determinism, secret-
safety, budget-quality, and concurrency-perf. 33 findings; all load-bearing claims
spot-verified against the code before acting. Fixed the must-fix + the cheap real wins:
- **HIGH — torn snapshot breaks the byte-identity gate (the headline)**: `build_gist`
  read the fingerprint (cache key) and the body across 4+N *separate* read-only
  connections, each taking its own WAL snapshot at open. The single worker could
  commit a reindex between opens → a gist whose advertised key (state A) doesn't match
  its bytes (state B), so the same key could map to two different byte strings under
  concurrency — directly violating the P3 gate (the single-threaded gate test can't
  see it). Fix: `open_readonly_snapshot` pins ONE deferred read transaction; added
  `*_with_conn` variants (`project_fingerprint`/`file_count`/`symbols_for_path`/
  `list_notes`, joining the existing `search_with_conn`); `build_gist`/`build_gist_auto`/
  `synthesize_intent` now thread that one connection so fingerprint, file_count, search,
  every symbols read, and notes observe one state. Also drops ~15 redundant opens/spawn.
  + **real concurrent-write regression test** (`gist_cache_key_stable_under_concurrent_writes`):
  a writer thread toggles the index while a reader builds 400 gists; asserts no cache
  key ever maps to two bodies, and that >1 state was actually observed (non-vacuous).
- **MEDIUM — case-sensitive cwd→project match**: `resolveProjectForCwd` compared paths
  case-sensitively while Windows/macOS fold case → gist silently un-injected on case
  drift. Now case-insensitive (a missed match only skips injection, so over-matching
  is the safe error).
- **MEDIUM — "stored cache" expectation**: clarified in the module doc that byte-
  identity is a *property of the deterministic single-snapshot build*, not a memoized
  blob — CONCEPT Flow C step 5 is satisfied by re-deriving identical bytes; there is
  deliberately no on-disk gist cache (nothing to stale/invalidate). [ponytail]
⏭ Honestly deferred (logged, non-blocking — LOWs / faithful deferrals): an explicit
[DP-22] confidence/thin gate (today thin only falls out incidentally when the index
is empty); snippet-text + graph-neighbor gist layers; note status/provenance downrank
([DP-26]); Director-path gist injection; toast wording when a gist is freshness-only;
git/recent-files cold-start synth signal. None affect the cache gate.
Green: clippy `-D warnings` clean · `cargo test` 244 passed / 1 pre-existing
(`authorize_spawn_cwd_blocks_symlink_escape`, Windows symlink privilege, untouched) ·
brain lib 52 · brain_sandbox 16 · tsc clean · biome lint clean.
⏭ Still pending for P3 (cross-phase): the live `pnpm tauri dev` run proving an actual
agent launch picks up the gist (the one piece that needs a GUI, not a test harness).

**P3 closed (backend + gate + launcher wiring + adversarial pass + hardening).**
Next: P4 (budgeted reflect + crash-resume).

## Phase 4 — Budgeted LLM reflect + crash-resume

Recon first: a 6-agent read-only fan-out grounded the build in exact source facts
(the Conductr reflect port — verbatim constants/prompt/schema; the brain store
migration-safety; the secrets+reqwest pattern; the resume signal substrate; the
P4-a pane-uuid plan; a Tier-2 capture-feasibility verdict). Then built the
deterministic, $0-testable core in three green milestones, ran an 8-reviewer
adversarial pass, and hardened.

### P4.1 — budgeted reflect (the only token-spending path) ✅
schema v6: `brain_budget` (global singleton) + `brain_budget_ledger`, CANONICAL/
preserved (absent from the upgrade DROP batch; + `upgrade_preserves_budget_state`).
`reflect/budget.rs` — crash-safe `check_and_reserve → (call) → reconcile`: a crash
between reserve and reconcile leaves a committed `reserved` row that the boot sweep
charges at its estimate (over-counts a crash, never leaks free spend); `spent_total`
monotonic, never recomputed. `reflect/schema.rs` — faithful Conductr port (verbatim
60/200/8 + SYSTEM_PROMPT with `(cap: 8)` + U+2014; loose-object tolerance; over-cap →
InvalidOutput as a Koden hardening). `reflect/digest.rs` — bounded, **metadata-only**
digest (index fields, never raw bodies). `reflect/llm.rs` — real Anthropic
`/v1/messages` client behind the `ReflectClient` seam (reqwest+rustls, block_on,
x-api-key, thinking:adaptive, output_config json_schema; status-only errors).
`reflect/proposal.rs` + `mod.rs` — kind→action map + `reflect_with_client` (offline/$0
fake-LLM testable core) + `reflect_once` (real wrapper). 7 sandbox tests drive the
full pipeline against the real index + a fake LLM (disabled/over-budget → zero
requests; happy path enqueues + charges actual; invalid JSON fails open but charges;
call-failure charges the estimate; dedup; empty-corpus noop).

### P4.1b — worker wiring + command surface + boot sweep ✅
`BrainEvent::Reflect`/`SetBudget` on the single-writer worker; boot
`sweep_orphaned_reservations`; commands `brain_reflect`/`brain_set_budget`/
`brain_budget_status` + bindings.

### P4.2 — crash-resume (Tier-1 + Tier-2 plan) ✅
`resume/`: `SessionKey = blake3(cwd ‖ agent ‖ pane_uuid)` (excludes the ephemeral
pty id); append-only per-pane JSONL journal (fail-open, size-gated tail compaction);
tolerant boot `recover_all` (drop trailing partial, guarded parse, skip cleanly-
exited) + TTL GC; `resume_command` → Tier-2 `--resume <id>` ONLY for `claude` with a
captured id, else Tier-1. Worker journals every signal on the writer thread; boot
recovery before the agent listener. 11 unit + 1 chain test.

### P4.3 — adversarial verification + hardening ✅
8 reviewers (waved 4-at-a-time) + synthesis, ~1.28M tokens, across the money path,
fail-open, secret-safety, faithfulness, resume correctness, schema/migration,
concurrency, and provider facts. Verdict: core sound; 4 genuine must-fixes (the
panel over-counted HIGHs via duplication, which synthesis corrected). All fixed:
- **SECRET LEAK (HIGH ×2)**: note `anchors` reached the cloud unredacted, and doctor
  finding `detail` strings interpolated raw frontmatter (`superseded_by` etc.). Fix:
  redact anchors at scan (the table is indexing, like titles) AND a belt-and-suspenders
  `secrets::redact` over the ENTIRE assembled message immediately before the cloud
  send — so no single un-redacted field can leak. + `reflect_redacts_secrets_before_cloud`
  (plants secrets in an anchor + superseded_by, asserts neither reaches the fake client).
- **MONEY UNDER-CHARGE (HIGH)**: a 2xx with 0/0 reported usage charged $0 (Anthropic
  still bills input). Fix: floor an implausible 0/0 success to the conservative
  estimate. + `reflect_zero_usage_charges_estimate`.
- **CEILING NOT ENFORCED ACROSS A FAILED RECONCILE (HIGH)**: the pre-flight gate read
  only `spent_total`. Fix: `check_and_reserve` now also counts outstanding committed
  `reserved` rows against the ceiling, and reconcile failures are logged (not
  swallowed); a stranded reservation can't let a later reflect overspend, and the
  boot sweep folds it. + `outstanding_reservation_counts_against_ceiling`.
- **OUTPUT SCHEMA (MEDIUM)**: `additionalProperties: true` would 400 structured
  outputs. Fix: strict `false` + enumerated props (+ a schema-strictness assertion).
- **TIMESTAMP UNIT (MEDIUM)**: the budget DDL seeded `updated_at` in seconds while all
  Rust writes use ms. Fix: seed in ms; documented the columns as epoch-ms.
- **RESUME SESSIONKEY SPLIT (was filed HIGH)**: `agent` rides only the `started`
  signal, so non-started signals hashed a different key (exited never reached the
  started journal → stale card forever; Tier-2 never fired). Fix: remember the agent
  on the session and reuse it for every signal (like `remembered_cwd`).
- Cheap hardening: NaN/Inf-safe budget gate (fail-safe to no-spend); migration wrapped
  in ONE transaction (atomic "advanced to v6"); `let-else` over a latent `expect()` in
  reflect_once; fsync-before-rename on journal compaction; honest docs (metadata-only
  digest, kind→action divergence rationale, gist-key rotation on schema bump,
  TTL-only GC).
Green: clippy `-D warnings` clean · `cargo test` 277 passed / 1 pre-existing
(`authorize_spawn_cwd_blocks_symlink_escape`, untouched) · brain_sandbox 26 · tsc +
biome lint clean.

⏭ P4 deferred — the GUI/external-integration half (explicitly backend-only; needs the
running app, not a test harness): **P4-a** the persisted frontend pane-uuid +
`KODEN_PANE_UUID` env (must live on the serialized leaf node; restart-survival is a
live-verify item — until it lands, resume keys on the spec-sanctioned `cwd+agent`
fallback); the **recovery-card UI** + `[Resume]`/`[Dismiss]` + journal-delete-on-
dismiss (`brain_recovered_panes`/`resume_command` are built + tested but not yet
rendered/wrapped); **Tier-2 live session-id capture** (recon verdict: achievable via a
Claude status-hook field on the agent bus + bus unification B5); the **wizard budget
step**; and the **one real-key smoke test**. Resume acceptance gates 4–5 are therefore
backend-only for now.

**P4 deterministic core closed (budget + reflect + resume + adversarial pass +
hardening).** Next: P5 (deferred semantic seams) / the V2 advanced track.

## Phase 5 — Deferred semantic seams ✅

P5 is decision-DEFERRED by spec: v1 ships ONLY the shape that lets semantic slot in
later with zero schema/search churn — no functional semantic search.

- `brain/search/vector.rs`: the stable `Embedder` + `VectorStore` traits + `DocId`
  (the same `project\0path` id space the lexical legs use, so a future vector leg
  fuses without remapping). Compiled in the v1 default build.
- `brain/search/mod.rs`: `registered_search_legs()` = `["identity","content"]` — the
  no-vector-leg invariant made checkable. The semantic vector leg is NEVER registered
  in the default build.
- schema v7: `brain_semantic_meta` (the `embedderId` header), CANONICAL/preserved
  (absent from the upgrade DROP batch), seeded empty (`""`,0) in v1; set at
  enablement so a later build detects a model/dim change and rebuilds. No
  `brain_vectors` table in v1 (created lazily at enablement).
- `semantic` cargo feature (DEFAULT-OFF, absent from the shipped binary) gating a
  real, **dependency-free reference impl** (`search/reference.rs`: a deterministic
  hashed-token `HashEmbedder` + a brute-force cosine `BruteForceStore`). It exists to
  prove the seams compose end-to-end and to keep the gated code from bit-rotting —
  NOT as the production stack. The production swap (fastembed-rs ONNX embedder +
  hnsw_rs persisted ANN) lands behind the SAME traits at enablement time, deps pinned
  then (`semantic = ["dep:fastembed","dep:hnsw_rs"]`); no heavy ONNX/HNSW deps enter
  the tree until then (ponytail: no speculative heavy infra for an off-by-default
  feature).
- CI: a `--features semantic` clippy + nextest step in the `rust` job (anti-bit-rot
  gate 3), while the default jobs prove the feature is OFF by default.

Tests: `semantic_feature_absent_from_default_build` (cfg, default-only),
`search_index_has_no_vector_leg_in_v1`, `semantic_header_seeded_empty_and_preserved`
(migrate: present, empty, survives an upgrade), `semantic_header_persisted_empty_in_v1`
(sandbox via `semantic_meta_readonly`), and the gated `seams_roundtrip_ranks_similar_first`
+ `upsert_replaces_not_duplicates` (run under `--features semantic`).

All three P5 acceptance gates met: (1) no functional semantic in v1 — default build
links no embedding/vector code; (2) seams + `embedderId` header are real + persisted,
enabling later needs no v1 schema migration (only new object = `brain_vectors`,
lazy); (3) gated code compiles + passes under `--features semantic`.

Green: default — clippy `-D warnings` clean · `cargo test` 280 passed / 1 pre-existing
(`authorize_spawn_cwd_blocks_symlink_escape`, untouched) · brain_sandbox 27. semantic —
`clippy --features semantic` clean · gated reference tests pass.

### P5 verification + hardening ✅
3 reviewers (feature-isolation, schema/migration, seam-design) + synthesis, ~296k
tokens. Verdict: **sound to close — zero must-fixes**; all 3 acceptance gates verified
against the code (feature provably absent from the default binary; header canonical/
preserved so enabling needs no v1 schema migration; gated code compiles + is CI-tested
under `--features semantic`). Findings were doc-honesty + test-robustness; fixed the
honesty-relevant ones now (the rest fold into the enablement phase):
- The "vector leg slots in with zero churn" comment conflated zero-SCHEMA-churn (true)
  with zero-CODE-churn (false) — there is no runtime leg registry. Reworded to state
  plainly that enabling the vector leg is a localized edit to `search_with_conn` +
  `SEARCH_LEG_LABELS` (only `weighted_rrf` is already N-leg).
- The no-vector-leg gate was a self-referential literal. Now `registered_search_legs()`
  delegates to a single source of truth (`SEARCH_LEG_LABELS` in sqlite.rs, next to the
  legs it builds), so the gate can't drift from the live path.
- `semantic_meta_readonly` is now genuinely fail-soft (missing table/row → `("",0)`),
  matching its doc, so a pre-migrate read can't error.
- Cheap hardenings: v7 gist-key-rotation doc note; a re-open-no-clobber migrate
  assertion; reference-impl defensive-branch tests (cosine length-mismatch, zero-norm
  empty text, upsert length-mismatch, kNN truncate/best-first).
⏭ Deferred to the semantic-enablement phase (logged): a `cargo tree -e features` CI
tripwire (no optional deps exist yet, so trivially true today); spec↔impl test-name
alignment; the cosine-pre-normalization contract doc on the traits (a production-
embedder concern).
Green: default clippy `-D warnings` clean · `cargo test` 280 / 1 pre-existing ·
brain_sandbox 27 · `--features semantic` clippy clean + gated tests pass.

**P5 closed (seams + header + default-off feature + reference impl + CI gate +
adversarial pass + hardening).**
V1 (P0→P5) is functionally complete. Next: V2 advanced track (stale-ADR curation,
HNSW ANN + real embedder enablement, Tier-2 capture, richer resume, cross-project
graph) + the cross-phase live-evidence pass (`pnpm tauri dev`, real-kill crash sim,
real-key smoke).

## Live-evidence pass (partial — what a test harness CAN prove)

Produced autonomously (headless), the parts that don't need the GUI:
- **V1 links into the real app binary**: `cargo build --bin koden` → `koden.exe`
  (~42 MB), exit 0. The whole brain subsystem ships in the shipped binary, not just
  under `cargo test`. `cargo build --features semantic` also links the gated stack.
- **REAL-kill crash sim** (BUILD-PROMPT §13.29 — a genuine process kill, not the
  in-process mock the unit test uses): new `src-tauri/examples/brain_crash_sim.rs`.
  Process 1 opens the store, journals `started`+`working` for a pane, then
  `std::process::abort()` (exit 127 — killed, no clean exit / no `exited` marker);
  the journal survives on disk; a FRESH process 2 runs `recover_all()` and recovers
  exactly one pane as still-`working` → `PASS`. Repeatable:
  `cargo run --example brain_crash_sim -- write <dir>` then `… -- recover <dir>`.
- The §6.5 offline sandbox already drives the REAL pipeline end-to-end (walk →
  blake3 → secrets-redact → FTS; reflect_with_client over a fake LLM; record_event →
  recover_all) across 26 `brain_sandbox` integration tests — real pipeline, not unit
  mocks.
- **Headless brain validation harness** (`src-tauri/examples/brain_cli.rs`). NOTE on
  naming (corrected after user feedback): the project's real **headless Koden CLI**
  is the `feat/koden-cli` SIBLING BRANCH — `src-tauri/src/bin/koden-cli.rs` +
  `src-tauri/src/cli/{doctor,fs_search,agent_detect,pty_echo,git_status,output}.rs` +
  `pty/headless.rs`. This `feat/koden-brain` branch was cut off `main` (not off
  koden-cli) per the mandate, so it can't see that binary; and koden-cli has no
  *brain* subcommands (the brain lives only on this branch). There is ALSO an e2e
  harness (`pnpm test:e2e` + `window.__KODEN_TEST__` + `scripts/launch-sandbox.mjs`),
  currently not runnable (Phase-0 WebDriver spike pending) and brain-unaware. So
  `brain_cli` is an INTERIM standalone Rust driver for this isolated branch; the
  clean end state is folding brain subcommands into `feat/koden-cli`'s `cli/` when
  the two workstreams merge (the BUILD-LOG header's "merges later"). My earlier "no
  headless CLI exists" was wrong on both counts. `brain_cli` drives the WHOLE V1
  brain end-to-end:
  `cargo run --example brain_cli -- all` runs a 14-check battery against a built-in
  fixture and exits 0 only if all pass. Live result — **14/14 passed**: index (4
  files) · memory scan · search · AST symbol + tiered impact (login→session) · gist
  byte-identical + secret-safe · secret-not-indexed · doctor (broken-anchor proposal)
  · reflect disabled-by-default (no spend) + enabled ($0 fake LLM, charged $0.0020) ·
  resume recover-working + skip-exited · semantic header empty. Also smoke-ran against
  REAL code (`brain_cli index/search/impact src/modules/brain`): 36 Rust files
  indexed; `session key derive`→`sessionkey.rs`, `budget reserve reconcile`→
  `reflect/budget.rs` top-ranked; impact resolves defs + lexical dependents (Rust AST
  `use`-edges are the documented P2 deferral, so ast_dependents is empty and the
  lexical over-approximation carries it).

⏭ DEFERRED FOLLOW-UP (user decision 2026-06-21 — chose to proceed to V2): wire the
**real Koden e2e CLI (`pnpm test:e2e`) to cover the brain** — the Phase-0 WebDriver
spike (`tauri-plugin-webdriver` behind a dev `webdriver` feature + un-stub
`wdio.conf.ts`) + brain methods on the `window.__KODEN_TEST__` bus (the brain
commands exist; just not exposed on the bus) + brain e2e scenario files. This is the
only way to test the brain through the REAL app/worker headlessly; bundle it with the
Phase-0 spike. Until then the pre-V2 gate is: brain_cli 14/14 + real-kill crash sim +
26 integration tests.

⏭ Still needs the running GUI app / the user (cannot be driven from a test harness
here): the **fake-claude → agent-detect → brain-worker replay** (the worker is
Tauri-`AppHandle`-coupled; needs `pnpm tauri dev` + a real terminal tab — harness in
`scripts/README-sandbox.md`); the **live gist-injection-at-spawn** proof; the
**recovery-card UI** + Resume/Dismiss (P4-a + GUI, not built); and the **one real-key
reflect smoke** (needs the user's Anthropic key — real spend, must not run without
explicit authorization; a real-kill BUDGET sim rides along with it, since leaving a
`reserved` row requires a real reflect — its crash-safety is already unit-proven by
`crash_midcall_is_overcounted_never_leaked`).

## V2 — advanced track

### V2.1 — Stale-ADR / memory curation (CONCEPT Flow G) ✅ (core)
The write-judgment scenario. `brain/curate/`:
- `detect.rs` — the two-stage significance gate (§5.4). REUSES the P1 doctor's
  `check()` for `broken_anchor`/`stale_revalidate` and ADDS `superseded_present`
  (a note whose `superseded_by` resolves to an existing note). Transparent weighted
  score → bands: a LONE `broken_anchor` (0.6) SKIPs (the doctor already proposes
  re-anchoring — no double-proposing); a single strong signal ESCALATEs to the LLM
  (the keep-as-history vs obsolete call earns the paid judgment); stacked signals
  ACT ($0).
- `schema.rs` — the Tier-2 verdict (`classification` still_valid|keep_as_history|
  obsolete + graded `action` archive|supersede|update|delete + confidence + reason),
  loose-parsed. Preserve-biased system prompt ("old ≠ wrong"). `delete` down-grades
  to the `Archive` apply-op — the Librarian never proposes silent deletion; deletion
  stays a human call.
- `mod.rs` — `curate_with_client` (testable core): ACT-band → $0 archive proposals;
  ESCALATE-band → budget-gated Tier-2 classify → graded archive-biased proposal.
  REUSES the P4 money path (one shared budget ledger + `ReflectClient` seam +
  charge-on-uncertainty + 0/0-usage floor). `curate_act_only` runs detection + the
  $0 ACT band when there's no key; `curate_once` is the real wrapper. All output
  flows into the existing human-gated P1 queue, deduped by signature, reject-sticky.
- Wired: `BrainEvent::Curate` on the single-writer worker + `brain_curate` command +
  `brainCurate` binding.
- Tests: 9 unit (detection bands incl. lone-broken-anchor-skips + dangling-
  supersession-not-a-candidate; verdict parse + delete→archive downgrade) + 4
  sandbox (act+escalate enqueues archive/supersede + charges; escalation-disabled
  still acts $0; still-valid → no proposal; act-only no-key).
Green: clippy `-D warnings` clean · `cargo test` 289 / 1 pre-existing · brain_sandbox
31 · tsc + biome lint clean.
⏭ Deferred (documented refinements): the `age`/`high-churn`/LLM-contradiction
detection signals (need git + a created timestamp on NoteRecord); a curation status
command/meter; the recovery/curation review-card UI (GUI).

### V2.1 verification + hardening ✅
3 reviewers (money-path/fail-open · detection-gate · proposal-safety) + synthesis.
Verdict: **sound to close, no must-fix** — every hard rule independently verified
(curation never edits/deletes a user file; never proposes silent deletion — delete
down-maps to Archive; no spend leak/under-charge on the SHARED ledger; no panic;
digest redacted pre-cloud; reject-sticky). Fixed the real correctness/quality
findings the panel surfaced:
- **Supersession direction (real under-detection)**: Flow G keys on the NEWER note's
  forward `supersedes` edge, but detection used only the old note's `superseded_by`,
  and `supersedes` was parsed yet never persisted — a spec-correct corpus yielded
  ZERO candidates. Fixed: persist `notes.supersedes` (schema v8; `notes` reclassified
  as DERIVED-from-disk so it joins the upgrade rebuild + the next scan repopulates it
  with the column), thread it through upsert/select/NoteRecord, and detect via the
  UNION of both edges (de-duped, no double-weight). + forward-edge + both-edges tests.
- **Multiple broken anchors**: 0.6×N crossed LOW, defeating the "lone broken_anchor
  skips" rule (the doctor owns re-anchoring). Fixed: broken_anchor saturates to one
  unit per note → can never cross LOW alone. + multi-anchor-skip test.
- **Self-supersession**: `superseded_by`/`supersedes` == own id resolved → flagged.
  Fixed: guard `stale != newer`. + test.
- **Observability**: curate now uses reflect's `reconcile_or_log` (made pub(crate))
  instead of swallowing reconcile errors — identical money-path discipline.
- **Front-door gate**: `curate_once` now shares reflect's `pre_flight` (no client
  built when the ceiling is off; precise reason stamped).
- **Coverage**: added 3 curate money-path sandbox tests mirroring reflect
  (OverBudget-stops-escalation, call-failure-charges-estimate, 0/0-usage-floors).
- Nits: corrected stale "reflect-only" boot-sweep/ledger comments (one shared ledger,
  reflect OR curation), the `proposal.source` comment (+`curate`), the ACT-band doc
  overclaim, and added a deferred-Flow-G-signals marker in detect.rs.
⏭ Deferred (documented): the `supersedes`-back-link-only convention vs union is now
moot (union handles both); stale_revalidate(doctor) vs archive(curation) are
intentionally distinct cards; churn + LLM-contradiction signals remain refinements.
Green: clippy `-D warnings` clean · `cargo test` 293 / 1 pre-existing · brain_sandbox
34 · tsc + biome unaffected.

**V2.1 closed (stale-ADR curation + adversarial pass + hardening).**

### V2.2 — ranking-quality calibration ✅
Turns the GUESSED BM25/RRF weights into MEASURED ones (the scout fan-out's top
recommendation: build the measurement layer before temporal re-rank, which is riskier
and must be judged against a discriminating benchmark).
- Parameterized the search core: `search_with_weights(conn, …, &SearchWeights)` +
  `SqliteIndex::search_weighted` (the calibration seam); `search_with_conn` is now a
  thin wrapper with `SearchWeights::default()`. + a byte-identical equivalence test.
- `brain_bench.rs`: graded metrics (recall@5 floor gate + MRR + precision@1) and 3
  CONFUSER pairs — the query term lives ONLY in the target's path and ONLY in a
  distractor's body, so the target sits solely in the identity leg and the distractor
  solely in the content leg → identity-vs-content weight alone decides rank-1. Without
  these the corpus scored a vanity 1.0 for any weighting.
- Offline calibration sweep (`weight_sweep_reports_mrr_subject_to_zero_leaks`,
  `#[ignore]`d): grid-sweeps weights maximizing MRR SUBJECT TO zero negative-control
  leaks (the regularizer). Result: STRICT identity-dominant (rrf_identity > rrf_content)
  → MRR 1.000; content tie-or-dominant (rrf_identity <= rrf_content, incl. the 1.0==1.0
  boundary which loses on the ascending-id tie-break) → MRR 0.875; all leak-free. So production
  `rrf_identity = 1.5` is in the optimal band — `provisional` comment downgraded to
  `measured` (with the §13.12 before/after note). RRF fuses by rank, so path_bm25 ∈
  {2,3,4} doesn't move MRR — the leg weights are the load-bearing knob.
- CI-running anti-vanity guard (`production_weights_beat_content_dominant`): asserts
  default MRR > content-dominant MRR, so the 1.000 can never be a vanity number.
- BENCH.md updated with the graded numbers + the calibration section.
Green: clippy `-D warnings` clean · brain_bench 3 + 1 ignored · lib 294 / 1
pre-existing · brain_sandbox 34. ⏭ temporal re-rank [DP-12] is the natural follow-on
(now has a discriminating benchmark to be judged against); its byte-identity trap is
documented (recency MUST be a stored snapshot-stable timestamp, never now()).

### V2.2 verification + hardening ✅
3 reviewers (refactor-equivalence · benchmark-honesty · calibration-soundness) +
synthesis. Verdict: the load-bearing rules HOLD and were exhaustively verified — the
refactor is a PURE extraction (`git show` confirmed; production consts byte-identical;
determinism + tie-break preserved → gist byte-identity gate intact), and the benchmark
genuinely discriminates (not a vanity 1.0). But the panel caught two **false "measured"
claims** (exactly the §13.12 failure) — fixed:
- **Corpus count**: BENCH.md's "real run" block said "13 files" but the redesigned
  confusers make it **16** (10 base + 3 pairs × 2). Re-ran and re-pasted the actual
  verbatim output (16 files).
- **`>=` vs strict `>`**: the optimal band is STRICT `rrf_identity > rrf_content` — the
  boundary 1.0==1.0 scores 0.875 (ties break to the distractor by ascending id), so
  `>=` was false and would have misled a tuner into lowering the default to 1.0 (a real
  regression). Corrected in sqlite.rs, BENCH.md, and the V2.2 log entry.
Plus the should-fix test-quality items: de-tautologized the equivalence test (pin the
production weights as inline literals + assert `Default` matches them field-by-field,
so default-weights drift actually trips it); raised the vacuous precision@1 floor
0.75→0.90 (the 9 base positives alone met 0.75); added a CI boundary assertion so the
strict-`>` claim is enforced (production beats the 1.0 equal-weight case); scoped
`search_weighted`'s doc (writer-conn calibration seam, not for production search). P0's
9/9 bench figure annotated as superseded by the V2.2 corpus.
Green: clippy `-D warnings` clean · brain_bench 3 + 1 ignored · lib 297 / 1 pre-existing.

**V2.2 closed (ranking calibration + adversarial pass + honesty corrections).**

### V2.3 — temporal re-rank ([DP-12]) ✅ (core)
Recency + frequency feed a deterministic, snapshot-stable multiplicative boost on
search — the natural follow-on now that V2.2 gives a discriminating benchmark to judge
it against. The whole design hinges on NOT breaking the P3 gist byte-identity gate
(`search_with_conn` feeds the gist's "Relevant files"), so:
- schema v9: `files.accessed_at_ms` + `files.accessed_count` (STORED). `files` is
  DERIVED — the upgrade now DROPs it (was DELETE) so the columns backfill on the next
  warm pass.
- `record_access(project, rel, now_ms)` is called by the worker ONLY on a real content
  change (`index_file → Ok(true)`), never on an unchanged hash-skip — so a warm pass
  over an unchanged index leaves `accessed_at_ms` fixed. Recency advances only when the
  fingerprint already changes (the gist is expected to differ then anyway).
- `temporal_boost(accessed_at_ms, accessed_count, ref_ms)`: quantized buckets (recency
  ladder <1d/<7d/<30d/<90d + log2 frequency), bounded `(1+RECENCY_W·r)·(1+FREQ_W·f)`.
  Applied as a POST-FUSION step in `search_with_weights` (RRF stays leg-pure), re-sorted
  with the SAME (score desc, id asc) comparator. `ref_ms = MAX(accessed_at_ms)` read off
  the SAME connection's snapshot — never `now()` on the read side. All-zero (unstamped)
  rows → a uniform boost → no reordering, so existing tests + the bench are unaffected.
- Determinism PROVEN: `temporal_boost_is_byte_stable_across_reads` (two searches over a
  stamped, unchanged index are identical) + the existing `gist_byte_identical_on_unchanged_relaunch`
  still passes (recency uniform across the two builds). `recency_reorders_equal_score_files`
  proves a fresher equal-score file is promoted; `temporal_boost_rewards_fresh_and_frequent`
  unit-tests the pure boost (fresh+frequent > stale+rare, bounded, uniform when zero).
Green: clippy `-D warnings` clean · `cargo test` 300 / 1 pre-existing · brain_sandbox
34 · brain_bench 3 + 1 ignored.
⏭ Deferred refinements: an explicit `touch_file` on agent file-open (a real access
counter beyond index-time recency); the same boost on the gist Memory (notes) layer;
weight tuning of RECENCY_W/FREQ_W via a recency-labeled bench fixture.

### V2.3 verification + hardening ✅
3 reviewers (determinism · boost-correctness · schema+wiring) + synthesis. The literal
byte-identity gate was re-verified HELD (no wall-clock in the read path; record_access
fires only on a hash-changing reindex; the re-sort comparator is byte-identical to
weighted_rrf). But the panel found TWO real defects that the narrow same-process gate
hid — fixed before close:
- **Boost could BURY a path-match (defeats [DP-2])**: max boost was 1.875 but the
  cross-leg RRF margin is only 1.5, so a fresh+frequent body-only hit could outrank a
  stale path-match — AND it was invisible to CI (index_dir stamps uniformly). Fixed:
  RECENCY_W 0.5→0.25, FREQ_W 0.25→0.1 (max boost 1.375 < 1.5), with a static invariant
  test (`temporal_boost_cannot_flip_cross_leg`) AND a bench guard
  (`temporal_boost_cannot_bury_path_match`) that stamps the distractor fresh+frequent
  and the target stale and asserts the path-match still wins (fails under the old weights).
- **Gist cache key didn't cover temporal state (cache poisoning)**: the body order
  depends on accessed_*, but the key only hashed (path,hash) — two index histories
  converging to the same content (same key) but different access counts produce
  different bytes. Fixed: a SEPARATE `project_temporal_digest` (blake3 over sorted
  (path, accessed_at_ms, accessed_count)) folded into the gist KEY — kept separate from
  the content fingerprint so the fingerprint stays portable. + `gist_key_covers_temporal_state`.
- should-fix: unstamped files (accessed_at_ms==0) now get a NEUTRAL recency factor
  (not "maximally stale"); ref_ms is now PER-PROJECT even on a project=None search
  (no cross-project reorder); a `debug_assert!(score.is_finite())` guards the NaN
  footgun for a future vector leg; docstrings corrected (the boundedness invariant +
  the intentional quantization cliffs).
Green: clippy `-D warnings` clean · `cargo test` 301 / 1 pre-existing · brain_sandbox
35 · brain_bench 4 + 1 ignored.

**V2.3 closed (temporal re-rank + adversarial pass + boundedness/cache-key hardening).**
