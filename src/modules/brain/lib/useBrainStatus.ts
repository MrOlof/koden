import { useEffect, useState } from "react";
import { brainIndexStatus, type BrainStatus } from "./bindings";

const POLL_MS = 2000;

/**
 * Polls the brain worker's index status for live UI (the tab icon's status dot +
 * breathe). Returns `null` until the first reply / on error, so callers render a
 * neutral "connecting" state rather than flapping. Fail-open: a failed poll is
 * swallowed and retried on the next tick.
 */
export function useBrainStatus(): BrainStatus | null {
  const [status, setStatus] = useState<BrainStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const tick = () => {
      brainIndexStatus()
        .then((r) => {
          if (alive) setStatus(r.status);
        })
        .catch(() => {});
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);
  return status;
}
