import type { IMarker, Terminal } from "@xterm/xterm";
import type { BusTurn } from "./turnStore";

// Trimmed sibling of BlockDecorations: just enough OSC bookkeeping to power the
// command minimap. One tick per shell command (OSC 133;C/D) AND one per Claude
// user turn (OSC 777 `working`, the UserPromptSubmit hook) — so a minimap inside
// a claude session shows the user's messages, ChatGPT-style. The two signals are
// disjoint (a plain shell never emits OSC 777), so turn-marking is self-gating.
// No ranges, selection, search, decorations or chrome — only a registered start
// marker + a status per mark. ponytail: this is intentionally a fraction of
// BlockDecorations; the minimap never needs output ranges or end markers.

// "turn" is a Claude-session user turn (OSC 777), not a shell command — it has
// no exit code, so it never settles to ok/fail and gets its own minimap tick.
export type CommandStatus = "running" | "ok" | "fail" | "turn";

type Mark = {
  id: number;
  marker: IMarker;
  text: string;
  status: CommandStatus;
};

// A command mark projected to its current absolute buffer line. Disposed /
// trimmed marks (line < 0) are filtered out before this is produced.
export type CommandMark = {
  id: number;
  line: number;
  text: string;
  status: CommandStatus;
};

export type CommandViewport = {
  top: number;
  bottom: number;
  length: number;
};

// Snapshot consumed by the terminal history popover: every collected mark
// projected to its current buffer line, plus the live viewport band. (Used to
// live in CommandMinimap.tsx; moved here next to its source when the in-terminal
// strip was replaced by the header history popover.)
export type CommandMinimapData = {
  marks: CommandMark[];
  viewport: CommandViewport;
  altScreen: boolean;
};

// Cap roughly matching a deep scrollback; oldest are shifted + disposed.
const MAX_MARKS = 500;

// How many rows below a turn marker to scan for the rendered prompt text. The
// `>` line sits within claude's input box just under the submit point.
const TURN_TEXT_SCAN = 4;

// Scanned-turn ids are derived from the buffer line they sit on so a row keeps
// the same identity across re-reads (the popover keys React rows on id). They
// must not collide with the OSC-133 mark ids (a small `++idSeq` counter), so
// they live in a high, line-addressed band: SCAN_ID_BASE + bufferLine. A few
// hundred-thousand-line scrollback stays well under MAX_SAFE_INTEGER.
// Bus-delivered turns live above this band (turnStore.TURN_LINE_BASE).
const SCAN_ID_BASE = 1_000_000_000;

export type CommandMarksOptions = {
  onChange?: () => void;
  // Session-lifetime bus turns for this leaf (see turnStore.ts). Injected so
  // the storage outlives this per-slot-bind instance: a pool rebind constructs
  // a fresh CommandMarks over the same store and the Inputs history persists.
  getBusTurns?: () => readonly BusTurn[];
};

export class CommandMarks {
  private readonly marks: Mark[] = [];
  private idSeq = 0;
  private altScreen = false;
  private readonly disposers: (() => void)[] = [];
  private readonly onChange?: () => void;
  private readonly getBusTurns?: () => readonly BusTurn[];
  private notifyRaf: number | null = null;
  // Memoized scrollback scan for Claude user-prompt lines (scanTurns), keyed
  // on a monotonic dirty counter bumped by onWriteParsed/onScroll. It must NOT
  // key on `buffer.active.length`: once the scrollback cap is reached xterm
  // trims the TOP while `length` (and baseY) stay constant, so a length key
  // freezes the cache forever and turns typed after cap-fill never surface.
  private scanCache: CommandMark[] = [];
  private scanCacheKey = -1;
  private scanDirty = 0;

