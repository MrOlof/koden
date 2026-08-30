import { clipboardWriteText } from "@/lib/clipboard";
import { ensureMonoFontsLoaded } from "@/lib/fonts";
import { revealInFinder } from "@/modules/explorer/lib/contextActions";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { readTerminalTokens, resolveCssColorToHex } from "@/styles/tokens";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { SearchAddon } from "@xterm/addon-search";
import type {
  IDecoration,
  ILink,
  ILinkProvider,
  IMarker,
  Terminal,
} from "@xterm/xterm";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { detectLinks } from "./linkDetect";

// The hover affordance (badge text + highlight tint) keys off what a click
// does, not the detection category. "open" reveals/opens; "copy" copies.
type LinkAct = "copy" | "open";

import {
  BlockDecorations,
  type BlockMatch,
  type VisibleBlocks,
} from "../block/lib/blockDecorations";
import type { BlockMode } from "../block/lib/modeMachine";
import { CommandMarks, type CommandMinimapData } from "./commandMarks";
import { DormantRing } from "./dormantRing";
import { addBusTurn, busTurnsForLeaf, clearBusTurns } from "./turnStore";
import {
  createShellIntegrationState,
  registerCwdHandler,
  registerPromptTracker,
} from "./osc-handlers";
import { openPty, type PtySession } from "./pty-bridge";
import "../block/block.css";
import { ensureAgentActivityListener, isAgentActivePty } from "./agentActivity";
import {
  acquireSlot,
  applyBackgroundActive,
  applyCursorBlink,
  applyFontFamily,
  applyFontSize,
  applyLetterSpacing,
  applyLineHeight,
  applyTheme as applyPoolTheme,
  applyScrollback,
  applyWebglPreference,
  configureRendererPool,
  discardRetainedSlot,
  disposeLeafSlot,
  focusSlot,
  getLiveSlotForLeaf,
  getSlotForLeaf,
  isLeafAltScreen,
  parkLeafSlot,
  poolSize,
  poolSlotStats,
  refreshLeafSlot,
  releaseSlot,
  setSlotFocused,
} from "./rendererPool";

type Callbacks = {
  onSearchReady?: (addon: SearchAddon) => void;
  onExit?: (code: number) => void;
  onCwd?: (cwd: string) => void;
};

type Session = {
  pty: PtySession | null;
  ptyOpening: boolean;
  initialCwd: string | undefined;
  lastCwd: string | null;
  pendingExit: number | null;
  shellExited: boolean;
  callbacks: Callbacks;
  visibleNow: boolean;
  focusedNow: boolean;
  disposed: boolean;
  ready: Promise<void>;
  cols: number;
  rows: number;
  container: HTMLDivElement | null;
  snapshot: string | null;
  searchQuery: string | null;
  dormantRing: DormantRing;
  hasSlot: boolean;
  blocks: boolean;
  blockMode: BlockMode;
  blockListeners: Set<() => void>;
  blockDecorations: BlockDecorations | null;
  // Command-minimap tracker, only on non-blocks leaves (blocks terminals use
  // BlockDecorations instead). Null until the slot is bound.
  commandMarks: CommandMarks | null;
  // Set by the block shell-input; called to pull focus back when the xterm
  // grid steals it at the prompt (e.g. on a click), so typing stays in the bar.
  inputFocus: (() => void) | null;
  // Per-leaf unsent shell-input text; the single workspace bar swaps it on focus change.
  inputDraft: string;
  // Live "input has text" flag from the block shell-input (gates the watermark).
  inputActive: boolean;
  // A command was submitted on this leaf; kills the watermark synchronously,
  // before the shell's OSC 133 C round-trips through the PTY.
  everSubmitted: boolean;
  // True if the slot was in alt-screen mode (TUI like vim, htop, dofek)
  // at the most recent release. Read once on the next bind to trigger a
  // SIGWINCH-driven repaint instead of replaying dormant bytes.
  altScreenAtRelease: boolean;
  // OSC 133 C..D window (or blocks running mode): a foreground process owns
  // the terminal, so the leaf must keep its live grid while hidden.
  commandRunning: boolean;
  hiddenReleaseTimer: ReturnType<typeof setTimeout> | null;
  spawnFailed: boolean;
};

const sessions = new Map<number, Session>();

// Block-overlay viewport listeners, keyed by leafId at module scope so the
// overlay (a child) can subscribe before the parent effect creates the session.
const blockViewportListeners = new Map<number, Set<() => void>>();

// Command-minimap listeners, same module-scope pattern as
// blockViewportListeners: the CommandMinimap overlay subscribes by leafId, and
// the CommandMarks tracker fans out via notifyCommands on every change.
const commandListeners = new Map<number, Set<() => void>>();

// Leaf-keyed SearchAddon registry. The addon is created per renderer slot and
// surfaced via onSearchReady; we stash it here keyed by leafId so any consumer
// holding only a leafId (e.g. the pane-header history popover's "Find in
// terminal" mode) can reach THIS leaf's addon — same shape as the command-mark
// accessors. App.tsx keeps its own activeLeaf-scoped copy for the header bar;
// this registry is the leaf-addressed path. Pruned in disposeSession.
const searchAddons = new Map<number, SearchAddon>();

export function getSearchAddonForLeaf(leafId: number): SearchAddon | null {
  return searchAddons.get(leafId) ?? null;
}

function notifyCommands(leafId: number): void {
  const set = commandListeners.get(leafId);
  if (set) for (const l of set) l();
}

const readyLeaves = new Set<number>();
const readyWaiters = new Map<
  number,
  { resolve: () => void; timer: ReturnType<typeof setTimeout> }[]
>();

function markSessionReady(leafId: number): void {
  if (readyLeaves.has(leafId)) return;
  readyLeaves.add(leafId);
  const waiters = readyWaiters.get(leafId);
  if (!waiters) return;
  readyWaiters.delete(leafId);
  for (const w of waiters) {
    clearTimeout(w.timer);
    w.resolve();
  }
}

export function whenSessionReady(
  leafId: number,
  timeoutMs = 4000,
): Promise<void> {
  if (readyLeaves.has(leafId)) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      const arr = readyWaiters.get(leafId);
      const i = arr?.findIndex((w) => w.timer === timer) ?? -1;
      if (arr && i >= 0) arr.splice(i, 1);
      resolve();
    }, timeoutMs);
    const arr = readyWaiters.get(leafId) ?? [];
    arr.push({ resolve, timer });
    readyWaiters.set(leafId, arr);
  });
}

export function writeToSession(leafId: number, data: string): boolean {
  const s = sessions.get(leafId);
  if (!s?.pty) return false;
  void s.pty.write(data);
  return true;
}

