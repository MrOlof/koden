# ADR-015 — Semantic search: embeddings prototype plan

Status: **Proposed** — 2026-07-11, drafted by Claude as proxy. NOT decided:
this is the one queue item that requires a heavy new dependency, which stays
a Kosta decision even under proxy pre-approval.

## Context

The precision gate measured the lexical ceiling: hard-concept recall **0.25**
("authentication flow" cannot reach `login.ts`/`session.ts` — no shared
tokens). Everything else in the search stack is now strong (macro P@10 0.96
after the coverage re-rank), so this is the single largest remaining quality
gap, and the only one lexical work cannot close. NorrGit closes it with
Snowflake arctic-embed-xs (384-d, ~30 MB) + RRF fusion, revision-pinned,
content-hash re-encode skip, graceful BM25 fallback — a proven recipe worth
copying as-is.

## What a yes costs (the actual decision)

- **A model runtime dep in the Tauri app**: candle (pure Rust, ~heavy build) or
  ort/onnxruntime (native lib per platform). Both violate the standing
  "no new deps" bar hard enough to need this ADR.
- **~30 MB model** shipped in the bundle or downloaded on first run (first-run
  download = offline-breaks + a consent moment; bundling = installer size).
- Index-time embedding cost (mitigated by content-hash skip + the parallel
  pipeline) and RAM for the vector table.
- The HNSW side already exists default-off (ADR-006 P5 reference code +
  upsert-replace fixes from ADR-010) — the missing piece is only the encoder.

## Proposed prototype (measured go/no-go, ~1 session of work)

1. Branch-local spike behind the existing default-off flag: ort + arctic-embed-xs
   (ONNX), revision-pinned, encode-on-index with blake3-keyed skip.
2. Score against `tests/brain_precision.rs` + a new hard-concept query class.
3. **Go** if hard-concept recall ≥ 0.7 with macro P@10 ≥ 0.90 held and
   first-index ≤ 2× current (i.e. ≤ ~6 s release on this repo). **No-go**
   otherwise — delete the spike, keep the ADR as the record.

## Recommendation

Do the prototype when you're ready to accept the dependency; the measured
gap is real but bounded (agents compensate by rephrasing — plan_context's
empty-result advisories now tell them to). Not urgent enough to pre-empt
shipping the branch.
