// Adoption ledger (ADR-025). A device that creates or edits a tab on BEHALF
// of a peer: materializing a tmux window it found on the host, applying a
// rename the sync pulled: must not stamp that write as its own edit, or the
// observer outranks the author on the next merge (the 2026-09-03 incident).
// The adopter registers the clock it is adopting at; the persistence layer
// consumes it when the tab lands on disk. Leaf module: no imports, so both
// the spaces store and the engine can use it without a cycle.
import type { SerializedTab } from "@/modules/spaces/lib/serialize";

/** "I know this tab exists, nothing more": always loses to any real edit. */
export const OBSERVED_CLOCK = 0;

// Unconsumed entries (the adopted tab never persisted, or persisted under an
// identity we did not predict) must not linger and mis-stamp a later edit.
const TTL_MS = 60_000;

type Entry = {
  clock: number;
  at: number;
  /** When set, the clock applies only if the persisted tab still carries
   * the adopted value; a user edit in the same debounce window stamps now. */
  matches?: (tab: SerializedTab) => boolean;
};

const pending = new Map<string, Entry>();

function prune(now: number): void {
  for (const [id, e] of pending) if (now - e.at > TTL_MS) pending.delete(id);
}

export function expectClock(
  identity: string,
  clock: number,
  matches?: (tab: SerializedTab) => boolean,
  now: number = Date.now(),
): void {
  prune(now);
  pending.set(identity, { clock, at: now, ...(matches && { matches }) });
}

/** Consume the registered clock for a tab being persisted, if any. */
export function takeExpectedClock(
  identity: string,
  tab: SerializedTab,
  now: number = Date.now(),
): number | undefined {
  prune(now);
  const e = pending.get(identity);
  if (!e) return undefined;
  pending.delete(identity);
  if (e.matches && !e.matches(tab)) return undefined;
  return e.clock;
}

export function resetAdoptionLedgerForTests(): void {
  pending.clear();
}
