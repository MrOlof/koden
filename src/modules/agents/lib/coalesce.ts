import type { NotificationKind } from "./types";

export type CalmEvent = {
  kind: NotificationKind;
  agent: string;
  title: string;
  body: string;
};

export const CALM_WINDOW_MS = 4000;

/**
 * Batches calm (non-urgent) notifications so four agents finishing within a
 * few seconds raise ONE OS notification instead of four. The window opens on
 * the first event; everything arriving before it closes joins the batch. A
 * lone event passes through with its original title/body.
 */
export function createCoalescer(
  flush: (title: string, body: string) => void,
  windowMs: number = CALM_WINDOW_MS,
): { add: (e: CalmEvent) => void } {
  let pending: CalmEvent[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;

  return {
    add(e: CalmEvent): void {
      pending = [...pending, e];
      if (timer !== null) return;
      timer = setTimeout(() => {
        const batch = pending;
        pending = [];
        timer = null;
        if (batch.length === 1) {
          flush(batch[0].title, batch[0].body);
          return;
        }
        const labels = [...new Set(batch.map((b) => b.body || b.agent))];
        const title = batch.every((b) => b.kind === "finished")
          ? `${batch.length} agents finished`
          : `${batch.length} agent updates`;
        flush(title, labels.join(", "));
      }, windowMs);
    },
  };
}