  constructor(
    private readonly term: Terminal,
    opts?: CommandMarksOptions,
  ) {
    this.onChange = opts?.onChange;
    this.getBusTurns = opts?.getBusTurns;
    const osc133 = term.parser.registerOscHandler(133, (data) => {
      this.onOsc133(data);
      // Return false so any other OSC-133 handler on the term (cwd/prompt
      // tracker) still runs; xterm stops at the first handler that returns true.
      return false;
    });
    // Claude Code's UserPromptSubmit hook emits OSC 777 `notify;Koden;working`
    // once per user turn (see agent.rs). A plain shell never emits OSC 777, so
    // receiving it IS the "armed claude session" signal — no extra gating
    // needed, and shell-command marks (OSC 133) stay disjoint from turn marks.
    const osc777 = term.parser.registerOscHandler(777, (data) => {
      this.onOsc777(data);
      // Return false so the agent detector / any other OSC-777 listener still
      // runs (xterm stops at the first handler that returns true).
      return false;
    });
    const parsed = term.onWriteParsed(() => {
      this.scanDirty++;
      this.syncAlt();
    });
    const scroll = term.onScroll(() => {
      this.scanDirty++;
      this.scheduleNotify();
    });
    const render = term.onRender(() => this.scheduleNotify());
    this.disposers.push(
      () => osc133.dispose(),
      () => osc777.dispose(),
      () => parsed.dispose(),
      () => scroll.dispose(),
      () => render.dispose(),
    );
  }

  // rAF-coalesce viewport churn (scroll/render fire per frame) so the React
  // overlay re-reads at most once per frame. Mirrors blockDecorations.ts.
  private scheduleNotify(): void {
    if (this.notifyRaf != null) return;
    this.notifyRaf = requestAnimationFrame(() => {
      this.notifyRaf = null;
      this.onChange?.();
    });
  }

  private syncAlt(): void {
    const alt = this.term.buffer.active.type === "alternate";
    if (alt === this.altScreen) return;
    this.altScreen = alt;
    this.scheduleNotify();
  }

  isAltScreen(): boolean {
    return this.altScreen;
  }

  private onOsc133(data: string): void {
    const marker = data[0];
    // Same payload split as BlockDecorations.onOsc133: "C;<cmd>" / "D;<code>".
    const rest = data.length > 2 && data[1] === ";" ? data.slice(2) : "";
    if (marker === "C") this.startMark(rest, "running");
    else if (marker === "D") this.finishMark(rest);
  }

  private onOsc777(data: string): void {
    // Only the per-turn submit marker mints a tick; attention/finished/etc.
    // are status transitions for the dock, not turn boundaries.
    if (!isTurnMarker(data)) return;
    // OSC 777 working carries no prompt text; getMarks() falls back to the
    // buffer line at the marker (the rendered prompt block) for the hover.
    this.startMark("", "turn");
  }

  private startMark(text: string, status: CommandStatus): void {
    const m = this.term.registerMarker(0);
    if (!m) return;
    this.marks.push({
      id: ++this.idSeq,
      marker: m,
      text,
      status,
    });
    while (this.marks.length > MAX_MARKS) {
      const old = this.marks.shift();
      try {
        old?.marker.dispose();
      } catch {}
    }
    this.scheduleNotify();
  }

  private finishMark(codeStr: string): void {
    // Settle the most-recent still-running mark. bash fires a bare D between
    // prompts even with nothing running, so guard on finding one.
    for (let i = this.marks.length - 1; i >= 0; i--) {
      const mk = this.marks[i];
      if (mk.status !== "running") continue;
      // Absent exit code = success (matches BlockDecorations): a bare `D` with
      // no code must not paint a red "fail" tick.
      const code = parseExitCode(codeStr);
      mk.status = code === null || code === 0 ? "ok" : "fail";
      this.scheduleNotify();
      return;
    }
  }

