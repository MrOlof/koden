/**
 * Optimistic-update guards for the proposal inbox (ADR-010 cluster 7) and the
 * ADR-018 "Memory changes" list.
 *
 * Resolving a proposal (or reverting a change) enqueues onto the brain worker
 * and updates the card optimistically; the bounded post-action poll re-reads
 * the backend BEFORE the worker lands the write, so an unguarded poll clobbers
 * the optimistic state back. [reconcileProposals] hides proposals whose resolve
 * is still in flight; [reconcileChanges] holds a reverting row at `reverted`.
 * Both forget keys once the worker has landed them.
 */

/** Composite key — proposal signatures are only unique per project. */
export function proposalKey(project: string, signature: string): string {
  return `${project} ${signature}`;
}

/**
 * Reconcile a fetched proposal list against the in-flight resolution set:
 * - forget pending keys the backend no longer returns (the worker applied them);
 * - hide proposals whose resolve is still in flight.
 * Mutates `pending` (a ref-held Set) and returns the visible list.
 *
 * `scope` is the project filter the fetch ran with (`null` = all projects).
 * Absence only means "applied" when the fetch could have returned the key: a
 * project-scoped fetch says nothing about OTHER projects' pending keys, so
 * forgetting them there would let a stale poll tick clobber the removal back
 * (project ids are fixed-length hex, so the `"${scope} "` prefix is unambiguous).
 */
export function reconcileProposals<
  T extends { project: string; signature: string },
>(fetched: T[], pending: Set<string>, scope: string | null): T[] {
  if (pending.size === 0) return fetched;
  const present = new Set(
    fetched.map((p) => proposalKey(p.project, p.signature)),
  );
  for (const key of pending) {
    if (present.has(key)) continue;
    if (scope === null || key.startsWith(`${scope} `)) pending.delete(key);
  }
  return fetched.filter(
    (p) => !pending.has(proposalKey(p.project, p.signature)),
  );
}

/**
 * The revert twin of [reconcileProposals] for the ADR-018 "Memory changes" list.
 * A reverted row stays IN the list (status flips, the card shows "Reverted"), so
 * instead of hiding rows this OVERRIDES the fetched status to `reverted` while a
 * revert is still in flight on the worker — a stale poll tick can't flash the
 * Revert button back (double-clicks are idempotent server-side, but the flicker
 * misleads). Keys are forgotten once the backend reports the row reverted, or —
 * scope-aware, like reconcileProposals — once a fetch that could have returned
 * the key no longer does (the row vanished, e.g. its project was removed).
 */
export function reconcileChanges<
  T extends { project: string; signature: string; status: string },
>(fetched: T[], reverting: Set<string>, scope: string | null): T[] {
  if (reverting.size === 0) return fetched;
  const byKey = new Map(
    fetched.map((c) => [proposalKey(c.project, c.signature), c] as const),
  );
  for (const key of reverting) {
    const row = byKey.get(key);
    if (row ? row.status === "reverted" : scope === null || key.startsWith(`${scope} `)) {
      reverting.delete(key);
    }
  }
  return fetched.map((c) =>
    reverting.has(proposalKey(c.project, c.signature)) && c.status === "applied"
      ? { ...c, status: "reverted" }
      : c,
  );
}
