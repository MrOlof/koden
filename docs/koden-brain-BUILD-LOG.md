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