export function submitToLeaf(leafId: number, text: string): void {
  const s = sessions.get(leafId);
  if (!s?.pty) return;
  s.everSubmitted = true;
  // Bracketed paste keeps a multiline command atomic; trailing CR runs it.
  if (text.includes("\n")) s.pty.write(`\x1b[200~${text}\x1b[201~\r`);
  else s.pty.write(`${text}\r`);
}

export function interruptLeaf(leafId: number): void {
  sessions.get(leafId)?.pty?.write("\x03");
}

export function leafCwd(leafId: number): string | null {
  return sessions.get(leafId)?.lastCwd ?? null;
}

export function navigateFocusedBlocks(dir: -1 | 1): boolean {
  for (const [, s] of sessions) {
    if (!s.visibleNow || !s.focusedNow || !s.blockDecorations) continue;
    s.blockDecorations.navigateBlocks(dir);
    return true;
  }
  return false;
}

export function clearLeafBlockSelection(leafId: number): boolean {
  return sessions.get(leafId)?.blockDecorations?.clearBlockSelection() ?? false;
}

export function leafGridSelection(leafId: number): string | null {
  const sel = getSlotForLeaf(leafId)?.term.getSelection() ?? "";
  return sel.length > 0 ? sel : null;
}

// Leaf-keyed command-mark accessors for the pane-header history popover. The
// header knows a leafId but not the per-pane session object (that lives inside
// useTerminalSession), so these mirror submitToLeaf/writeToSession: look the
// session up in `sessions` by leafId. The in-hook getCommandMarks /
// subscribeCommands / scrollToCommand still serve the (now removed) overlay
// path's shape; these are the same logic reachable from outside the hook.
export function getCommandMarksForLeaf(
  leafId: number,
): CommandMinimapData | null {
  const cm = sessions.get(leafId)?.commandMarks;
  if (!cm) return null;
  return {
    marks: cm.getMarks(),
    viewport: cm.viewport(),
    altScreen: cm.isAltScreen(),
  };
}

// Record a real (text-bearing) Claude/Codex user turn for a leaf, called by
// the AgentBusBridge when the UserPromptSubmit bus hook delivers a prompt.
// Storage is session-lifetime (turnStore), so turns delivered while the pane
// is hidden/unbound are kept and pool rebinds never wipe the Inputs history.
export function addTurnForLeaf(leafId: number, text: string): void {
  if (addBusTurn(leafId, text)) notifyCommands(leafId);
}

export function subscribeCommandsForLeaf(
  leafId: number,
  cb: () => void,
): () => void {
  // No session yet (pane still binding a slot): hand back a no-op unsubscribe so
  // the caller's effect cleanup stays uniform. The CommandMarks tracker fans out
  // via notifyCommands once the session exists; this shares the same listener map.
  let set = commandListeners.get(leafId);
  if (!set) {
    set = new Set();
    commandListeners.set(leafId, set);
  }
  set.add(cb);
  return () => {
    const live = commandListeners.get(leafId);
    live?.delete(cb);
    if (live && live.size === 0) commandListeners.delete(leafId);
  };
}

export function scrollToCommandForLeaf(leafId: number, line: number): void {
  const slot = getSlotForLeaf(leafId);
  if (!slot) return;
  const term = slot.term;
  // Center the command in the viewport (same offset block navigation uses).
  const target = Math.max(0, line - Math.floor(term.rows / 2));
  term.scrollToLine(target);
  // A live agent TUI repaints continuously and focus-restore from the closing
  // popover can land user input — either can snap the viewport away before the
  // scroll ever paints. Re-assert the target for ~300ms, backing off the
  // moment the user wheel-scrolls themselves.
  let userScrolled = false;
  const onWheel = () => {
    userScrolled = true;
  };
  term.element?.addEventListener("wheel", onWheel, { passive: true });
  const reassert = () => {
    if (!userScrolled && term.buffer.active.viewportY !== target) {
      term.scrollToLine(target);
    }
  };
  requestAnimationFrame(reassert);
  window.setTimeout(reassert, 120);
  window.setTimeout(() => {
    reassert();
    term.element?.removeEventListener("wheel", onWheel);
  }, 300);
  flashJumpLine(term, line);
}

// Brief full-width spruce band on the jump target so the click reads as a jump
// even when the viewport didn't need to move (a short session fits one screen).
// registerMarker's offset is cursor-relative (see the link-hover code above);
// an unanchored turn's synthetic high-band line yields no valid marker and the
// flash silently skips — scrollToLine already clamped to the bottom.
function flashJumpLine(term: Terminal, absLine: number): void {
  const buf = term.buffer.active;
  const marker = term.registerMarker(absLine - (buf.baseY + buf.cursorY));
  if (!marker || marker.line < 0) return;
  const decoration = term.registerDecoration({
    marker,
    width: term.cols,
    backgroundColor: resolveCssColorToHex("var(--primary)", "#5b8a6f"),
    layer: "bottom",
  });
  if (!decoration) {
    marker.dispose();
    return;
  }
  window.setTimeout(() => {
    try {
      decoration.dispose();
      marker.dispose();
    } catch {}
  }, 900);
}

export function getLeafBlockMode(leafId: number): BlockMode {
  return sessions.get(leafId)?.blockMode ?? "prompt";
}

export function subscribeLeafBlockMode(
  leafId: number,
  cb: () => void,
): () => void {
  const s = sessions.get(leafId);
  if (!s) return () => {};
  s.blockListeners.add(cb);
  return () => {
    s.blockListeners.delete(cb);
  };
}

export function setLeafInputFocus(
  leafId: number,
  fn: (() => void) | null,
): void {
  const s = sessions.get(leafId);
  if (s) s.inputFocus = fn;
}

export function focusLeafInput(leafId: number): void {
  sessions.get(leafId)?.inputFocus?.();
}

export function getLeafDraft(leafId: number): string {
  return sessions.get(leafId)?.inputDraft ?? "";
}

export function setLeafDraft(leafId: number, text: string): void {
  const s = sessions.get(leafId);
  if (s) s.inputDraft = text;
}

export function setLeafInputActivity(leafId: number, active: boolean): void {
  const s = sessions.get(leafId);
  if (!s || s.inputActive === active) return;
  s.inputActive = active;
  const set = blockViewportListeners.get(leafId);
  if (set) for (const l of set) l();
}

export type WatermarkState = "visible" | "hidden" | "dead";