  // Project live marks to their current absolute line. bash emits no command
  // text on the C marker (PS0 can't interpolate), leaving text empty: fall back
  // to the buffer line just above the marker (the echoed command) so the hover
  // preview still shows something, else a generic label.
  getMarks(): CommandMark[] {
    const buf = this.term.buffer.active;
    const out: CommandMark[] = [];
    const markerTurns: CommandMark[] = [];
    for (const mk of this.marks) {
      if (mk.marker.isDisposed || mk.marker.line < 0) continue;
      const text =
        mk.status === "turn"
          ? mk.text || this.turnText(buf, mk.marker.line)
          : mk.text || this.lineText(buf, mk.marker.line) || "command";
      const row = {
        id: mk.id,
        line: mk.marker.line,
        text,
        status: mk.status,
      };
      if (mk.status === "turn") markerTurns.push(row);
      else out.push(row);
    }
    const busTurns = this.getBusTurns?.() ?? [];
    if (busTurns.length > 0) {
      // Bus turns (the UserPromptSubmit hook channel) are the authoritative
      // turn rows: real prompt text, one per submit. Marker/scanned turns are
      // the SAME submits with lossier text, so they are dropped as rows; but
      // when one carries matching text its buffer line becomes the bus turn's
      // scroll anchor, so click-to-scroll lands on the prompt instead of
      // clamping a synthetic high-band line to the bottom.
      const anchors = [...markerTurns, ...this.scanTurns()].sort(
        (a, b) => a.line - b.line,
      );
      const used = new Set<number>();
      for (const t of busTurns) {
        // Wrap/whitespace-tolerant match: the scraped line is a lossy render
        // of the same prompt, so exact equality misses wrapped or repainted
        // boxes — and an unanchored turn falls back to a high-band synthetic
        // line, which click-to-scroll can only clamp to the bottom.
        const anchor = anchors.find(
          (a) => !used.has(a.line) && turnTextMatches(a.text, t.text),
        );
        if (anchor) used.add(anchor.line);
        out.push({
          id: t.id,
          line: anchor ? anchor.line : t.id,
          text: t.text,
          status: "turn",
        });
      }
    } else {
      // No bus signal (hook missing, or session predates it): merge marker
      // turns with the scrollback scrape. The scrape must ALWAYS run here; a
      // CLI that emits the OSC 777 marker for only SOME submits (CC 2.1.206's
      // UI-gated terminalSequence) would otherwise hide every unmarked turn.
      // A scanned line duplicating a marker turn (same text within the rows
      // turnText scraped) is skipped.
      out.push(...markerTurns);
      const seenLines = new Set<number>();
      for (const m of out) seenLines.add(m.line);
      for (const t of this.scanTurns()) {
        if (seenLines.has(t.line)) continue;
        const dup = markerTurns.some(
          (mt) =>
            t.line >= mt.line &&
            t.line < mt.line + TURN_TEXT_SCAN &&
            mt.text === t.text,
        );
        if (dup) continue;
        seenLines.add(t.line);
        out.push(t);
      }
    }
    out.sort((a, b) => a.line - b.line);
    return out;
  }

  // Recover Claude user turns by reading the rendered scrollback directly,
  // independent of any OSC signal: Claude prints each submitted prompt as a
  // `>` / `❯` line (see extractTurnPrompt). Walk the whole active buffer once
  // per change batch and emit a CommandMark per matching line. This backstops
  // the OSC 777 UserPromptSubmit marker, whose emission in CC >= 2.1.206 is
  // UI-lifecycle-gated inside the CLI (a silent no-op whenever its emitter is
  // unregistered): treat the marker as best-effort, never the turn source of
  // truth. The bus channel (turnStore) is authoritative when present.
  //
  // Memoized on scanDirty (bumped per write/scroll): a full walk of a few
  // thousand lines on demand is cheap, and repeated getMarks() reads (one per
  // frame while the popover is open) reuse the scan between changes.
  scanTurns(): CommandMark[] {
    const buf = this.term.buffer.active;
    const len = buf.length;
    if (this.scanDirty === this.scanCacheKey) return this.scanCache;

    // The live input box at the bottom holds the prompt the user is CURRENTLY
    // typing; its `>` line is not a submitted turn yet. The cursor sits inside
    // that box, so skip any matching line at or below the cursor's absolute
    // row (only on the main screen — an alt-screen TUI has no Claude box).
    const liveInputFloor = this.altScreen ? len : buf.baseY + buf.cursorY;

    const turns: CommandMark[] = [];
    let lastText = "";
    for (let line = 0; line < len; line++) {
      if (line >= liveInputFloor) break;
      const text = extractTurnPrompt(
        buf.getLine(line)?.translateToString(true),
      );
      if (!text) continue;
      // Skip consecutive duplicates (a repainted box can echo the same prompt
      // on adjacent rows); a genuinely repeated question still shows once.
      if (text === lastText) continue;
      lastText = text;
      turns.push({
        id: SCAN_ID_BASE + line,
        line,
        text,
        status: "turn",
      });
    }
    this.scanCache = turns;
    this.scanCacheKey = this.scanDirty;
    return turns;
  }

