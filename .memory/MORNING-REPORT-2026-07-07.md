# Morning report — overnight NorrGit-parity run (2026-07-07)

> Night shift executed under your proxy approval. 10 commits on `feat/koden-brain`
> (`454fecc..6f7207f` + the `0f95577` example follow-up), every one
> fix → adversarial-verify → commit. Nothing pushed. GUI validation still held.
> Delete this file once read; the durable record is in `INDEX.md` + ADR-010's
> addendum + ADR-012/013.

## Headline numbers

| Metric | Before (2026-07-06) | After | Target |
|---|---|---|---|
| First-index, release, this repo | 29.1 s / 1592 files | **2.8 s / 1598 files** | 5–15 s ✅ |
| Search macro P@10 (new gate) | 0.53 | **0.96** | floor 0.45→raised |
| Camel-token class P@10 | 0.29 | **1.00** | worst class, fixed |
| Search recall@10 / neg-pollution | 0.96 / 0.05 | 0.96 / 0.05 | unchanged ✅ |
| Search latency | 2–4 ms | 3.5–6 ms (release, w/ coverage probes) | flat |

## What was stolen (all landed, all gates green)

1. **Impact parity** — depth-annotated, bidirectional, bounded, exclude_tests.
2. **Precision gate** — standing hand-labeled quality floor; it immediately paid
   for itself by pinpointing the camel-token weakness that item 5 fixed.
3. **detect_changes** — git diff → affected files + dependents (pre-commit tool).
4. **plan_context** — one-call planner bundle with advisory isolation.
5. **Token-coverage re-rank** — the measured P@10 jump above.
6. **Perf pair** (ours, not NorrGit's) — bounded temporal boost (bit-identical
   ranking, property-tested) + delta-proportional edge relink.
7. **Parallel first-index** — the 10× SLA win; single-writer + determinism +
   redact-before-FTS all preserved (secret_index gate proves the last one).
8. **hotspots + changed_between** — git-backed, no stored history, injection-gated.

## Your morning decisions

- **ADR-012 (Proposed)** — symbol-granular graph. The one capability where
  NorrGit remains structurally ahead. Big investment; the ADR scopes a v1.
- **ADR-013 (Proposed)** — stored bitemporal history. Its cheap first step
  shipped tonight; the ADR is about whether the stored upgrade is ever needed.
- **Framework intelligence** (route/ORM extractors) — not drafted; say the word
  and it becomes ADR-014.
- **Embeddings-on** — HNSW stays default-off; flipping it is a product call.

## Honest caveats

- The 2.8 s SLA number shares the same warm-FS-cache conditions as the 29.1 s
  baseline (both measured same-machine, post-build) — relative win is real,
  cold-boot number will be higher.
- brain_precision P@10 divides by returned count (set-pollution metric, not a
  rank metric); rank-1 quality is brain_bench's job. Hard-concept recall stays
  0.25 — the known lexical/abbreviation ceiling; that's the embeddings decision.
- Round-2's workflow crashed at its sweep step (agent died without structured
  output) — all 5 commits were already in; I re-ran the sweep inline. Round 2
  also had 3 repair rounds total (coverage gate probe-cap ×2, reorder buffer ×1)
  — all verified to zero.
- `0921062` was committed by the fix agent instead of the commit step (rule
  slip, content verified after the fact by both lenses — no action needed).

## Scorecard vs NorrGit after tonight

Parity or better: impact, detect_changes, plan_context, quality gates, temporal
v1, multi-repo (registry), visualizer, doctor, memory (ours is curated +
budgeted; theirs is ingestion), **safety (they have no secrets gate — flagged
as their gap)**, freshness (watcher vs stale-warnings), agent UX (push-gist vs
20 pull tools), first-index speed.
Still theirs: symbol granularity (ADR-012), stored bitemporal + backfill
(ADR-013), framework intelligence, shipped embeddings, 15-language polyglot
breadth.