// Watermark gate: a block terminal that has never run a command, whose grid is
// still untouched, and whose input is empty. Synchronous so tab switches, slot
// rebinds and the Enter-to-OSC-133 gap never flash it over real content.
// "dead" is permanent and lets the component unmount for good. The grid check
// scans glyphs, not the cursor: the prompt integration prints a blank gap line
// at spawn, so the cursor sits below row 0 even on a visually empty terminal.
export function blockWatermarkState(leafId: number): WatermarkState {
  const s = sessions.get(leafId);
  if (!s || s.disposed) return "dead";
  if (s.everSubmitted || s.blockDecorations?.hasAnyBlock()) return "dead";
  if (!s.blockDecorations || s.inputActive) return "hidden";
  const slot = getSlotForLeaf(leafId);
  if (!slot) return "hidden";
  const buf = slot.term.buffer.active;
  if (buf.baseY > 0) return "dead";
  const rows = Math.min(buf.length, slot.term.rows);
  for (let i = 0; i < rows; i++) {
    if (buf.getLine(i)?.translateToString(true)) return "dead";
  }
  return "visible";
}

/**
 * Clear the scrollback and screen of the currently focused terminal, keeping
 * the active prompt line — macOS Terminal's ⌘K behaviour. Returns false when no
 * focused terminal slot is bound (e.g. focus is in the editor or AI panel).
 */
export function clearFocusedTerminal(): boolean {
  for (const [leafId, s] of sessions) {
    if (!s.visibleNow || !s.focusedNow) continue;
    const slot = getSlotForLeaf(leafId);
    if (!slot) continue;
    slot.term.clear();
    return true;
  }
  return false;
}

export function leafIdForPty(ptyId: number): number | null {
  for (const [leafId, s] of sessions) {
    if (s.pty?.id === ptyId) return leafId;
  }
  return null;
}

// Reverse of leafIdForPty: the session-owned pty id for a leaf (bus lines are
// tagged with KODEN_SESSION = pty id). Null until the pty has opened.
export function ptyIdForLeaf(leafId: number): number | null {
  return sessions.get(leafId)?.pty?.id ?? null;
}

function leafBusy(s: Session): boolean {
  return s.commandRunning || (s.pty !== null && isAgentActivePty(s.pty.id));
}

const HIDDEN_RELEASE_DELAY_MS = 300;

// A parked hidden leaf went idle: give the post-command prompt a moment to
// render into the live buffer, then hand the slot back to the pool.
function scheduleHiddenRelease(leafId: number, s: Session): void {
  if (s.visibleNow || !s.hasSlot) return;
  cancelHiddenRelease(s);
  s.hiddenReleaseTimer = setTimeout(() => {
    s.hiddenReleaseTimer = null;
    if (s.disposed || s.visibleNow || !s.hasSlot) return;
    if (s.blocks || isLeafAltScreen(leafId) || leafBusy(s)) return;
    unbindLeafFromSlot(leafId, s);
  }, HIDDEN_RELEASE_DELAY_MS);
}

function cancelHiddenRelease(s: Session): void {
  if (s.hiddenReleaseTimer !== null) {
    clearTimeout(s.hiddenReleaseTimer);
    s.hiddenReleaseTimer = null;
  }
}

async function releaseIfIdle(leafId: number, s: Session): Promise<void> {
  const busy = await leafHasForegroundJob(leafId);
  if (busy || s.disposed || s.visibleNow || !s.hasSlot) return;
  if (s.blocks || isLeafAltScreen(leafId) || leafBusy(s)) return;
  unbindLeafFromSlot(leafId, s);
}

async function leafHasForegroundJob(leafId: number): Promise<boolean> {
  const s = sessions.get(leafId);
  if (!s?.pty || s.shellExited) return false;
  try {
    return await invoke<boolean>("pty_has_foreground_job", { id: s.pty.id });
  } catch (e) {
    console.error("[koden] pty_has_foreground_job failed for leaf", leafId, e);
    return false;
  }
}

function onLeafCommandState(leafId: number, running: boolean): void {
  const s = sessions.get(leafId);
  if (!s || s.commandRunning === running) return;
  s.commandRunning = running;
  if (!running) {
    scheduleHiddenRelease(leafId, s);
    return;
  }
  cancelHiddenRelease(s);
  // A command started in a hidden released leaf (e.g. submitted by the AI):
  // rebind its retained slot so output parses live instead of filling the
  // ring. Deferred: this callback fires inside xterm's parse loop and the
  // rebind touches the same terminal (fit/resize).
  if (!s.visibleNow && !s.hasSlot && s.container && !s.disposed) {
    setTimeout(() => {
      if (s.disposed || s.visibleNow || s.hasSlot || !s.container) return;
      if (!leafBusy(s)) return;
      bindLeafToSlot(leafId, s);
      parkLeafSlot(leafId);
    }, 0);
  }
}

ensureAgentActivityListener((ptyId) => {
  const leafId = leafIdForPty(ptyId);
  if (leafId === null) return;
  const s = sessions.get(leafId);
  if (s) scheduleHiddenRelease(leafId, s);
});

configureRendererPool({
  resolveLeaf(leafId) {
    const s = sessions.get(leafId);
    if (!s) return null;
    return {
      writeToPty: (data) => {
        // Shell spawn failed (bad cwd, missing binary): Enter retries.
        if (s.spawnFailed) {
          if (data.includes("\r")) void respawnSession(leafId);
          return;
        }
        s.pty?.write(data);
      },
      resizePty: (cols, rows) => {
        s.cols = cols;
        s.rows = rows;
        s.pty?.resize(cols, rows);
      },
      kickPty: (cols, rows) => {
        const pty = s.pty;
        if (!pty || cols <= 0 || rows <= 0) return;
        // Linux only emits SIGWINCH when the winsize ioctl actually
        // changes dims, so bump +1 row then restore. The TUI receives
        // (possibly two) SIGWINCHes and repaints from scratch.
        pty
          .resize(cols, rows + 1)
          .then(() => pty.resize(cols, rows))
          .catch((e) => console.warn("[koden] kickPty failed:", e));
      },
    };
  },
  evictLeaf(leafId) {
    const s = sessions.get(leafId);
    if (!s) return;
    unbindLeafFromSlot(leafId, s);
  },
  isLeafFocused(leafId) {
    const s = sessions.get(leafId);
    return !!s && s.visibleNow && s.focusedNow;
  },
  isLeafBlocks(leafId) {
    return sessions.get(leafId)?.blocks ?? false;
  },
  isLeafBusy(leafId) {
    const s = sessions.get(leafId);
    return !!s && leafBusy(s);
  },
  isLeafVisible(leafId) {
    return sessions.get(leafId)?.visibleNow ?? false;
  },
  storeSnapshot(leafId, out) {
    const s = sessions.get(leafId);
    if (!s) return;
    s.snapshot = out.snapshot;
    if (out.cols > 0) s.cols = out.cols;
    if (out.rows > 0) s.rows = out.rows;
    s.altScreenAtRelease = out.altScreen;
  },
});

