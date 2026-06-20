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
the negative control + the semantic band.

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

⏭ P2 remaining: AST graph tables (nodes/edges) + imports/refs/calls extraction +
module resolution (tsconfig paths / pkg exports / Cargo members) + forward/reverse
adjacency with incremental relink + `brain_code_graph`/`brain_code_impact`/
`brain_neighbors` + the property test "incremental relink == full rebuild".
