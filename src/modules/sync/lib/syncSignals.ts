import { recordTombstone } from "./meta";

/** Import-light seam for stores outside the sync module (useSpaces.remove).
 * Fire-and-forget: a missed tombstone means a deleted space can reappear
 * after a merge, never data loss. */
export function recordSpaceDeleted(spaceId: string): void {
  void recordTombstone(spaceId).catch(() => {});
}