function ensureSession(
  leafId: number,
  initialCwd?: string,
  blocks = false,
): Session {
  const existing = sessions.get(leafId);
  if (existing) return existing;

  const session: Session = {
    pty: null,
    ptyOpening: false,
    initialCwd,
    lastCwd: null,
    pendingExit: null,
    shellExited: false,
    callbacks: {},
    visibleNow: false,
    focusedNow: false,
    disposed: false,
    ready: Promise.resolve(),
    cols: 0,
    rows: 0,
    container: null,
    snapshot: null,
    searchQuery: null,
    dormantRing: new DormantRing(),
    hasSlot: false,
    blocks,
    blockMode: "prompt",
    blockListeners: new Set(),
    blockDecorations: null,
    commandMarks: null,
    inputFocus: null,
    inputDraft: "",
    inputActive: false,
    everSubmitted: false,
    altScreenAtRelease: false,
    commandRunning: false,
    hiddenReleaseTimer: null,
    spawnFailed: false,
  };
  sessions.set(leafId, session);

  session.ready = (async () => {
    await ensureMonoFontsLoaded();
    await document.fonts.ready;
  })();

  return session;
}

function deliverPtyBytes(leafId: number, bytes: Uint8Array): void {
  const s = sessions.get(leafId);
  if (!s) return;
  // Retained slots keep parsing live (render paused); the ring is only for
  // leaves whose buffer was stolen or never bound.
  const slot = getLiveSlotForLeaf(leafId);
  if (slot) slot.term.write(bytes);
  else s.dormantRing.push(bytes);
}

const SPAWN_RETRY_DELAY_MS = 250;

async function openPtyWithRetry(
  leafId: number,
  s: Session,
  cwd: string | undefined,
): Promise<PtySession> {
  try {
    return await openPtyForSession(leafId, s, cwd);
  } catch (e) {
    console.error("[koden] openPty failed, retrying once:", e);
    await new Promise((r) => setTimeout(r, SPAWN_RETRY_DELAY_MS));
    if (s.disposed) throw e;
    return openPtyForSession(leafId, s, cwd);
  }
}

// Spawn failure must not flow through onExit: handleLeafExit closes the pane
// (or respawns the last one, which would loop). Show the error in the pane
// and let Enter retry instead of leaving a dead black grid.
function surfaceSpawnFailure(leafId: number, s: Session, e: unknown): void {
  console.error("[koden] shell spawn failed:", e);
  s.shellExited = true;
  s.spawnFailed = true;
  const detail = String(e)
    .replace(/[\x00-\x1f\x7f]/g, " ")
    .slice(0, 300);
  deliverPtyBytes(
    leafId,
    new TextEncoder().encode(
      `\r\n\x1b[31m[koden] failed to start shell: ${detail}\x1b[0m\r\n\x1b[2mpress Enter to retry\x1b[0m\r\n`,
    ),
  );
}

async function openPtyForSession(
  leafId: number,
  s: Session,
  cwd: string | undefined,
): Promise<PtySession> {
  const startCols = s.cols > 0 ? s.cols : 80;
  const startRows = s.rows > 0 ? s.rows : 24;
  const pty = await openPty(
    startCols,
    startRows,
    {
      onData: (bytes) => deliverPtyBytes(leafId, bytes),
      onExit: (code) => {
        s.shellExited = true;
        s.pty = null;
        s.commandRunning = false;
        const slot = getSlotForLeaf(leafId);
        if (slot) slot.term.options.disableStdin = true;
        scheduleHiddenRelease(leafId, s);
        if (s.callbacks.onExit) s.callbacks.onExit(code);
        else s.pendingExit = code;
      },
    },
    cwd,
    s.blocks,
  );
  // Only resize if the bound dims changed during the spawn: a same-size
  // ResizePseudoConsole during conhost warmup is a known ConPTY trigger for
  // a console that never renders (blank tab).
  if (
    s.cols > 0 &&
    s.rows > 0 &&
    (s.cols !== startCols || s.rows !== startRows)
  ) {
    void pty.resize(s.cols, s.rows);
  }
  return pty;
}

function applyBlockMode(leafId: number, mode: BlockMode): void {
  const s = sessions.get(leafId);
  if (!s) return;
  s.blockMode = mode;
  s.commandRunning = mode !== "prompt";
  const slot = getSlotForLeaf(leafId);
  if (slot) {
    const prompt = mode === "prompt";
    slot.term.options.disableStdin = prompt;
    // Disable the helper textarea at the prompt so a grid click can't focus the
    // xterm (no flashing cursor) and can't steal focus from the shell input.
    if (slot.term.textarea) slot.term.textarea.disabled = prompt;
    if (!prompt) {
      slot.term.focus();
    } else if (s.visibleNow && s.focusedNow) {
      const inputFocus = s.inputFocus;
      if (inputFocus) setTimeout(inputFocus, 0);
    }
  }
  for (const l of s.blockListeners) l();
}

// Drive-letter normalization, mirroring parseOsc7 in osc-handlers.ts: turn
// backslashes into forward slashes (xterm/Rust canonical form) and strip a
// leading "/" that precedes a Windows drive letter (/C:/x -> C:/x).
function normalizeFsPath(raw: string): string {
  let p = raw.replace(/\\/g, "/");
  if (/^\/[A-Za-z]:/.test(p)) p = p.slice(1);
  return p;
}

function isAbsoluteish(p: string): boolean {
  return (
    p.startsWith("/") ||
    p.startsWith("~") ||
    /^[A-Za-z]:\//.test(p) ||
    p.startsWith("//") // normalized UNC
  );
}

// Resolve a relative path (./x, ../x, plain x/y) against the leaf's last cwd so
// reveal targets the right file. Absolute and home paths pass through. Kept
// deliberately simple: it does not collapse ".." segments, the OS opener does.
function resolveAgainstCwd(path: string, cwd: string | null): string {
  if (isAbsoluteish(path) || !cwd) return path;
  const base = cwd.replace(/\/+$/, "");
  return `${base}/${path.replace(/^\.\//, "")}`;
}

// xterm's IDecorationOptions.backgroundColor accepts only #RRGGBB (no alpha),
// so a subtle tint is produced by blending an accent toward the terminal
// background rather than relying on opacity. Colors come from the central theme
// engine (resolved to rgb() by the token probe), keeping the highlight in step
// with light/dark and every preset. Copy links tint toward cyan, path/open
// links toward blue, so the highlight itself signals what a click does.
type Rgb = { r: number; g: number; b: number };

