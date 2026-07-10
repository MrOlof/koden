import type { Terminal } from "@xterm/xterm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  CommandMarks,
  extractTurnPrompt,
  isTurnMarker,
  parseExitCode,
} from "./commandMarks";
import {
  addBusTurn,
  type BusTurn,
  busTurnsForLeaf,
  TURN_LINE_BASE,
} from "./turnStore";

type OscHandler = (data: string) => boolean | Promise<boolean>;

// CommandMarks coalesces notifications through rAF; the headless test env has
// none and these tests read getMarks() synchronously, so a no-op is enough.
beforeEach(() => {
  vi.stubGlobal("requestAnimationFrame", () => 1);
  vi.stubGlobal("cancelAnimationFrame", () => {});
});
afterEach(() => {
  vi.unstubAllGlobals();
});

// Minimal headless xterm fake backed by a fixed array of buffer lines, enough
// for scanTurns to walk the scrollback. cursorY defaults past the end so the
// live-input-box floor never trims a test line unless a test sets it.
function makeFakeTerm(lines: string[], cursorY = lines.length) {
  const handlers = new Map<number, OscHandler>();
  const term = {
    options: {} as Record<string, unknown>,
    element: null,
    parser: {
      registerOscHandler(code: number, h: OscHandler) {
        handlers.set(code, h);
        return { dispose: () => handlers.delete(code) };
      },
    },
    registerMarker: vi.fn(() => ({
      line: 0,
      isDisposed: false,
      dispose: vi.fn(),
    })),
    onWriteParsed: vi.fn(() => ({ dispose: vi.fn() })),
    onScroll: vi.fn(() => ({ dispose: vi.fn() })),
    onRender: vi.fn(() => ({ dispose: vi.fn() })),
    buffer: {
      active: {
        type: "normal",
        length: lines.length,
        baseY: 0,
        cursorY,
        viewportY: 0,
        getLine: (i: number) =>
          i >= 0 && i < lines.length
            ? { translateToString: () => lines[i] }
            : undefined,
      },
    },
  } as unknown as Terminal;
  return term;
}

// Richer fake for the OSC-777-marker and scrollback-cap tests: mutable lines,
// captured onWriteParsed/onScroll callbacks, OSC emission, and markers minted
// at a controllable line (real xterm anchors registerMarker(0) at the cursor).
function makeLiveTerm(initial: string[]) {
  const lines = [...initial];
  const writeCbs: (() => void)[] = [];
  const scrollCbs: (() => void)[] = [];
  const handlers = new Map<number, OscHandler>();
  let markerLine = 0;
  const term = {
    parser: {
      registerOscHandler(code: number, h: OscHandler) {
        handlers.set(code, h);
        return { dispose: () => handlers.delete(code) };
      },
    },
    registerMarker: () => ({
      line: markerLine,
      isDisposed: false,
      dispose: () => {},
    }),
    onWriteParsed: (cb: () => void) => {
      writeCbs.push(cb);
      return { dispose: () => {} };
    },
    onScroll: (cb: () => void) => {
      scrollCbs.push(cb);
      return { dispose: () => {} };
    },
    onRender: () => ({ dispose: () => {} }),
    buffer: {
      active: {
        type: "normal",
        baseY: 0,
        viewportY: 0,
        get length() {
          return lines.length;
        },
        get cursorY() {
          return lines.length; // past the end: no live-input-box trimming
        },
        getLine: (i: number) =>
          i >= 0 && i < lines.length
            ? { translateToString: () => lines[i] }
            : undefined,
      },
    },
  };
  return {
    term: term as unknown as Terminal,
    osc: (code: number, data: string) => void handlers.get(code)?.(data),
    turnAt(line: number) {
      markerLine = line;
      void handlers.get(777)?.("notify;Koden;working");
    },
    // Append at the scrollback cap: xterm trims the top so `length` stays
    // constant while content churns; write + scroll events still fire.
    appendAtCap(line: string) {
      lines.shift();
      lines.push(line);
      for (const cb of writeCbs) cb();
      for (const cb of scrollCbs) cb();
    },
  };
}

