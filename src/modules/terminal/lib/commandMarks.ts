import type { IMarker, Terminal } from "@xterm/xterm";

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
const SCAN_ID_BASE = 1_000_000_000;

export type CommandMarksOptions = {
  onChange?: () => void;
};

export class CommandMarks {
  private readonly marks: Mark[] = [];
  private idSeq = 0;
  private altScreen = false;
  private readonly disposers: (() => void)[] = [];
  private readonly onChange?: () => void;
  private notifyRaf: number | null = null;
  // Memoized scrollback scan for Claude user-prompt lines (scanTurns). The
  // active buffer only grows (lines append; xterm trims the TOP past the
  // scrollback cap, but `length` then stops growing), so keying the cache on
  // `buffer.active.length` re-scans only when new rows arrived. -1 forces the
  // first scan.
  private scanCache: CommandMark[] = [];
  private scanCacheLen = -1;

  constructor(
    private readonly term: Terminal,
    opts?: CommandMarksOptions,
  ) {
    this.onChange = opts?.onChange;
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
    const parsed = term.onWriteParsed(() => this.syncAlt());
    const scroll = term.onScroll(() => this.scheduleNotify());
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

  // Mint a turn mark carrying the REAL prompt text, delivered by the Claude Code
  // UserPromptSubmit bus hook (the reliable channel). Once any real turn mark
  // exists, getMarks() stops merging the lossy scrollback scrape — so this both
  // captures every turn and shows the actual prompt instead of a scraped line.
  addTurn(text: string): void {
    const t = text.trim().slice(0, 400);
    if (t) this.startMark(t, "turn");
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
    let hasTurnMark = false;
    for (const mk of this.marks) {
      if (mk.marker.isDisposed || mk.marker.line < 0) continue;
      if (mk.status === "turn") hasTurnMark = true;
      const text =
        mk.status === "turn"
          ? mk.text || this.turnText(buf, mk.marker.line)
          : mk.text || this.lineText(buf, mk.marker.line) || "command";
      out.push({ id: mk.id, line: mk.marker.line, text, status: mk.status });
    }
    // scanTurns (scraping the rendered `>` lines) is the FALLBACK ONLY. Once a
    // real turn mark arrives (the UserPromptSubmit bus hook with prompt text),
    // use those exclusively — the scrape is lossy (Claude's TUI repaints, so it
    // only catches the first turn) and would double-list or override real text.
    if (!hasTurnMark) {
      const seenLines = new Set<number>();
      for (const m of out) seenLines.add(m.line);
      for (const t of this.scanTurns()) {
        if (seenLines.has(t.line)) continue;
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
  // per growth and emit a CommandMark per matching line. This is the robust
  // path — it does not depend on the OSC 777 UserPromptSubmit hook (which the
  // installed Claude Code v2.1.x does not emit to the PTY), so a plain
  // `claude`/`cm` session still lists the user's messages.
  //
  // Memoized on buffer length: a full walk of a few thousand lines on demand
  // is cheap, and we only redo it when the buffer actually grew, so repeated
  // getMarks() reads (one per frame while the popover is open) reuse the scan.
  scanTurns(): CommandMark[] {
    const buf = this.term.buffer.active;
    const len = buf.length;
    if (len === this.scanCacheLen) return this.scanCache;

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
    this.scanCacheLen = len;
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
