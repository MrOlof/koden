# Koden Brain — Benchmark Report

Measured results pasted from a **real run** of the deterministic, offline relevance
benchmark (CONCEPT §12.2, anti-gaming rules BUILD-PROMPT §13.12).

Reproduce: `cd src-tauri && cargo test --locked --test brain_bench -- --nocapture`
Source + labels: `src-tauri/tests/brain_bench.rs` (hermetic — corpus + ground-truth
defined in-test; no network, no `~/.koden`).

## Methodology
- A small realistic mixed TS/Rust corpus (13 files, incl. 3 confuser pairs) is
  indexed through the **real** pipeline (`brain::worker::index_dir`: walk → blake3 →
  secrets redact → FTS5).
- Graded metrics over the labeled positives: **recall@5** (HARD floor ≥ 0.80),
  **MRR**, and **precision@1** (the rank-1 signal that drives the gist's top line).
- Query bands with **labeled ground-truth**:
  - **positives** — lexically-aligned; the labeled target must appear in top-5.
  - **confusers** — the query term is ONLY in the target's path and ONLY in a
    distractor's body, so identity-vs-content weighting decides rank-1 (this is what
    lets the corpus DISCRIMINATE weight settings instead of scoring a flat 1.0).
  - **negative control** — the right answer is "not here"; must return nothing
    (HARD gate).
  - **semantic-intent** — query words don't lexically overlap the code; P0
    lexical search is *expected to miss* (semantic recall is P5). Reported, not
    asserted — the honest gap.

## Results (2026-06-21, P0 lexical + V2.2 calibration, `feat/koden-brain`)

```
corpus: 13 files, indexed 13 (pruned 0)   (3 confuser pairs added)
coverage: 12 positive (incl. confusers) + 3 negative-control + 2 semantic-intent

[positives] recall@5 = 12/12 = 1.00 · MRR = 1.000 · precision@1 = 1.00 (measured)
    worst cases: none

[negative control] leaks = 0/3

[semantic-intent] (P0 lexical expected to miss; semantic is P5) hit 0/2
    "money formatting" -> ideal src/utils/currency.ts: miss (lexical gap)
    "user authentication flow" -> ideal src/auth/login.ts: miss (lexical gap)
```

## Weight calibration (V2.2) — measured, not guessed

The BM25/RRF weights were `// Provisional defaults` with no test proving any value
beat a neighbor. V2.2 added **confuser fixtures** (the query term lives ONLY in the
target's path and ONLY in a distractor's body, so identity-vs-content weighting
alone decides rank-1) and a deterministic offline **calibration sweep**
(`weight_sweep_reports_mrr_subject_to_zero_leaks`, `#[ignore]`d):

```
WEIGHT CALIBRATION SWEEP (max MRR s.t. negative-control leaks == 0)
  rrf_identity >= rrf_content (1.5, 3.0):  MRR 1.000   leaks 0   <- production band
  rrf_identity <= rrf_content (0.25,0.5,1): MRR 0.875  leaks 0   (confusers steal rank-1)
  best leak-free: MRR 1.000 ; production default (rrf_identity 1.5) -> MRR 1.000
```

So the production `rrf_identity = 1.5 > rrf_content = 1.0` is in the MRR-optimal,
leak-free band — now **measured**, and the `provisional` comment is downgraded to
`measured` in `sqlite.rs`. RRF fuses by RANK, so the per-column bm25 magnitudes
(path 3×) only order within a leg; the sweep confirms `path_bm25 ∈ {2,3,4}` doesn't
move MRR — the leg RRF weights are the load-bearing cross-leg knob. A CI-running
guard (`production_weights_beat_content_dominant`) asserts the corpus genuinely
discriminates (default MRR > content-dominant MRR), so the 1.000 is never a vanity
number. Re-run the sweep and record before/after on any future weight change (§13.12).

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
