# ADR-010: Brain module correctness review — confirmed findings + fix plan

Status: Accepted — 2026-07-03 · **EXECUTED 2026-07-06** (overnight run, all clusters fixed + adversarially verified; perf pair still deferred)

## Execution record (2026-07-06, overnight)

Executed by a 37-agent workflow (sequential fix clusters, 2 adversarial verifiers per
cluster — correctness + regression lenses — with a repair loop, then a full test sweep).
Eight checkpoint commits on `feat/koden-brain` (`f989238..3036916`, 28 files, +2608/−299):

| Commit | Cluster |
|---|---|
| `6b955f3` | 1 — reconcile-delete safety (data-loss class) |
| `3558854` | 2 — watcher gap (armed pre-walk + buffered; Rescan/Err → reconcile; project-relative skip-dirs) |
| `4b7990e` | 3 — security allowlists (`brain_write_gist` agent_id, tier2 session id) |
| `5635bcf` | 4 — corrupt-cache rebuild (rename-aside + retry once, canonical-table salvage, BUSY≠corrupt bounded retry) |
| `0a40b51` | 5 — paid-path economics (failure classification + backoff, digest hash on InvalidOutput, pre-pay reject-sig check, `max_completion_tokens`, AccountKey redaction reachable, monthly→cumulative comments) |
| `ffa4f91` | 6 — 12 commands async via blocking pool (git::commands idiom); `PanicStatusGuard` drop-guard → Degraded |
| `5f047e3` | 7 — beyond the ordered plan: TS def-query scope-anchoring (`is_ts_module_scoped`), Windows-only case-fold `resolve()`, `is_sane_root` on `brain_add_project`, rollback-safe `brain_remove_project`, HNSW upsert-replace + trait `remove`, FTS5 query-token dedupe, BrainPane in-flight poll guard |
| `3036916` | 8 — ADR-011 gist upgrades (known-unknowns + per-claim freshness labels) |

Key semantic changes: `FileOutcome{Indexed,NotIndexable,Absent,Unknown}` +
`Walked{files,complete}` — error paths are now "unknown", never "absence"; deletion
everywhere requires positive `NotFound` evidence; partial/truncated walks never feed
reconcile-delete (same rule in the memory-note reconcile — the unverified
`memory/mod.rs:174` claim was CONFIRMED during fixing). `SCHEMA_VERSION` 9→11 (derived
tables rebuild; gist cache keys rotate). The cluster-7 "Rescan ignores project filter"
finding turned out already fixed by cluster 2 (`no_change_needed`).

Verification: `cargo test --lib` 342 passed (only the known pre-existing Windows
symlink-privilege failure), `brain_sandbox` 48/48 (independently re-run by the
orchestrator), `pnpm check-types` clean, vitest 1090/1090 (known `eager-budget`
env failures only). Confirmed-defect repairs during the run: cluster 5 (1 LOW,
contradiction-dedup narrowing) and cluster 7 (2 MED + 2 LOW, incl. a flaky HNSW test
and an `is_sane_root` canonicalization gap) — all repaired and re-verified to zero.

Still open after this run: the deferred perf pair (`sqlite.rs` temporal-boost scan +
`rebuild_edges` O(project) per delta) and live GUI/real-run validation.

### Hardening addendum (2026-07-06 evening, `a3ef629..1cf968f`)

Same fix→verify→commit discipline, closing the behavioral-sim gaps: inline-secret
redaction moved to the index chokepoint + detector (d) rewritten to wholesale
between-marker PEM redaction with a per-block `PEM_BLOCK_LINE_CAP=1024`
(`a3ef629`+`0d9159e`, permanent regression test `tests/secret_index.rs`); the cluster-5
fail-streak cap got a real integration seam (`librarian_round_step` extraction +
`tests/librarian_rounds.rs`, `b114704`); `.gitignore` now honored in non-git roots,
project-bounded, watcher/dir-event/full-walk agreement proven (`1cf968f`; UI walkers
fs/tree|grep|search still `require_git(true)` — future task). **First-index SLA settled
in release build: 29.1 s / 1592 files (debug 61 s) — over the §12.1 5–15 s target ~2×;
searches 2–4 ms.** The perf pair is now the named lever for closing the SLA gap.

## Context