describe("parseExitCode", () => {
  it("treats an empty code (e.g. no payload) as null / success", () => {
    expect(parseExitCode("")).toBeNull();
  });

  it("parses a zero exit code as 0 (ok)", () => {
    expect(parseExitCode("0")).toBe(0);
  });

  it("parses a non-zero exit code (fail)", () => {
    expect(parseExitCode("1")).toBe(1);
    expect(parseExitCode("130")).toBe(130);
  });

  it("reads a leading integer out of a noisy OSC 133 D payload", () => {
    // The shell may append extra fields after the code (e.g. "0;...").
    expect(parseExitCode("0;something")).toBe(0);
    expect(parseExitCode("2;aborted")).toBe(2);
  });

  it("returns null for a non-numeric code", () => {
    expect(parseExitCode("abc")).toBeNull();
  });
});

describe("isTurnMarker", () => {
  it("fires only for the UserPromptSubmit `working` payload", () => {
    expect(isTurnMarker("notify;Koden;working")).toBe(true);
  });

  it("ignores other OSC 777 transitions (attention/finished) and noise", () => {
    expect(isTurnMarker("notify;Koden;attention")).toBe(false);
    expect(isTurnMarker("notify;Koden;finished")).toBe(false);
    expect(isTurnMarker("notify;Other;working")).toBe(false);
    expect(isTurnMarker("")).toBe(false);
    expect(isTurnMarker("working")).toBe(false);
  });
});

describe("extractTurnPrompt", () => {
  it("pulls the message out of a plain `>` prompt line", () => {
    expect(extractTurnPrompt("> hello there")).toBe("hello there");
  });

  it("handles claude's heavy caret glyph", () => {
    expect(extractTurnPrompt("❯ fix the build")).toBe("fix the build");
  });

  it("strips a leading box-drawing border before the glyph", () => {
    expect(extractTurnPrompt("│ > run the tests")).toBe("run the tests");
    expect(extractTurnPrompt("┃ ❯  deploy now ")).toBe("deploy now");
  });

  it("returns empty for non-prompt lines so the caller keeps scanning", () => {
    expect(extractTurnPrompt("────────────")).toBe("");
    expect(extractTurnPrompt("  some output text")).toBe("");
    expect(extractTurnPrompt(undefined)).toBe("");
    expect(extractTurnPrompt("")).toBe("");
  });

  it("skips the empty-input placeholder", () => {
    expect(extractTurnPrompt('> Try "edit the file"')).toBe("");
    expect(extractTurnPrompt(">")).toBe("");
    expect(extractTurnPrompt("> …")).toBe("");
  });
});

describe("CommandMarks.scanTurns", () => {
  it("emits a turn mark per rendered Claude prompt line, skipping noise", () => {
    const cm = new CommandMarks(
      makeFakeTerm([
        "● Reading file…", // output, not a prompt
        "> Hi", // prompt
        "────────────", // border
        "> Testing", // prompt
        "  some output", // output
      ]),
    );
    const turns = cm.scanTurns();
    expect(turns.map((t) => t.text)).toEqual(["Hi", "Testing"]);
    expect(turns.every((t) => t.status === "turn")).toBe(true);
    // Stable, line-addressed ids so the popover can key React rows on them.
    expect(turns[0].line).toBe(1);
    expect(turns[1].line).toBe(3);
    expect(turns[0].id).not.toBe(turns[1].id);
  });

  it("yields nothing for a buffer with no prompt lines (plain shell)", () => {
    const cm = new CommandMarks(
      makeFakeTerm(["$ ls -al", "file-a  file-b", "$ "]),
    );
    expect(cm.scanTurns()).toEqual([]);
  });

  it("dedupes consecutive duplicate prompts from a repainted box", () => {
    const cm = new CommandMarks(
      makeFakeTerm(["> deploy", "> deploy", "> done"]),
    );
    expect(cm.scanTurns().map((t) => t.text)).toEqual(["deploy", "done"]);
  });

  it("skips the live input box at/below the cursor (in-progress typing)", () => {
    // Last `> typing now` sits on the cursor row → it's the unsent draft.
    const cm = new CommandMarks(
      makeFakeTerm(["> first", "output", "> typing now"], 2),
    );
    expect(cm.scanTurns().map((t) => t.text)).toEqual(["first"]);
  });

  it("caches the scan until the buffer grows", () => {
    const term = makeFakeTerm(["> one"]);
    const cm = new CommandMarks(term);
    const a = cm.scanTurns();
    const b = cm.scanTurns();
    expect(b).toBe(a); // same array instance: cache hit, no re-walk
  });

  it("surfaces scanned turns through getMarks(), sorted by line", () => {
    const cm = new CommandMarks(makeFakeTerm(["$ build", "> ship it"]));
    const marks = cm.getMarks();
    // The scanned turn is present and tagged "turn" (no OSC 777 needed).
    expect(marks.some((m) => m.status === "turn" && m.text === "ship it")).toBe(
      true,
    );
    // Merged output stays ascending by buffer line.
    for (let i = 1; i < marks.length; i++) {
      expect(marks[i].line).toBeGreaterThanOrEqual(marks[i - 1].line);
    }
  });
});

