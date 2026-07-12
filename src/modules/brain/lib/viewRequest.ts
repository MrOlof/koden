// One-shot Brain-pane view requests (ADR-020). The Brain tab is opened via
// App's openOrchestrationTab("brain") and BrainPane owns its mode locally, so a
// toast's "View" action needs a side channel to land on the Memory view: it
// records the request here, then opens the tab. A mounted pane consumes it
// live; a pane mounting later consumes the pending request on subscribe.

export type BrainViewMode = "search" | "memory";

let pending: BrainViewMode | null = null;
const subscribers = new Set<(mode: BrainViewMode) => void>();

/** Ask the (open or about-to-open) Brain pane to show `mode`. */
export function requestBrainView(mode: BrainViewMode): void {
  if (subscribers.size > 0) {
    for (const cb of subscribers) cb(mode);
    return;
  }
  pending = mode;
}

/** BrainPane-side: receive live requests + drain any pending one. */
export function subscribeBrainView(
  cb: (mode: BrainViewMode) => void,
): () => void {
  subscribers.add(cb);
  if (pending !== null) {
    const p = pending;
    pending = null;
    cb(p);
  }
  return () => {
    subscribers.delete(cb);
  };
}