Full adversarial review of the Brain module (`src-tauri/src/modules/brain/`, all 46 files,
plus the frontend brain surface) on branch `feat/koden-brain` at `f989238`. Method:
10 parallel dimension reviewers (store/FTS5, migrations, tokenize+rank, vector/HNSW,
worker+watcher, AST graph, reflect/budget/secrets, memory/curate/resume, command
surface + bindings, and an end-to-end index-coherence sweep), then **2 adversarial
refuters per finding** (correctness lens + reachability lens). 142 agents, ~8.8M tokens.

Result: 66 raw findings → **48 confirmed, 3 disputed, 2 refuted, 13 unverified**
(the verify agents for all 10 `memory-curate-resume` findings + 3 `reflect-budget`
findings died on an API spend limit — those are code-cited by the reviewer but not
independently refuted; treat as strong leads, re-verify or just check while fixing).

## What held up under attack (don't churn these)

- **FTS5 injection impossible by construction** — `tokenize.rs` emits only `[a-z0-9]+`
  runs; no quote/`NEAR`/`*` can reach `MATCH`. Verified end-to-end.
- **Concurrency compile-time enforced** — worker owns the single writer `Connection`
  (`!Sync`), commands use read-only WAL connections + pinned snapshot for multi-reads.
- **Migrations atomic + crash-safe** — DROP-derived + DDL + version stamp in one txn;
  canonical/derived split has regression tests.
- **`tokenize.rs` is a faithful Conductr port** (rule-by-rule verified vs `lexical.ts`);
  RRF math safe, deterministic tie-breaks.
- **P5 semantic seam disciplined** — reference embedder default-OFF, comparators match,
  leg labels pinned by test.

## Decision — fix in this order

1. **Reconcile-delete safety** (the only data-loss class) — deletion needs positive
   evidence (`NotFound`), never inference from read failure or walk truncation.
2. **Watcher gap** — arm watcher BEFORE the warm walk (buffer events until it completes);
   honor the notify `Rescan` overflow flag; make skip-dir check project-relative.
3. **Two one-line security fixes** — `brain_write_gist` agent_id allowlist;
   `tier2.rs` session-id allowlist (`[A-Za-z0-9_-]`).
4. **Corrupt-cache rebuild path** — restores ADR-006's "rebuildable cache" contract.
5. **Librarian retry classification + backoff**; store digest hash on `InvalidOutput`;
   check reject signatures BEFORE paying.
6. **Commands off the main thread**; then perf pair (temporal boost, incremental edges)
   when repo scale demands it.

## The findings

### 1. Data loss / index-wipe cluster (fix first)

| Sev | Where | Bug |
|---|---|---|
| HIGH | `worker.rs:491` | Reconcile-delete conflates *unreadable* with *deleted* — an unavailable project root (unmounted drive, permission blip) wipes the whole project index incl. non-rebuildable temporal state + pending paid proposals |
| HIGH | `worker.rs:174` | Watcher armed only AFTER warm population — edits during the initial index are permanently missed (no event, hash already recorded) |
| HIGH | `freshness/watch.rs:100` | `collect()` drops notify `Err` results and ignores the `Rescan` overflow flag — missed events never reconciled |
| MED | `freshness/walk.rs:79` | `MAX_SCANNED` (50k) truncation feeds reconcile-delete: files past the cap pruned each full pass, re-indexed by watcher — permanent oscillation on big repos |
| MED | `worker.rs:528` | `under_skip_dir` checks ABSOLUTE path components — a project rooted under any dir named `dist`/`build`/`target`/`vendor` gets zero incremental updates |
| MED | `worker.rs:440` | TOCTOU on the 1MB size cap: `index_one_file` reads unbounded, minutes after the stat |
| MED (unverified) | `memory/mod.rs:174` | Transient note-read failure (Windows AV/editor lock) → note reconcile-deleted → pending PAID proposals destroyed in the same txn; a failed `read_dir` wipes every note+proposal in the project |

Shared root cause for the top three: error paths and edge signals treated as "absence"
rather than "unknown".

### 2. Corrupt cache = permanently dead brain

- **MED (filed high, both refuters downgraded)** `worker.rs:67` — corrupt `index.sqlite`
  fails `migrate()` at the first PRAGMA → `Degraded` → worker thread exits, no respawn,
  no rebuild command → every launch dead until user manually deletes an undiscoverable
  app-data file. Contradicts ADR-006. Worse than filed: transient `SQLITE_BUSY` at boot
  also kills the brain for the session (untyped error handling). Fix: match
  `SQLITE_CORRUPT`/`NOTADB` on open → rename cache → retry once. CAREFUL: `proposals`,
  `reject_signatures`, `brain_budget` are canonical — salvage, don't nuke.
