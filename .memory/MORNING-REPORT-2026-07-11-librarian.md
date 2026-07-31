# Morning report — Librarian gauntlet night (2026-07-10 → 11)

> Executed under proxy pre-approval ("go ham on the simulation testing, once
> verified, fixed, then you do the ADRs"). 7 commits `10ba031..83f4ce7`, all
> verified before commit, nothing pushed. Total real LLM spend: **~$0.08**.
> (A separate Koden Svart design session ran concurrently in its own worktree —
> see its own report; no overlap with this tree.)

## The gauntlet: 3 legs, 21 scenarios, all reliable-verified

**L1 lifecycle accuracy** (8 real paid rounds over an evolving corpus, $0.019):
9/9 scenarios — seed→accept→contradict→stale→reject→scale-to-40-notes→steady-state.
Per-round: 1.3–4.4 s wall, $0.0005–0.003, tokens in 357→1720 / out 88–569.

**L2 concurrent load** (5 simulated terminals, 1502 files, $0.004): 5/5 —
baseline search p50 5.4 ms; during a 15 s round p50 6.6 ms (never blocks);
event loss 0/407 under 20 files/s burst; cadence obeys min-gap under chaos
(12 rounds vs 30 boundaries); zero cross-project contamination.

**L3 multi-session kill matrix** (7 process sessions, one store, $0.053):
7/7 — budget/reject-sig/proposal continuity across restarts; kills mid-fake-call,
mid-REAL-network-call, mid-index; 5 rapid kill cycles; ledger arithmetic exact
after every session; 7 orphan sweeps charged at estimate.

**Judge verdict on the real proposals:** recall **1.00** (10/10 planted facts),
precision **1.00** (22/22 grounded, zero fabrications, verified per-item),
kind classification correct at first surfacing. One real defect: **2.4×
proposal redundancy** — title-signature dedup caught ~19% of re-wordings.

## Defects found → fixed → verified → committed

| Commit | What |
|---|---|
| `10ba031` | **LIB-SPEND-01**: digest pin now persisted (canonical table) — a restart+edit no longer wastes one paid round per project |
| `3c3ffe5` | **ADR-016 implemented**: LLM call off the worker thread. Staleness during a round: 20.1 s → **0.79 s**. Two interleaving races (mid-flight manual pin clobber; RemoveProject mid-flight) caught by adversarial verify and fixed; helper-panic guard added |
| `83f4ce7` | **Proposal-stream dedup** (the judge's finding): Jaccard≥0.5 near-dupe gate + "Already proposed" digest advisory + don't-restate prompt + findings name their target note. Delta-gate hash proven not to self-re-fire. Live-confirmed: restatements silent, a planted contradiction still fires conflict |

## Pre-approved builds also landed

- `8546fd6` — **sidecar journal**: proposals/reject-sigs/budget/pins survive
  header-destroying corruption; re-spend refused OverBudget after recovery
  (closes the last durability gap; ledger-reserve exclusion prevents double-charge)
- `247666a` — **UI walkers** (tree/grep/search) honor .gitignore/.kodenignore in
  non-git roots — file explorer and brain now agree (note: one test's semantics
  deliberately flipped, renamed accordingly)
- `7ddde42`/`f6dc786`/`8b9b7c9` — brain_cli harness (set-key, reflect-live,
  session subcommands) committed

## ADR verdicts (made as your proxy)

- **ADR-012 symbol granularity → Deferred, demand-driven** (3 explicit re-open triggers written in)
- **ADR-013 stored temporal → rejected-for-now** (git-backed v1 covers demand; revisit triggers listed)
- **ADR-016 librarian off worker thread → Accepted AND implemented** (numbers above)
- **ADR-015 embeddings prototype → Proposed, yours** — needs a model-runtime dependency; go/no-go criteria pre-registered (hard-concept recall ≥0.7, P@10 ≥0.90 held, first-index ≤2×)

## Honest notes

- The dedup prompt change reduces proposal VOLUME by design: the model no longer
  restates knowledge that already exists as a note. On a quiet corpus, zero
  proposals is now the correct steady state.
- A residual dedup ceiling: a fully-synonymized rewrite with zero shared tokens
  slips the lexical gate; the prompt advisory + human inbox backstop it, and the
  embeddings decision (ADR-015) is the structural answer.
- Workflow infrastructure had a rough night (4 structured-output deaths); every
  loss was recovered from journals with zero work lost. Direct agents were solid.
- The OpenAI key is STILL IN THE KEYRING (per your "don't worry" — but it's in
  the chat transcript twice; rotate when you're up).
- Suppressed-dupe count is log-only (a sim-file compile constraint); can move
  onto ReflectOutcome whenever you want it surfaced in the UI.

Final state: `feat/koden-brain` at `83f4ce7`, 411/412 lib + 12/12 integration
suites green, tree clean. GUI validation → merge remains the last gate, and the
Svart branch will want rebasing onto this. Delete this file once read.
