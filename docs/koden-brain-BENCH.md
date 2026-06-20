# Koden Brain — Benchmark Report

Measured results pasted from a **real run** of the deterministic, offline relevance
benchmark (CONCEPT §12.2, anti-gaming rules BUILD-PROMPT §13.12).

Reproduce: `cd src-tauri && cargo test --locked --test brain_bench -- --nocapture`
Source + labels: `src-tauri/tests/brain_bench.rs` (hermetic — corpus + ground-truth
defined in-test; no network, no `~/.koden`).

## Methodology
- A small realistic mixed TS/Rust corpus (10 files) is indexed through the **real**
  pipeline (`brain::worker::index_dir`: walk → blake3 → secrets redact → FTS5).
- Three query bands with **labeled ground-truth**:
  - **positives** — lexically-aligned; the labeled target must appear in top-5.
  - **negative control** — the right answer is "not here"; must return nothing
    (HARD gate).
  - **semantic-intent** — query words don't lexically overlap the code; P0
    lexical search is *expected to miss* (semantic recall is P5). Reported, not
    asserted — the honest gap.

## Results (2026-06-20, P0 lexical, `feat/koden-brain`)

```
corpus: 10 files, indexed 10 (pruned 0)
coverage: 9 positive + 3 negative-control + 2 semantic-intent queries

[positives] recall@5 = 9/9 = 1.00 (measured)
    worst cases: none

[negative control] leaks = 0/3

[semantic-intent] (P0 lexical expected to miss; semantic is P5) hit 0/2
    "money formatting" -> ideal src/utils/currency.ts: miss (lexical gap)
    "user authentication flow" -> ideal src/auth/login.ts: miss (lexical gap)

known weakness: pure semantic/synonym queries are out of scope for P0
```

## Honest reading (not a vanity 1.0)
The 1.00 figure is **only** over lexically-aligned positives — by construction
those share tokens with the code, so a correct BM25+RRF index should find them.
The discriminating signal is the other two bands: the negative control is clean
(0/3 — no false matches on out-of-corpus topics), and the **semantic-intent band
is 0/2**, which honestly quantifies P0's ceiling — synonym/conceptual queries
("money" ≠ "currency", "authentication" ≠ "login") are misses until Tier-1
semantic embeddings (P5) land. Gates enforced by the test: negative-control leaks
must be 0; positive recall@5 must be ≥ 0.80.

## Gaps / next
- Grow the labeled corpus toward the §12.2 fixture archetypes (renamed-symbols,
  broken-imports, moved-files) with per-fixture `labels.json` once the fixture
  generator lands.
- Add latency measurement (p50/p95 of `brain_search`) on a larger seeded corpus
  against the 150 ms gate (criterion bench) — deferred with the perf-budget work.
- Re-run after P2 (AST symbol column populated) and P5 (semantic) to show the
  semantic-intent band closing.