- MED `migrate.rs:29` — version-read errors conflated with "fresh db" (skips derived
  rebuild, then stamps current). MED `migrate.rs:53` — downgrade silently stamps version
  DOWN without rebuilding.
- Migration-dimension notes: no ALTER mechanism exists for canonical tables (first future
  canonical column add will break upgraded installs); no test asserts every derived table
  appears in the drop list.

### 3. Command surface

| Sev | Where | Bug |
|---|---|---|
| HIGH | `commands.rs:193` | Path traversal in `brain_write_gist` via unsanitized `agent_id` |
| HIGH | `commands.rs:102` | All brain commands synchronous on the Tauri main thread with 5s `busy_timeout` — UI-wide freezes during indexing bursts |
| MED | `commands.rs:40` | `brain_add_project` accepts any dir (drive root, `~`) — `is_sane_root` only guards the boot seed |
| MED | `commands.rs:57` | `brain_remove_project` mutates registry before enqueueing the prune — failure leaves orphaned rows + diverged persisted state |
| MED | `worker.rs:194` | Rescan ignores its project filter — any targeted rescan re-reads the entire workspace corpus |
| MED | `registry.rs:130` | `resolve()` prefix match case-sensitive — Windows cwd casing silently breaks project resolution (frontend equivalent case-folds) |
| LOW | `registry.rs:149` | Lowercased project ids collide case-differing roots on Linux — second project silently never indexed |
| MED | `BrainPane.tsx:246` | Optimistically-removed proposal clobbered back by the bounded 4-shot poll before the worker applies the resolve |
| MED (unverified) | `resume/tier2.rs:27` | Latent command injection: session id spliced into `--resume {id}` unvalidated. Unreachable today (always `None`) but the planned capture source is agent-bus content — add the allowlist NOW |

### 4. Paid-path economics (Librarian) — real money

- **HIGH (verified in depth)** `worker.rs:653` — unbounded paid retry loop. Failed
  reflects re-arm `dirty` with no backoff/counter; `digest_hash` stored only on success,
  so `InvalidOutput` re-sends the byte-identical digest as a fresh PAID call every 5 min;
  client errors charge the full estimate even when the provider billed nothing. A
  persistently non-conforming model burns ~$2/day (debug-log only) until the ceiling pins
  at OverBudget. Capped by the ceiling, but converts the whole budget to failed spend and
  bricks the feature. Fix: classify persistent vs transient, failure cap/backoff, store
  digest hash on `InvalidOutput`.
- MED `worker.rs:639` — autonomous reflect passes `now_date=None` → `stale_revalidate`
  structurally invisible to the event-driven Librarian (and pure time passage generates
  no watcher event → no dirty → no round). ADR-008's goal partially defeated; only
  manual clicks surface overdue notes.
- MED (unverified) `reflect/llm_openai.rs:96` — sends `max_tokens`; newer OpenAI models
  400 it → combines with charge-on-uncertainty into phantom spend. Use
  `max_completion_tokens` (or both).
- MED (unverified) `curate/mod.rs:173` — escalate band pays the LLM BEFORE the
  reject-signature / pending-dedup check (ACT band does it right) → rejected
  contradictions re-charge forever (co-anchored pairs are permanent). Also no verdict
  cache for byte-identical contradiction digests.
- LOW `reflect/mod.rs:309` — manual reflect discards the digest hash → next auto round
  duplicates the spend. LOW (unverified) `sqlite.rs:326` — digest findings come from an
  ORDER BY-less SELECT → delta gate depends on SQLite scan-order stability.
- LOW (unverified) `reflect/digest.rs:63` — repo-controlled note titles/anchors flow
  unsanitized into the LLM prompt (mitigations real: schema-validated, capped, human-gated).
- MED `secrets.rs:37` — `AccountKey=` prefix is unreachable dead code (`=` not a candidate
  char; `accountkey` not in SECRET_KEY_WORDS) — Azure storage keys only partially redacted.

### 5. Index quality & scale ceilings

- **HIGH** `ast/mod.rs:71` — unanchored TS def queries capture function-local variables
  and object-literal methods as definitions — graph noise poisoning impact analysis +
  Brain Map.
