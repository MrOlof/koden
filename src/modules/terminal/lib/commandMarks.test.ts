import type { Terminal } from "@xterm/xterm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  CommandMarks,
  extractTurnPrompt,
  isTurnMarker,
  parseExitCode,
} from "./commandMarks";

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

describe("CommandMarks.addTurn", () => {
  it("mints a text-bearing turn mark surfaced by getMarks()", () => {
    const cm = new CommandMarks(makeFakeTerm(["plain output"]));
    cm.addTurn("explain the build error");
    const marks = cm.getMarks();
    expect(marks).toHaveLength(1);
    expect(marks[0].status).toBe("turn");
    expect(marks[0].text).toBe("explain the build error");
  });

  it("suppresses the lossy scrollback scrape once a real turn mark exists", () => {
    // Buffer has a scrapeable `> ship it` prompt. Before any real turn, the
    // scrape surfaces it; after a bus-delivered turn arrives, the real text wins
    // and the scrape must NOT also list anything (no double-listing).
    const cm = new CommandMarks(makeFakeTerm(["> ship it"]));
    expect(cm.getMarks().some((m) => m.text === "ship it")).toBe(true);
    cm.addTurn("the real prompt");
    expect(cm.getMarks().map((m) => m.text)).toEqual(["the real prompt"]);
  });

  it("surfaces EVERY turn in arrival order, not just the first", () => {
    // Regression: the old impl anchored each turn to registerMarker(0); a
    // repainting agent TUI drove those marker lines to -1 so getMarks filtered
    // all but one out. Bus turns are marker-free now, so all must survive.
    const cm = new CommandMarks(makeFakeTerm(["plain output"]));
    cm.addTurn("hi");
    cm.addTurn("What's up ?");
    cm.addTurn("505");
    const turns = cm.getMarks().filter((m) => m.status === "turn");
    expect(turns.map((t) => t.text)).toEqual(["hi", "What's up ?", "505"]);
    // Distinct, stable ids for React keys; ascending so they sort in arrival order.
    expect(new Set(turns.map((t) => t.id)).size).toBe(3);
    expect(turns[0].line).toBeLessThan(turns[1].line);
    expect(turns[1].line).toBeLessThan(turns[2].line);
  });

  it("ignores empty/whitespace and caps very long prompts", () => {
    const cm = new CommandMarks(makeFakeTerm(["x"]));
    cm.addTurn("   ");
    expect(cm.getMarks()).toHaveLength(0);
    cm.addTurn("a".repeat(900));
    expect(cm.getMarks()[0].text).toHaveLength(400);
  });
});