describe("CommandMarks bus turns (getBusTurns)", () => {
  const withTurns = (lines: string[], turns: BusTurn[]) =>
    new CommandMarks(makeFakeTerm(lines), { getBusTurns: () => turns });

  it("mints a text-bearing turn row surfaced by getMarks()", () => {
    const turns: BusTurn[] = [];
    const cm = withTurns(["plain output"], turns);
    turns.push({ id: TURN_LINE_BASE + 1, text: "explain the build error" });
    const marks = cm.getMarks();
    expect(marks).toHaveLength(1);
    expect(marks[0].status).toBe("turn");
    expect(marks[0].text).toBe("explain the build error");
  });

  it("makes bus turns the only turn rows once they exist (no double-listing)", () => {
    // Buffer has a scrapeable `> ship it` prompt. Before any bus turn, the
    // scrape surfaces it; after a bus-delivered turn arrives, the real text
    // wins and the scrape must NOT also list anything.
    const turns: BusTurn[] = [];
    const cm = withTurns(["> ship it"], turns);
    expect(cm.getMarks().some((m) => m.text === "ship it")).toBe(true);
    turns.push({ id: TURN_LINE_BASE + 1, text: "the real prompt" });
    expect(cm.getMarks().map((m) => m.text)).toEqual(["the real prompt"]);
  });

  it("surfaces EVERY turn in arrival order, not just the first", () => {
    // Regression: turns anchored to registerMarker(0) vanished when a
    // repainting agent TUI drove marker lines to -1. Bus turns are marker-free
    // so all must survive.
    const turns: BusTurn[] = [
      { id: TURN_LINE_BASE + 1, text: "hi" },
      { id: TURN_LINE_BASE + 2, text: "What's up ?" },
      { id: TURN_LINE_BASE + 3, text: "505" },
    ];
    const cm = withTurns(["plain output"], turns);
    const rows = cm.getMarks().filter((m) => m.status === "turn");
    expect(rows.map((t) => t.text)).toEqual(["hi", "What's up ?", "505"]);
    expect(new Set(rows.map((t) => t.id)).size).toBe(3);
    expect(rows[0].line).toBeLessThan(rows[1].line);
    expect(rows[1].line).toBeLessThan(rows[2].line);
  });

  it("anchors a bus turn to a matching rendered line for click-to-scroll", () => {
    const turns: BusTurn[] = [
      { id: TURN_LINE_BASE + 1, text: "ship it" },
      { id: TURN_LINE_BASE + 2, text: "unrendered prompt" },
    ];
    const cm = withTurns(["output", "> ship it", "more output"], turns);
    const rows = cm.getMarks();
    expect(rows.map((m) => m.text)).toEqual(["ship it", "unrendered prompt"]);
    // Matched text: anchored to the real buffer line, not the synthetic band.
    expect(rows[0].line).toBe(1);
    // No rendered match: keeps its synthetic high-band line (sorts last).
    expect(rows[1].line).toBeGreaterThanOrEqual(TURN_LINE_BASE);
  });

  it("survives a CommandMarks dispose + reconstruct (renderer pool rebind)", () => {
    // Turn storage is session-lifetime (turnStore), injected via getBusTurns:
    // releasing and rebinding a slot builds a fresh CommandMarks over the same
    // store and the Inputs history must persist (it used to be wiped).
    const leafId = 987_654;
    addBusTurn(leafId, "before any bind");
    const first = new CommandMarks(makeFakeTerm(["x"]), {
      getBusTurns: () => busTurnsForLeaf(leafId),
    });
    expect(first.getMarks().map((m) => m.text)).toEqual(["before any bind"]);
    first.dispose();
    addBusTurn(leafId, "after rebind");
    const second = new CommandMarks(makeFakeTerm(["x"]), {
      getBusTurns: () => busTurnsForLeaf(leafId),
    });
    expect(second.getMarks().map((m) => m.text)).toEqual([
      "before any bind",
      "after rebind",
    ]);
    second.dispose();
  });
});

