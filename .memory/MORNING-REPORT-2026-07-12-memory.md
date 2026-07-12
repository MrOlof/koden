# Morning report — the memory pipeline (overnight 2026-07-11 → 12)

**TLDR: the "charge people for it" memory loop is BUILT and PROVEN LIVE.
A real Claude session received Koden's memory mid-session with zero tools and
quoted it verbatim; its own activity flowed back into the Brain and appeared
in the NEXT injection — hands-off, crash-safe, cache-friendly, redacted.
Branch at `909241d` + this report. Nothing pushed.**

## What shipped tonight

**ADR-019 — real-time memory injection (`0aa964e`).** Every project gets a
derived artifact (`.koden-memory/.koden-gist.json`): the cache-stable gist,
pre-escaped by Rust as complete hook-output JSON. A second Koden-owned
`UserPromptSubmit` hook (installed globally, migration-safe) walks up from the
session's cwd and cats it — so **every prompt in every Claude session carries
the project's current memory**, plain Windows Terminal included (deliberately
ungated by KODEN_TERMINAL). Byte-compare-gated writes preserve the agent's
prompt cache; the artifact is excluded from the index, the note scan, and the
reflect digest (no self-feeding). Verifier executed the real hook shell
command against hostile artifacts (JSON-breakout, shell metacharacters,
nested-worktree traps): all contained. Toggle in the Brain tab, default on.

**ADR-020 — session activity layer + hands-off freshness (`566f322` +
fixups).** A canonical `brain_activity` trail per project: your prompts
(secrets-redacted AT INGEST, trivial-turn filtered), files touched, session
start/end — written incrementally so a crashed session still leaves its
trail. Session exit now triggers: targeted index reconcile + gist refresh —
**the manual-rescan era is over**. The trail surfaces day-bucketed in the
injected gist ("Recent activity") and rides the Librarian's reflect message
via the send-only split, so she distills it into durable notes WITHOUT the
delta gate burning budget per turn (hash provably unchanged by 50 ingested
turns — tested). Retention: capped + TTL-pruned per project; project removal
wipes the trail (verifier-caught major: it originally survived removal and
could resurrect — fixed + regression-tested). Second catch: the trail was
recording the brain's own artifact refreshes as "activity" — filtered.

**Notifications (in ADR-020).** One coalesced event per Librarian round
(never per-note) → sonner toast ("Librarian · <project>: N memory updates" +
View → Memory changes feed) + notification-bell entry, both behind
`memoryNotifications` (default ON, toggle in Librarian tab) — plus an
always-on ambient `● brain` status-bar segment (muted idle / pulse warming /
flash on activity, hover = last-activity summary).

## The live gauntlet (evidence in `.memory/svart-verification/`)

1. Boot → artifacts emitted **fleet-wide** (every registered project), valid
   single-key JSON, real gist content (headers, known-unknowns, files).
2. **`gauntlet-injection.png`** — real `claude -p` in norrsken-saltvatten,
   asked to describe its context WITHOUT tools: answered *"Yes. First line:
   `# Koden Brain · norrsken-saltvatten · 79 files · fp:76c830f3741f`"* —
   byte-identical to the artifact on disk. Injection proven with a live model.
3. Second session → trail rows `start/turn/end` in `brain_activity`
   (prompt text stored REDACTED), exit fired the targeted rescan
   (log-confirmed: only that project re-indexed), gist refreshed, and the
   next injection carries `## Recent activity — 2026-07-12 · claude`.
4. Ambient `● brain` segment live in the status bar.

## Honest notes

- One live-test flake was MY harness, not the product: a `cd X; claude` composite
  command attributes the turn to the pre-cd cwd (OSC-7 timing) — correct
  behavior; and one stale-instance run (rapid relaunches) showed zero rows
  before a clean boot showed the pipeline working. Documented so you don't
  re-chase it.
- Not live-fired tonight: the apply toast end-to-end (needs a real budgeted
  Librarian apply; the coalescing seam is unit-tested) — flip your cap on and
  the first real round will demo it. The status segment "reflecting" pulse is
  completion-based (no dispatch seam) — ADR follow-up.
- The artifacts appear as untracked `.koden-memory/` files in your real repos —
  gitignore or commit per repo taste (ADR-019 documents the posture).
- Your OpenAI key is still in the keyring (the brain session flagged rotation).

## Gates (final state)

cargo lib **446**, brain_apply 19 (incl. stacking-revert + gist-refresh),
brain_sandbox 51, journal/offload/rounds/pin/secret all green, tsc clean,
vitest **404**, only the two documented env failures. ADRs 018/019/020
committed; ADR-017 annotated.

**The pitch is now real: open a terminal anywhere in your workspace, run
claude, and it already knows the project — and what happened last session —
with zero setup, zero commands, zero spend when idle.**
