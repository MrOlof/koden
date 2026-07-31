# ADR-011: Gist known-unknowns + per-claim freshness labels

Status: Accepted + IMPLEMENTED — 2026-07-06 (commit `3036916`, cluster 8 of the ADR-010
overnight execution run)

## Context

On 2026-07-05 Kosta ran an external design review (ChatGPT) over a brain feature
inventory. The reviewed list turned out to be **Conductr's** feature set, not the Koden
Brain — and most of the reviewer's "trim this" advice was already ADR-006's design
(semantic deferred, 5 memory types, no MCP, gist-as-product, propose-not-apply). Two
ideas, however, were genuinely missing from our spec and cheap to add. Both serve the
same principle the reviewer put well: **anything injected into the LLM should carry its
freshness state — memory must be invalidatable, visibly, at the injection point** (we
already invalidate in the store; we did not surface it to the agent).

## Decision

**1. Known-unknowns section.** When a retrieval leg returns nothing for the synthesized
intent, the gist says so explicitly ("No code hits for \"<intent>\"." / "No memory notes
in this project.") instead of silently omitting. Rendered immediately after the
never-trimmed freshness line as one atomic block (a tight budget can't strand a dangling
header), trimmed last-but-one. Gated on a ready, NON-EMPTY index (conn present +
`file_count > 0`, non-blank intent for the code leg) so an unready index still yields the
freshness-only gist — "thin over wrong" and the [DP-22] confidence gate are preserved:
silence and *verified absent* are different signals, and only the latter is claimed.

**2. Per-claim freshness labels on injected memory notes.** `[current]` /
`[possibly-stale]` / `[historical(superseded)]` instead of silent downranking. Labels
derive ONLY from cache-key-covered state:

- supersession edges (the note's own `superseded_by`, or another note's `supersedes`
  forward edge — note files are indexed, so their content is in the fingerprint);
- anchor touch state (`accessed_at_ms`/`accessed_count`, covered by the temporal digest
  in the key), with `possibly-stale` requiring `accessed_count >= 2` (the first stamp is
  the initial index walk, not a code change) and a touch strictly after the note's
  day-granular `created` date via pure integer civil-date math.

`revalidate_after` is **excluded** from labels (option (a)): comparing against "today"
is wall-clock-dependent — the same cache key would yield different bytes as time passes,
violating the byte-identity contract. Ponytail ceiling named in code; upgrade path =
fold a day-granularity date into the cache key (busts the prompt cache at most once/day).

`SCHEMA_VERSION` bumped 10 → 11 (no DDL change; rotates every gist cache key so one key
never mixes pre-/post-layout bytes).

## Consequences

- Byte-identity holds by construction (all new inputs read off the same pinned WAL
  snapshot; determinism asserted twice in new tests; the pre-existing
  `gist_byte_identical_on_unchanged_relaunch` and
  `gist_cache_key_stable_under_concurrent_writes` tests still pass).
- Agents now see "this note may be outdated because its anchored code changed after it
  was written" instead of a silently down-weighted note — the anti-stale layer is
  visible at the point of use.
- Time-based staleness (`revalidate_after`) still surfaces only via doctor/curate and
  manual review, not in gist labels, until the date-in-key upgrade is taken.

## Provenance

Implemented in `src-tauri/src/modules/brain/gist/mod.rs`; verified by the cluster-8
adversarial pass (2 lenses, zero confirmed defects) in workflow run `wf_285c6ead-489`
(session `34d7c8ee`, 2026-07-06). ADR-010's execution record lists the full night.
