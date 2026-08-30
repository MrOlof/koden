// Pure bind-time replay ordering for a renderer slot. Kept free of xterm so the
// ordering invariant is unit-testable with a fake terminal.

/**
 * Written right after a restored scrollback. CUD 999 clamps the cursor to the
 * bottom row so the separator lands below every restored line (SerializeAddon
 * parks the cursor where the previous launch left it, which may be mid-screen
 * for a TUI); the trailing newline hands the fresh shell a clean line.
 */
export const RESTORE_SEPARATOR =
  "\x1b[0m\x1b[999B\r\n\x1b[2m[restored]\x1b[0m\r\n";

export type ReplayTerminal = { write(data: string | Uint8Array): void };

export type FirstBindReplay = {
  /** Scrollback persisted by a previous launch; consumed on the first bind only. */
  restored: string | null;
  /** Buffer serialized when this leaf's slot was stolen (same launch). */
  snapshot: string | null;
  altScreen: boolean;
  drainRing: (write: (bytes: Uint8Array) => void) => void;
};

/**
 * Replay order on a (non-fast) bind: restored scrollback + separator, then the
 * same-launch snapshot, then the dormant ring. PTY bytes that arrived before
 * the leaf ever had a grid sit in the ring, so they always land AFTER the
 * restored text. In alt-screen the ring is discarded: incremental
 * cursor-positioned TUI repaints cannot be replayed over a snapshot (the
 * caller kicks SIGWINCH so the TUI redraws from scratch instead).
 */
export function replayFirstBind(
  term: ReplayTerminal,
  r: FirstBindReplay,
): void {
  if (r.restored) {
    try {
      term.write(r.restored + RESTORE_SEPARATOR);
    } catch (e) {
      console.warn("[koden] restored scrollback replay failed:", e);
    }
  }
  if (r.snapshot) {
    try {
      term.write(r.snapshot);
    } catch (e) {
      console.warn("[koden] snapshot replay failed:", e);
    }
  }
  if (r.altScreen) {
    r.drainRing(() => {});
  } else {
    r.drainRing((bytes) => term.write(bytes));
  }
}
