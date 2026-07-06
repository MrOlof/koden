/**
 * Optimistic-removal guard for the proposal inbox (ADR-010 cluster 7).
 *
 * Resolving a proposal enqueues onto the brain worker and removes the card
 * optimistically; the bounded post-action poll re-reads the backend BEFORE the
 * worker applies the resolve, so an unguarded poll clobbers the removed card
 * back into the inbox. The guard hides proposals whose resolve is still in
 * flight and forgets them once the worker has applied them.
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
