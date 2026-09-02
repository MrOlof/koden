import { recordTombstone } from "./meta";

/** Import-light seam for stores outside the sync module (useSpaces.remove).
 * Fire-and-forget: a missed tombstone means a deleted space can reappear
 * after a merge, never data loss. */
export function recordSpaceDeleted(spaceId: string): void {
  void recordTombstone(spaceId).catch(() => {});
}

// Layout-changed signal (ADR-024 liveness): the spaces store pings here on
// every saveState/saveSpacesList so the engine can push soon after a real
// layout edit instead of waiting for the 60 s poll. Import-light on purpose —
// the store must not import the engine.
let onWsChanged: (() => void) | null = null;

export function setWsChangedListener(fn: (() => void) | null): void {
  onWsChanged = fn;
}

export function notifyWsChanged(): void {
  onWsChanged?.();
}
