export type TerminalKeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "metaKey" | "shiftKey" | "key" | "code"
>;

export type PlatformOpts = { isMac: boolean };

export type TerminalClipboardAction = "copy" | "cut" | "paste";

/**
 * Desktop-style clipboard shortcuts for the terminal grid.
 *
 * The product default favours expected desktop UX over classic terminal
 * bindings, while never clobbering the control codes a shell needs:
 *   - Ctrl+C  copies when there is a selection, otherwise falls through so
 *     xterm sends SIGINT (\x03).
 *   - Ctrl+X  cuts when there is a selection (terminal text is read-only, so
 *     this copies and drops the selection), otherwise falls through (\x18).
 *   - Ctrl+V  always pastes.
 *   - Ctrl+Shift+C / Ctrl+Shift+V stay as explicit copy / paste.
 * macOS keeps its native Cmd-based clipboard; Ctrl there is left untouched so
 * Ctrl+C remains SIGINT.
 */
export function terminalClipboardAction(
  event: TerminalKeyEvent,
  opts: PlatformOpts & { hasSelection: boolean },
): TerminalClipboardAction | null {
  if (opts.isMac) return null;
  if (!event.ctrlKey || event.altKey || event.metaKey) return null;
  const isC = event.code === "KeyC" || event.key === "c" || event.key === "C";
  const isV = event.code === "KeyV" || event.key === "v" || event.key === "V";
  const isX = event.code === "KeyX" || event.key === "x" || event.key === "X";
  if (event.shiftKey) {
    if (isC) return "copy";
    if (isV) return "paste";
    return null;
  }
  if (isV) return "paste";
  if (isC) return opts.hasSelection ? "copy" : null;
  if (isX) return opts.hasSelection ? "cut" : null;
  return null;
}

export function isTerminalShiftEnter(event: TerminalKeyEvent): boolean {
  return (
    event.key === "Enter" &&
    event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey
  );
}

export function terminalWordNavigationSequence(event: TerminalKeyEvent): string | null {
  if (!event.altKey || event.ctrlKey || event.metaKey) return null;
  if (event.key === "ArrowLeft" || event.code === "ArrowLeft") return "\x1bb";
  if (event.key === "ArrowRight" || event.code === "ArrowRight") return "\x1bf";
  return null;
}

/** Cmd+Left/Right → readline line-start (Ctrl+A) / line-end (Ctrl+E).
 * macOS-only — Cmd doesn't exist as a navigation modifier elsewhere. */
export function terminalLineNavigationSequence(
  event: TerminalKeyEvent,
  opts: PlatformOpts,
): string | null {
  if (!opts.isMac) return null;
  if (!event.metaKey || event.altKey || event.ctrlKey) return null;
  if (event.key === "ArrowLeft" || event.code === "ArrowLeft") return "\x01";
  if (event.key === "ArrowRight" || event.code === "ArrowRight") return "\x05";
  return null;
}

/** Modifier+Backspace deletion:
 *   macOS  Cmd+Backspace    → Ctrl+U (kill-to-line-start)
 *   macOS  Option+Backspace → Ctrl+W (kill-word-backward)
 *   Other  Ctrl+Backspace   → Ctrl+W (kill-word-backward)
 */
export function terminalDeleteSequence(
  event: TerminalKeyEvent,
  opts: PlatformOpts,
): string | null {
  if (event.key !== "Backspace" && event.code !== "Backspace") return null;
  if (opts.isMac) {
    if (event.metaKey && !event.altKey && !event.ctrlKey) return "\x15";
    if (event.altKey && !event.metaKey && !event.ctrlKey) return "\x17";
    return null;
  }
  if (event.ctrlKey && !event.altKey && !event.metaKey) return "\x17";
  return null;
}