describe("CommandMarks partial OSC 777 emission (CC 2.1.206)", () => {
  it("merges scanned turns with marker turns instead of bailing on the first marker", () => {
    // CC 2.1.206 emits the UserPromptSubmit terminalSequence only while its
    // UI-gated emitter is registered: turns 1-2 minted OSC 777 markers, turns
    // 3-4 did not. The old all-or-nothing bail (any marker turn suppresses the
    // scrape) hid the unmarked turns; the merge must list all four exactly once.
    const t = makeLiveTerm([
      "> hi",
      "response a",
      "> 5+5",
      "response b",
      "> hiii",
      "response c",
      "> 30 countries list them",
      "done",
    ]);
    const cm = new CommandMarks(t.term);
    t.turnAt(0);
    t.turnAt(2);
    const turns = cm.getMarks().filter((m) => m.status === "turn");
    expect(turns.map((m) => m.text)).toEqual([
      "hi",
      "5+5",
      "hiii",
      "30 countries list them",
    ]);
  });

  it("prefers bus turns over the marker/scan union, deduped, anchored", () => {
    const t = makeLiveTerm([
      "> hi",
      "response a",
      "> 5+5",
      "response b",
      "> hiii",
      "response c",
      "> 30 countries list them",
      "done",
    ]);
    const turns: BusTurn[] = [
      { id: TURN_LINE_BASE + 1, text: "hi" },
      { id: TURN_LINE_BASE + 2, text: "5+5" },
      { id: TURN_LINE_BASE + 3, text: "hiii" },
      { id: TURN_LINE_BASE + 4, text: "30 countries list them" },
    ];
    const cm = new CommandMarks(t.term, { getBusTurns: () => turns });
    t.turnAt(0);
    t.turnAt(2);
    const rows = cm.getMarks().filter((m) => m.status === "turn");
    expect(rows.map((m) => m.text)).toEqual([
      "hi",
      "5+5",
      "hiii",
      "30 countries list them",
    ]);
    // Each anchored to its rendered prompt line.
    expect(rows.map((m) => m.line)).toEqual([0, 2, 4, 6]);
  });
});

describe("CommandMarks.scanTurns at the scrollback cap", () => {
  it("re-scans after writes that no longer grow buffer.length", () => {
    // At the scrollback cap xterm trims the top while `length` stays constant;
    // the old buf.length memo key froze the cache so new prompts never
    // surfaced. The dirty-counter key must pick up the churn.
    const t = makeLiveTerm(Array.from({ length: 200 }, (_, i) => `line ${i}`));
    const cm = new CommandMarks(t.term);
    expect(cm.scanTurns()).toEqual([]);
    t.appendAtCap("> late prompt");
    expect(cm.scanTurns().map((m) => m.text)).toEqual(["late prompt"]);
  });
});
