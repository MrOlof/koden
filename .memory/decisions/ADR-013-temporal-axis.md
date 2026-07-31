# ADR-013 — Temporal axis (commit-anchored graph history)

Status: **v1 shipped / stored upgrade Rejected-for-now** — decided 2026-07-11
by Claude as Kosta's pre-approved proxy. The recommended first step (git-backed
`hotspots` + `changed_between`, commit `6f7207f`) shipped 2026-07-07 and covers
every query anyone has actually asked; the stored bitemporal machinery adds
capture complexity + unbounded storage for "graph state at a past commit"
queries with no demand. **Revisit triggers:** (1) a real need to reconstruct
graph (not file) state at a historical commit; (2) churn/recency signals
wanted in ranking beyond the existing temporal boost; (3) ADR-012 gets built
(symbol history should then be designed WITH it, not migrated later).

Originally Proposed 2026-07-07 as the second of the two NorrGit-parity
capabilities that are architecture changes, not steals.

## Context

The brain answers "what is true now" (live index, blake3 freshness, gist) but
not "what was true when / how did it change": no blame-shaped queries, no
hotspot ranking by churn, no as-of reconstruction. Recency exists only as the
search-time temporal boost (mtime-based).

NorrGit's stage 14 is the strongest design in its README and worth copying
closely if we do this at all:

- **Bitemporal intervals**: valid-time = the commit a fact was true at
  (committer SHA + timestamp, never wall-clock — re-analyzing the same HEAD is
  a no-op), transaction-time = the analyze run.
- **Guard-first capture**: skip recording (loudly, with a marker) when there is
  no HEAD, the tree is dirty, a per-file error degraded the graph, or HEAD moved
  mid-run — anchoring an unverified graph to a commit is worse than a visible
  gap. This matches our ADR-010 "errors are Unknown, never Absent" philosophy
  exactly.
- **Append-only, no FK to live tables** — history survives deletes; the live
  graph and its determinism gates are untouched.
- Surfaced read-only (blame / changed_between / hotspots / as_of) + optional
  recency decay on search; true per-commit backfill by replaying analyze across
  history in a throwaway worktree.

## Decision (proposed)

Adopt a **scoped-down v1**: append-only `file_versions` (file-level deltas per
commit anchor: appeared / content-hash-changed / disappeared, plus def-count as
a cheap churn signal), captured guard-first at the end of a clean full walk when
the project root is a clean git checkout. Expose `hotspots` and
`changed_between` first (the two with immediate agent value — "what churns" and
"what changed since the gist was built"); blame/as_of and backfill only if v1
earns its keep. Symbol-level history waits for ADR-012 — building it
file-granular first means we never have to migrate history across granularity.

Non-goals for v1: wall-clock anchoring (violates the no-op-on-same-HEAD rule),
dirty-tree capture, history UI (the Brain Map commit scrubber is a later toy).

## Cost / risk

- Storage growth is append-only and unbounded — needs a retention ceiling
  ('// ponytail:' documented) from day one.
- Watcher-driven indexing means most walks happen on dirty trees mid-edit; the
  guard will skip capture often. Capture realistically fires on
  save-quiesce-at-clean-HEAD moments — acceptable (sparse honest history beats
  dense wrong history) but must be documented so sparse data isn't read as a
  bug.
- The DB-only-canonical caveat applies: history lost on unattachable corruption
  (same class as proposals/budget ledger — the sidecar-journal idea from the
  2026-07-06 probe would cover it).

## Alternatives

- **Just shell out to git** at query time (`git log --numstat` for hotspots,
  `git diff --name-only A..B` for changed_between): zero storage, zero capture
  machinery, answers 80% of v1. **This is the recommended first step** — ship
  git-backed `hotspots`/`changed_between` (cheap, this week), and only build
  stored bitemporal capture when a query genuinely needs graph state (not file
  state) at a past commit. The stored design above is then the upgrade path.
- Do nothing: agents ask git themselves. Works in Koden's terminal, but the
  brain can't fold churn into ranking or gist.