  private lineText(buf: Terminal["buffer"]["active"], line: number): string {
    // The C marker lands on the first output row, so the command echo is the
    // row above. Fall back to the marker row itself if that's blank.
    const above = buf
      .getLine(line - 1)
      ?.translateToString(true)
      .trim();
    if (above) return above;
    return buf.getLine(line)?.translateToString(true).trim() ?? "";
  }

  // Claude's submit hook carries no prompt text, so recover it from the buffer
  // around the turn marker: claude renders the user's message in a box whose
  // line starts with a `>` prompt glyph. Scan a small window down from the
  // marker for that line; if nothing matches (TUI repaint, blank box) fall back
  // to a generic label so the hover is never empty.
  private turnText(buf: Terminal["buffer"]["active"], line: number): string {
    for (let i = line; i < line + TURN_TEXT_SCAN; i++) {
      const prompt = extractTurnPrompt(buf.getLine(i)?.translateToString(true));
      if (prompt) return prompt;
    }
    return "claude turn";
  }

  viewport(): CommandViewport {
    const buf = this.term.buffer.active;
    return {
      top: buf.viewportY,
      bottom: buf.viewportY + this.term.rows,
      length: buf.length,
    };
  }

  dispose(): void {
    if (this.notifyRaf != null) cancelAnimationFrame(this.notifyRaf);
    for (const mk of this.marks) {
      try {
        mk.marker.dispose();
      } catch {}
    }
    this.marks.length = 0;
    for (const d of this.disposers) {
      try {
        d();
      } catch {}
    }
    this.disposers.length = 0;
  }
}

export function parseExitCode(s: string): number | null {
  if (!s) return null;
  const n = Number.parseInt(s, 10);
  return Number.isFinite(n) ? n : null;
}

// True for the one OSC 777 payload that marks a user turn: the UserPromptSubmit
// hook's `notify;Koden;working`. Other 777 payloads (attention/finished) are
// dock status transitions, not turn boundaries, so they mint no tick.
export function isTurnMarker(data: string): boolean {
  return data === "notify;Koden;working";
}

export function normTurnText(s: string): string {
  return s.replace(/\s+/g, " ").trim();
}

// Match a scraped rendered prompt line against a bus-delivered prompt. Exact
// equality after whitespace normalization, or a long-enough prefix either way:
// a wrapped prompt renders only its first row (scraped ⊂ bus), and the bus
// text is sliced at 400 chars (bus ⊂ scraped for very long prompts). The
// 12-char floor keeps short prompts on exact match so "hi" can't anchor "hiii".
export function turnTextMatches(scraped: string, bus: string): boolean {
  const a = normTurnText(scraped);
  const b = normTurnText(bus);
  if (!a || !b) return false;
  if (a === b) return true;
  return (
    (a.length >= 12 && b.startsWith(a)) || (b.length >= 12 && a.startsWith(b))
  );
}

// claude renders the submitted prompt as a box line beginning with a `>` glyph
// (plain `>` or the heavy `❯`), optionally inside a box-drawing border. Pull the
// message text out of such a line; return "" for anything else (borders, the
// empty input placeholder, status spinner, etc.) so the caller keeps scanning.
export function extractTurnPrompt(raw: string | undefined): string {
  if (!raw) return "";
  // Drop any leading whitespace / box-drawing border before the prompt glyph.
  // Beyond plain spaces and light/heavy verticals, this covers NBSP and the
  // dashed / half-block verticals a TUI may draw the prompt box with — some
  // Claude builds (notably the GLM / z.ai endpoint) lead the line with one of
  // these instead of a plain space, which previously defeated the match.
  const trimmed = raw.replace(/^[\s │┃├┠┆┇┊┋╎╏▏▎▍▌▕|‖]+/, "").trimStart();
  const m = /^[>❯›]\s+(.+)$/.exec(trimmed);
  if (!m) return "";
  const text = m[1].trim();
  // Skip the empty-input placeholder claude shows when no prompt is typed.
  if (!text || text.startsWith("Try ") || text === "…") return "";
  return text;
}