- MED `sqlite.rs:585` — `rebuild_edges` rewrites the ENTIRE project edge table on every
  watcher delta (O(project) per save). MED `sqlite.rs:939` — temporal boost full-scans
  the files table on every search (all projects when project=None). These two are where
  the store fails its own 10k-file ambition.
- MED `freshness/watch.rs:121` — nested/overlapping project roots: full walks index a
  file under BOTH, watcher updates only the innermost → outer copy stale, duplicate hits.
- MED `worker.rs:562` — vanished-path handling is O(K×N) on bulk deletes (reloads ALL
  indexed paths per vanished path).
- LOW `worker.rs:536` — Windows case-only rename leaves duplicate rows until next full
  walk. LOW `worker.rs:548` — dir-event branch walks THROUGH symlinks the full walk won't.
- LOW `sqlite.rs:641` — import resolution misses NodeNext `./x.js`→`x.ts` + `/index.jsx`.
  LOW `sqlite.rs:701` — `code_impact_readonly` reads across multiple implicit read txns
  (can tear across a concurrent rebuild). LOW `graph.rs:51` — `anchor_path` drops
  root-level file anchors (no Brain Map edge). LOW `sqlite.rs:201` — `.ok()` on
  index_file's pre-read conflates DB error with "not indexed" (orphans old FTS doc).
  LOW `sqlite.rs:515` — `remove_file` doesn't delete `code_edges` (relies on caller
  rebuild whose errors are discarded). LOW `sqlite.rs:806` — `run_leg` tie-break omits
  `project_id` (cross-project ties nondeterministic). LOW `sqlite.rs:869` — query tokens
  not deduped before MATCH (double-count in bm25 vs Conductr reference).
- **HNSW (feature-gated P5, NOT live — fix before ever enabling `semantic`):**
  HIGH `hnsw_store.rs:66` — `upsert` appends instead of replacing → stale embeddings win
  after edits; no delete path on the trait. Disputed: dim-mismatch assert panics
  (`:85`), zero-norm cosine scores 1.0 (`:92`).

### 6. Memory/doctor (ALL unverified — strong leads, check while fixing)

- MED `memory/doctor.rs:147` — applied/rejected proposals tombstone recurring findings
  FOREVER (same signature + `ON CONFLICT DO NOTHING`) — a note that goes stale again
  never resurfaces. Add occurrence context (date) to signatures.
- MED `worker.rs:576` — dir-level `.koden-memory` delete/rename misses the
  `/.koden-memory/` marker (single dir event, no trailing slash) → ghost notes feed
  doctor/curate/paid escalation indefinitely.
- LOW `memory/doctor.rs:91` — lexical date compare: `2026-6-5`-style dates silently
  never trigger `stale_revalidate`. LOW `doctor.rs:49` — URL/Windows-drive anchors
  colon-split-mangled into perpetual false `broken_anchor` findings.
- LOW `memory/mod.rs:191` — duplicate frontmatter ids across files: last-writer-wins in
  platform-dependent read_dir order. LOW `curate/detect.rs:93` — doctor + curate
  double-propose the same `stale_revalidate` (different signatures, rejecting one
  doesn't suppress the other). LOW `curate/mod.rs:157` — no escalate cap → 60 stale
  notes = 60 sequential blocking LLM calls starving the single worker thread.

### Refuted / disputed (for the record)

- **"Monthly ceiling never resets" — REFUTED, correctly.** Cumulative cap is deliberate,
  tested design; shipped UI says "cumulative cap, not a monthly reset"
  (`BrainPane.tsx:665`, `OnboardingWizard.tsx:845` — spot-checked by hand). Residue:
  4 stale internal comments say "monthly" (`commands.rs:302`, `budget.rs:34`,
  `store/schema.rs:127`, `bindings.ts:138`) — one-word doc cleanup.
- "No HNSW persistence" — refuted (default-OFF P5 reference code; trait doesn't promise it yet).
- Disputed (split votes, low): worker-thread panic supervision — actionable kernel:
  a dev-build panic leaves status stuck at **Ready**; a drop-guard setting `Degraded`
  fixes observability.

## Provenance

Workflow run `wf_00c58448-52b`, output `tasks\wahbwtews.output` + `journal.jsonl` in the
session transcript dir (session `50ee4152`, 2026-07-02) — full evidence chain per finding
lives there, but this ADR is self-contained. All `worker.rs`/`sqlite.rs` etc. paths are
relative to `src-tauri/src/modules/brain/`. Line numbers valid at `f989238`.