function parseRgb(css: string): Rgb | null {
  const m = css.match(/(\d+(?:\.\d+)?)/g);
  if (!m || m.length < 3) return null;
  return { r: Number(m[0]), g: Number(m[1]), b: Number(m[2]) };
}

function mix(a: Rgb, b: Rgb, t: number): Rgb {
  return {
    r: Math.round(a.r + (b.r - a.r) * t),
    g: Math.round(a.g + (b.g - a.g) * t),
    b: Math.round(a.b + (b.b - a.b) * t),
  };
}

function toHex({ r, g, b }: Rgb): string {
  const h = (n: number) =>
    Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}`;
}

// 0.28 of the accent over the terminal background reads as a clear-but-subtle
// wash without drowning the glyphs. Falls back to a neutral gray pair if the
// theme vars are unset (empty CSS var resolves to an unparseable string).
function linkHighlightColor(action: LinkAct): string {
  const t = readTerminalTokens();
  const bg = parseRgb(t.background) ?? { r: 24, g: 24, b: 27 };
  const accentCss = action === "copy" ? t.ansiCyan : t.ansiBlue;
  const accent = parseRgb(accentCss) ?? { r: 120, g: 130, b: 150 };
  return toHex(mix(bg, accent, 0.28));
}

// A DOM hover badge gives the per-link affordance xterm's two-flag
// ILinkDecorations can't: a colored "Open" vs "Copy" label so the user knows
// what a Ctrl/Cmd+click will do. xterm routes mouse events around any child of
// term.element carrying the `xterm-hover` class.
function showLinkBadge(term: Terminal, action: LinkAct, e: MouseEvent): void {
  const host = term.element;
  if (!host) return;
  removeLinkBadge(term);
  const badge = document.createElement("div");
  badge.className = `xterm-hover koden-link-badge koden-link-badge-${action}`;
  badge.textContent = action === "copy" ? "Copy" : "Open";
  const rect = host.getBoundingClientRect();
  badge.style.left = `${Math.max(0, e.clientX - rect.left + 12)}px`;
  badge.style.top = `${Math.max(0, e.clientY - rect.top + 12)}px`;
  host.appendChild(badge);
}

function removeLinkBadge(term: Terminal): void {
  term.element?.querySelector(".koden-link-badge")?.remove();
}

function copyValue(value: string): void {
  void clipboardWriteText(value).then(() => toast.success("Copied"));
}

function createSmartLinkProvider(leafId: number): ILinkProvider {
  return {
    provideLinks(line, callback) {
      if (!usePreferencesStore.getState().smartLinksEnabled) {
        callback(undefined);
        return;
      }
      const s = sessions.get(leafId);
      const slot = getSlotForLeaf(leafId);
      const term = slot?.term;
      if (!s || !term) {
        callback(undefined);
        return;
      }
      // Only the hovered line: cheap, and never eager per write.
      const buf = term.buffer.active;
      const text = buf.getLine(line - 1)?.translateToString(true);
      if (!text) {
        callback(undefined);
        return;
      }
      const detected = detectLinks(
        text,
        usePreferencesStore.getState().linkTypes,
      );
      if (detected.length === 0) {
        callback(undefined);
        return;
      }
      const links: ILink[] = detected.map((d) => {
        // Per-link highlight handles, disposed in leave() and on dispose().
        let marker: IMarker | null = null;
        let decoration: IDecoration | null = null;
        const clearHighlight = () => {
          decoration?.dispose();
          marker?.dispose();
          decoration = null;
          marker = null;
        };
        return {
          // xterm columns are 1-based; +1 for start, end is inclusive so the
          // detector's exclusive end already lands on the right cell.
          range: {
            start: { x: d.start + 1, y: line },
            end: { x: d.end, y: line },
          },
          text: d.value,
          // Background highlight (below) replaces the underline; keep the
          // pointer cursor as the hover affordance.
          decorations: { pointerCursor: true, underline: false },
          hover: (e) => {
            showLinkBadge(term, d.action, e);
            clearHighlight();
            // registerMarker's offset is relative to the CURRENT cursor row, so
            // recompute it here, not at provideLinks time: the cursor may have
            // moved since. `line` is the 1-based absolute buffer line (the OSC
            // handlers map it with getLine(line-1)); subtract the absolute
            // cursor row. Negative offsets address scrollback.
            const b = term.buffer.active;
            const m = term.registerMarker(line - 1 - (b.baseY + b.cursorY));
            if (!m) return;
            marker = m;
            decoration =
              term.registerDecoration({
                marker: m,
                x: d.start,
                width: d.end - d.start,
                backgroundColor: linkHighlightColor(d.action),
                layer: "bottom",
              }) ?? null;
            if (!decoration) {
              m.dispose();
              marker = null;
            }
          },
          leave: () => {
            removeLinkBadge(term);
            clearHighlight();
          },
          dispose: () => clearHighlight(),
          activate: (e, value) => {
            removeLinkBadge(term);
            clearHighlight();
            // Plain left-click ALWAYS copies; Ctrl/Cmd+click opens (only for
            // "open"-typed categories). "copy"-typed categories copy either way.
            // The URL provider (rendererPool) uses the same model.
            const wantsOpen = (e.ctrlKey || e.metaKey) && d.action === "open";
            if (!wantsOpen) {
              copyValue(value);
              return;
            }
            // Ctrl/Cmd+click on an openable token: a filesystem path is revealed
            // in the file manager (never open/exec'd — output can be untrusted);
            // an email opens as mailto:; anything else is handed to the OS opener
            // and copied on failure so the click is never a no-op.
            if (d.category === "path") {
              const resolved = resolveAgainstCwd(
                normalizeFsPath(value),
                s.lastCwd,
              );
              void revealInFinder(resolved);
              return;
            }
            const target = d.category === "email" ? `mailto:${value}` : value;
            void openUrl(target).catch(() => copyValue(value));
          },
        };
      });
      callback(links);
    },
  };
}

function bindLeafToSlot(leafId: number, s: Session): void {
  if (!s.container) return;
  const altScreen = s.altScreenAtRelease;
  s.altScreenAtRelease = false;
  acquireSlot({
    leafId,
    container: s.container,
    snapshot: s.snapshot,
    altScreen,
    drainRing: (write) => s.dormantRing.drain(write),
    // Keep stdin alive after a spawn failure so Enter can trigger the retry.
    shellExited: s.shellExited && !s.spawnFailed,
    searchQuery: s.searchQuery,
    cols: s.cols,
    rows: s.rows,
    registerOsc: (term) => {
      if (s.blocks) {
        const deco = new BlockDecorations(term, {
          onCwd: (next) => {
            markSessionReady(leafId);
            if (s.lastCwd === next) return;
            s.lastCwd = next;
            s.callbacks.onCwd?.(next);
          },
          onMode: (mode) => applyBlockMode(leafId, mode),
          onViewport: () => {
            const set = blockViewportListeners.get(leafId);
            if (set) for (const l of set) l();
          },
        });
        s.blockDecorations = deco;
        const onGridFocus = () => {
          if (s.blockMode === "prompt") s.inputFocus?.();
        };
        term.textarea?.addEventListener("focus", onGridFocus);
        return [
          () => {
            s.blockDecorations = null;
            deco.dispose();
            term.textarea?.removeEventListener("focus", onGridFocus);
          },
        ];
      }
      // Shared in-command flag — see osc-handlers.ts. The prompt tracker
      // flips it on OSC 133 B/C/D/A; the cwd handler reads it to ignore OSC
      // 7 emitted by untrusted command output (remote SSH, `cat` of an
      // attacker file, etc.).
      const shellState = createShellIntegrationState();
      const prompt = registerPromptTracker(term, shellState, (running) =>
        onLeafCommandState(leafId, running),
      );
      const cwd = registerCwdHandler(
        term,
        (next) => {
          markSessionReady(leafId);
          if (s.lastCwd === next) return;
          s.lastCwd = next;
          s.callbacks.onCwd?.(next);
        },
        shellState,
      );
      // Path + copy links. WebLinksAddon (http/https) is loaded on the slot
      // and runs first; this provider handles the rest and skips URL ranges.
      // Disposed per-leaf with the OSC handlers so a recycled slot never keeps
      // a stale leaf's provider.
      const linkProvider = term.registerLinkProvider(
        createSmartLinkProvider(leafId),
      );
      // Command minimap tracker. Registered after the prompt tracker so its OSC
      // 133 handler runs first (xterm dispatches in reverse order); it returns
      // false to fall through to the prompt tracker, which must keep flipping
      // commandRunning / inCommand. Lives on every non-blocks leaf; the overlay
      // mount is what's gated by the pref, this is cheap when unobserved.
      const commandMarks = new CommandMarks(term, {
        onChange: () => notifyCommands(leafId),
        getBusTurns: () => busTurnsForLeaf(leafId),
      });
      s.commandMarks = commandMarks;
      return [
        prompt.dispose,
        cwd,
        () => {
          removeLinkBadge(term);
          linkProvider.dispose();
        },
        () => {
          s.commandMarks = null;
          commandMarks.dispose();
        },
      ];
    },
    onSearchReady: (addon) => {
      searchAddons.set(leafId, addon);
      s.callbacks.onSearchReady?.(addon);
    },
  });
  s.snapshot = null;
  s.hasSlot = true;
  if (s.blocks) applyBlockMode(leafId, s.blockMode);
  if (s.lastCwd !== null) s.callbacks.onCwd?.(s.lastCwd);
  if (s.pendingExit !== null) {
    const code = s.pendingExit;
    s.pendingExit = null;
    s.callbacks.onExit?.(code);
  }
}

function unbindLeafFromSlot(leafId: number, s: Session): void {
  if (!s.hasSlot) return;
  const out = releaseSlot(leafId);
  if (out) {
    if (out.cols > 0) s.cols = out.cols;
    if (out.rows > 0) s.rows = out.rows;
  }
  s.hasSlot = false;
}

function attachSession(
  leafId: number,
  container: HTMLDivElement,
  callbacks: Callbacks,
): void {
  const s = sessions.get(leafId);
  if (!s || s.disposed) return;
  s.callbacks = callbacks;
  s.container = container;

  if (s.visibleNow) bindLeafToSlot(leafId, s);

  if (!s.pty && !s.ptyOpening && !s.shellExited) {
    s.ptyOpening = true;
    openPtyWithRetry(leafId, s, s.initialCwd)
      .then((pty) => {
        s.ptyOpening = false;
        if (s.disposed) {
          pty.close();
          return;
        }
        s.pty = pty;
      })
      .catch((e) => {
        s.ptyOpening = false;
        if (!s.disposed) surfaceSpawnFailure(leafId, s, e);
      });
  }
}

function detachSession(leafId: number): void {
  const s = sessions.get(leafId);
  if (!s) return;
  unbindLeafFromSlot(leafId, s);
  s.callbacks = {};
  s.container = null;
}

export async function respawnSession(
  leafId: number,
  cwd?: string,
): Promise<void> {
  const s = sessions.get(leafId);
  if (!s || s.disposed) return;
  s.pty?.close();
  s.pty = null;
  s.snapshot = null;
  s.dormantRing = new DormantRing();
  s.shellExited = false;
  s.pendingExit = null;
  s.altScreenAtRelease = false;
  s.commandRunning = false;
  s.spawnFailed = false;
  cancelHiddenRelease(s);

  const slot = getSlotForLeaf(leafId);
  if (slot) {
    slot.term.options.disableStdin = false;
    slot.term.clear();
    slot.term.reset();
  } else {
    discardRetainedSlot(leafId);
  }

  s.ptyOpening = true;
  let pty: PtySession;
  try {
    pty = await openPtyWithRetry(leafId, s, cwd ?? s.initialCwd);
  } catch (e) {
    s.ptyOpening = false;
    if (!s.disposed) surfaceSpawnFailure(leafId, s, e);
    return;
  }
  s.ptyOpening = false;
  if (s.disposed) {
    pty.close();
    return;
  }
  s.pty = pty;
}

export async function leafHasForegroundProcess(
  leafId: number,
): Promise<boolean> {
  const s = sessions.get(leafId);
  if (!s?.pty || s.shellExited) return false;
  try {
    const result = await invoke<boolean>("pty_has_foreground_process", {
      id: s.pty.id,
    });
    return result;
  } catch (e) {
    console.error(
      "[koden] pty_has_foreground_process failed for leaf",
      leafId,
      e,
    );
    return false;
  }
}

export function disposeSession(leafId: number): void {
  const s = sessions.get(leafId);
  if (!s) return;
  s.disposed = true;
  cancelHiddenRelease(s);
  disposeLeafSlot(leafId);
  s.hasSlot = false;
  s.snapshot = null;
  s.pty?.close();
  s.pty = null;
  sessions.delete(leafId);
  blockViewportListeners.delete(leafId);
  commandListeners.delete(leafId);
  searchAddons.delete(leafId);
  clearBusTurns(leafId);
  readyLeaves.delete(leafId);
  const waiters = readyWaiters.get(leafId);
  if (waiters) {
    readyWaiters.delete(leafId);
    for (const w of waiters) {
      clearTimeout(w.timer);
      w.resolve();
    }
  }
}

type Options = {
  leafId: number;
  container: React.RefObject<HTMLDivElement | null>;
  visible: boolean;
  focused?: boolean;
  initialCwd?: string;
  blocks?: boolean;
  onSearchReady?: (addon: SearchAddon) => void;
  onExit?: (code: number) => void;
  onCwd?: (cwd: string) => void;
};

export function useTerminalSession({
  leafId,
  container,
  visible,
  focused = true,
  initialCwd,
  blocks = false,
  onSearchReady,
  onExit,
  onCwd,
}: Options) {
  const cbRef = useRef({ onSearchReady, onExit, onCwd });
  cbRef.current = { onSearchReady, onExit, onCwd };

  // initialCwd seeds the first PTY spawn only. It must NOT be an effect dep:
  // OSC 7 updates the leaf cwd on every `cd`, and re-running the bind effect
  // would detach/rebind the renderer slot (disposing block markers) on each cd.
  const initialCwdRef = useRef(initialCwd);
  initialCwdRef.current = initialCwd;

  useEffect(() => {
    let cancelled = false;
    const s = ensureSession(leafId, initialCwdRef.current, blocks);
    s.ready.then(() => {
      if (cancelled || s.disposed) return;
      const node = container.current;
      if (!node) return;
      attachSession(leafId, node, {
        onSearchReady: (a) => cbRef.current.onSearchReady?.(a),
        onExit: (c) => cbRef.current.onExit?.(c),
        onCwd: (c) => cbRef.current.onCwd?.(c),
      });
      if (s.visibleNow && s.focusedNow && !s.blocks) focusSlot(leafId);
    });
    return () => {
      cancelled = true;
      detachSession(leafId);
    };
  }, [leafId, container, blocks]);

  const [blockMode, setBlockMode] = useState<BlockMode>("prompt");
  useEffect(() => {
    if (!blocks) return;
    const s = ensureSession(leafId, initialCwdRef.current, blocks);
    setBlockMode(s.blockMode);
    const cb = () => setBlockMode(sessions.get(leafId)?.blockMode ?? "prompt");
    s.blockListeners.add(cb);
    return () => {
      s.blockListeners.delete(cb);
    };
  }, [leafId, blocks]);

  const fontSize = usePreferencesStore((p) => p.terminalFontSize);
  const zoomLevel = usePreferencesStore((p) => p.zoomLevel);
  useEffect(() => {
    applyFontSize(Math.max(4, Math.round(fontSize * zoomLevel)));
  }, [fontSize, zoomLevel]);

  const fontFamily = usePreferencesStore((p) => p.terminalFontFamily);
  useEffect(() => {
    applyFontFamily(fontFamily);
  }, [fontFamily]);

  const letterSpacing = usePreferencesStore((p) => p.terminalLetterSpacing);
  useEffect(() => {
    applyLetterSpacing(letterSpacing);
  }, [letterSpacing]);

  const lineHeight = usePreferencesStore((p) => p.terminalLineHeight);
  useEffect(() => {
    applyLineHeight(lineHeight);
  }, [lineHeight]);

  const scrollback = usePreferencesStore((p) => p.terminalScrollback);
  useEffect(() => {
    applyScrollback(scrollback);
  }, [scrollback]);

  const webglPref = usePreferencesStore((p) => p.terminalWebglEnabled);
  useEffect(() => {
    applyWebglPreference(webglPref);
  }, [webglPref]);

  const cursorBlink = usePreferencesStore((p) => p.terminalCursorBlink);
  useEffect(() => {
    applyCursorBlink(cursorBlink);
  }, [cursorBlink]);

  const bgActive = usePreferencesStore(
    (p) => p.backgroundKind === "image" && !!p.backgroundImageId,
  );
  useEffect(() => {
    applyBackgroundActive(bgActive);
  }, [bgActive]);

  useEffect(() => {
    const s = sessions.get(leafId);
    if (!s) return;
    s.visibleNow = visible;
    s.focusedNow = focused;
    if (visible) {
      cancelHiddenRelease(s);
      if (s.container && !s.hasSlot) bindLeafToSlot(leafId, s);
      else if (s.hasSlot) refreshLeafSlot(leafId);
      setSlotFocused(leafId, focused);
      if (focused && !blocks) focusSlot(leafId);
    } else if (s.hasSlot) {
      // Always park first (keeps the grid live, pauses rendering); release
      // only after confirming nothing owns the terminal. Sync signals (OSC
      // 133, agent detect) short-circuit; the async foreground-process check
      // covers shells without integration.
      parkLeafSlot(leafId);
      if (!s.blocks && !isLeafAltScreen(leafId) && !leafBusy(s)) {
        void releaseIfIdle(leafId, s);
      }
    }
  }, [leafId, visible, focused, blocks]);

  const write = useCallback(
    (data: string) => sessions.get(leafId)?.pty?.write(data),
    [leafId],
  );

  const focus = useCallback(() => focusSlot(leafId), [leafId]);

  const getBuffer = useCallback(
    (maxLines = 200): string | null => {
      const s = sessions.get(leafId);
      if (!s) return null;
      const slot = getLiveSlotForLeaf(leafId);
      if (slot) {
        const buf = slot.term.buffer.active;
        const total = buf.length;
        const lines: string[] = [];
        const start = Math.max(0, total - maxLines);
        for (let i = start; i < total; i++) {
          lines.push(buf.getLine(i)?.translateToString(true) ?? "");
        }
        while (lines.length && lines[lines.length - 1] === "") lines.pop();
        return lines.join("\n");
      }
      if (!s.snapshot) return "";
      const plain = stripAnsi(s.snapshot);
      const lines = plain.split(/\r?\n/);
      const tail = lines.slice(-maxLines);
      while (tail.length && tail[tail.length - 1] === "") tail.pop();
      return tail.join("\n");
    },
    [leafId],
  );

  const getSelection = useCallback((): string | null => {
    const slot = getSlotForLeaf(leafId);
    const sel = slot?.term.getSelection() ?? "";
    return sel.length > 0 ? sel : null;
  }, [leafId]);

  const applyTheme = useCallback(() => {
    applyPoolTheme();
  }, []);

  const selectBlockAt = useCallback(
    (clientY: number) =>
      sessions.get(leafId)?.blockDecorations?.selectBlockAt(clientY),
    [leafId],
  );

  const readBlockId = useCallback(
    (id: string) =>
      sessions.get(leafId)?.blockDecorations?.readById(id) ?? null,
    [leafId],
  );

  const subscribeBlocks = useCallback(
    (cb: () => void) => {
      let set = blockViewportListeners.get(leafId);
      if (!set) {
        set = new Set();
        blockViewportListeners.set(leafId, set);
      }
      set.add(cb);
      return () => {
        const live = blockViewportListeners.get(leafId);
        live?.delete(cb);
        if (live && live.size === 0) blockViewportListeners.delete(leafId);
      };
    },
    [leafId],
  );

  const visibleBlocks = useCallback(
    (): VisibleBlocks =>
      sessions.get(leafId)?.blockDecorations?.visibleBlocks() ?? {
        blocks: [],
        sticky: null,
      },
    [leafId],
  );

  const searchBlock = useCallback(
    (id: string, query: string) =>
      sessions.get(leafId)?.blockDecorations?.searchBlock(id, query) ?? [],
    [leafId],
  );

  const revealMatch = useCallback(
    (m: BlockMatch) => sessions.get(leafId)?.blockDecorations?.revealMatch(m),
    [leafId],
  );

  const clearSearch = useCallback(
    () => sessions.get(leafId)?.blockDecorations?.clearSearch(),
    [leafId],
  );

  // Command minimap wiring. Same module-scope subscribe pattern as
  // subscribeBlocks so the overlay can attach before the session binds a slot.
  const subscribeCommands = useCallback(
    (cb: () => void) => {
      let set = commandListeners.get(leafId);
      if (!set) {
        set = new Set();
        commandListeners.set(leafId, set);
      }
      set.add(cb);
      return () => {
        const live = commandListeners.get(leafId);
        live?.delete(cb);
        if (live && live.size === 0) commandListeners.delete(leafId);
      };
    },
    [leafId],
  );

  const getCommandMarks = useCallback((): CommandMinimapData => {
    const cm = sessions.get(leafId)?.commandMarks;
    if (!cm) {
      return {
        marks: [],
        viewport: { top: 0, bottom: 0, length: 0 },
        altScreen: false,
      };
    }
    return {
      marks: cm.getMarks(),
      viewport: cm.viewport(),
      altScreen: cm.isAltScreen(),
    };
  }, [leafId]);

  const scrollToCommand = useCallback(
    (line: number) => {
      const slot = getSlotForLeaf(leafId);
      if (!slot) return;
      // Center the command in the viewport (same offset as block navigation /
      // revealMatch use). scrollToLine takes a 0-based absolute line.
      slot.term.scrollToLine(
        Math.max(0, line - Math.floor(slot.term.rows / 2)),
      );
    },
    [leafId],
  );

  return useMemo(
    () => ({
      write,
      focus,
      getBuffer,
      getSelection,
      applyTheme,
      blockMode,
      selectBlockAt,
      readBlockId,
      subscribeBlocks,
      visibleBlocks,
      searchBlock,
      revealMatch,
      clearSearch,
      subscribeCommands,
      getCommandMarks,
      scrollToCommand,
    }),
    [
      write,
      focus,
      getBuffer,
      getSelection,
      applyTheme,
      blockMode,
      selectBlockAt,
      readBlockId,
      subscribeBlocks,
      visibleBlocks,
      searchBlock,
      revealMatch,
      clearSearch,
      subscribeCommands,
      getCommandMarks,
      scrollToCommand,
    ],
  );
}

const ANSI_RE =
  /\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][AB012]|\x1b[78=>]|\x1bc|\x1b[NOP\]X^_]/g;

function stripAnsi(s: string): string {
  return s.replace(ANSI_RE, "");
}

export function terminalDebugStats() {
  const liveSessions = [...sessions.entries()].map(([leafId, s]) => ({
    leafId,
    pty: !!s.pty,
    visible: s.visibleNow,
    focused: s.focusedNow,
    hasSlot: s.hasSlot,
    ringBytes: s.dormantRing.byteLength(),
    snapshotLen: s.snapshot?.length ?? 0,
    shellExited: s.shellExited,
  }));
  const ringTotal = liveSessions.reduce((n, s) => n + s.ringBytes, 0);
  const snapshotTotal = liveSessions.reduce((n, s) => n + s.snapshotLen, 0);
  const slots = poolSlotStats();
  return {
    poolSize: poolSize(),
    webglContexts: slots.filter((s) => s.webgl).length,
    idleSlots: slots.filter((s) => s.leafId === null).length,
    slots,
    sessionCount: liveSessions.length,
    sessions: liveSessions,
    ringBytesTotal: ringTotal,
    snapshotCharsTotal: snapshotTotal,
    domCanvases: document.querySelectorAll("canvas").length,
    domScreens: document.querySelectorAll(".xterm-screen").length,
    domRows: document.querySelectorAll(".xterm-rows > div").length,
    jsHeapBytes:
      (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory
        ?.usedJSHeapSize ?? null,
  };
}

if (import.meta.env?.DEV && typeof window !== "undefined") {
  (window as unknown as { __kodenTerm?: unknown }).__kodenTerm =
    terminalDebugStats;
}

function tailLines(text: string, maxLines: number): string {
  const lines = text.split(/\r?\n/);
  const tail = lines.slice(-maxLines);
  while (tail.length && tail[tail.length - 1] === "") tail.pop();
  return tail.join("\n");
}

/**
 * Last `maxLines` of any leaf's buffer for the koden CLI (modules/cli): the
 * live or retained renderer slot first, else the dormant snapshot. `raw`
 * keeps ANSI (SerializeAddon replay); otherwise plain text. Null when the
 * leaf has no session at all.
 */
export function readLeafTail(
  leafId: number,
  maxLines: number,
  raw: boolean,
): string | null {
  const s = sessions.get(leafId);
  if (!s) return null;
  const slot = getLiveSlotForLeaf(leafId);
  if (slot) {
    if (raw) {
      return tailLines(
        slot.serializeAddon.serialize({ scrollback: maxLines }),
        maxLines,
      );
    }
    const buf = slot.term.buffer.active;
    const total = buf.length;
    const lines: string[] = [];
    for (let i = Math.max(0, total - maxLines); i < total; i++) {
      lines.push(buf.getLine(i)?.translateToString(true) ?? "");
    }
    while (lines.length && lines[lines.length - 1] === "") lines.pop();
    return lines.join("\n");
  }
  if (!s.snapshot) return "";
  return tailLines(raw ? s.snapshot : stripAnsi(s.snapshot), maxLines);
}
