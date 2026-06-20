import { describe, expect, it } from "vitest";

import {
  isTerminalShiftEnter,
  terminalClipboardAction,
  terminalDeleteSequence,
  terminalLineNavigationSequence,
  terminalWordNavigationSequence,
  type TerminalKeyEvent,
} from "./keymap";

const evt = (partial: Partial<TerminalKeyEvent>): TerminalKeyEvent => ({
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  key: "",
  code: "",
  ...partial,
});

describe("terminalWordNavigationSequence", () => {
  it("maps Option+Left to readline word-left", () => {
    expect(
      terminalWordNavigationSequence(
        evt({ altKey: true, key: "ArrowLeft", code: "ArrowLeft" }),
      ),
    ).toBe("\x1bb");
  });

  it("maps Option+Right to readline word-right", () => {
    expect(
      terminalWordNavigationSequence(
        evt({ altKey: true, key: "ArrowRight", code: "ArrowRight" }),
      ),
    ).toBe("\x1bf");
  });

  it("does not remap plain arrows", () => {
    expect(
      terminalWordNavigationSequence(
        evt({ key: "ArrowLeft", code: "ArrowLeft" }),
      ),
    ).toBeNull();
  });
});

describe("terminalLineNavigationSequence", () => {
  it("maps Cmd+Left to readline line-start on macOS", () => {
    expect(
      terminalLineNavigationSequence(
        evt({ metaKey: true, key: "ArrowLeft", code: "ArrowLeft" }),
        { isMac: true },
      ),
    ).toBe("\x01");
  });

  it("maps Cmd+Right to readline line-end on macOS", () => {
    expect(
      terminalLineNavigationSequence(
        evt({ metaKey: true, key: "ArrowRight", code: "ArrowRight" }),
        { isMac: true },
      ),
    ).toBe("\x05");
  });

  it("does not remap Cmd+Arrow off macOS", () => {
    expect(
      terminalLineNavigationSequence(
        evt({ metaKey: true, key: "ArrowLeft", code: "ArrowLeft" }),
        { isMac: false },
      ),
    ).toBeNull();
  });

  it("does not remap Cmd+Option+Arrow (selection-style combos pass through)", () => {
    expect(
      terminalLineNavigationSequence(
        evt({ metaKey: true, altKey: true, key: "ArrowLeft", code: "ArrowLeft" }),
        { isMac: true },
      ),
    ).toBeNull();
  });
});

describe("terminalDeleteSequence", () => {
  it("maps Cmd+Backspace to kill-to-line-start on macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ metaKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: true },
      ),
    ).toBe("\x15");
  });

  it("maps Option+Backspace to kill-word-backward on macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ altKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: true },
      ),
    ).toBe("\x17");
  });

  it("maps Ctrl+Backspace to kill-word-backward off macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ ctrlKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: false },
      ),
    ).toBe("\x17");
  });

  it("does not remap Ctrl+Backspace on macOS (reserved for native readline binding)", () => {
    expect(
      terminalDeleteSequence(
        evt({ ctrlKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: true },
      ),
    ).toBeNull();
  });

  it("does not remap Cmd+Backspace off macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ metaKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: false },
      ),
    ).toBeNull();
  });

  it("does not remap plain Backspace", () => {
    expect(
      terminalDeleteSequence(
        evt({ key: "Backspace", code: "Backspace" }),
        { isMac: true },
      ),
    ).toBeNull();
  });
});

describe("terminalClipboardAction", () => {
  const opts = (hasSelection: boolean, isMac = false) => ({
    isMac,
    hasSelection,
  });

  it("copies on Ctrl+C when text is selected", () => {
    expect(
      terminalClipboardAction(evt({ ctrlKey: true, code: "KeyC" }), opts(true)),
    ).toBe("copy");
  });

  it("falls through on Ctrl+C with no selection so SIGINT is sent", () => {
    expect(
      terminalClipboardAction(evt({ ctrlKey: true, code: "KeyC" }), opts(false)),
    ).toBeNull();
  });

  it("cuts on Ctrl+X when text is selected", () => {
    expect(
      terminalClipboardAction(evt({ ctrlKey: true, code: "KeyX" }), opts(true)),
    ).toBe("cut");
  });

  it("falls through on Ctrl+X with no selection", () => {
    expect(
      terminalClipboardAction(evt({ ctrlKey: true, code: "KeyX" }), opts(false)),
    ).toBeNull();
  });

  it("always pastes on Ctrl+V regardless of selection", () => {
    expect(
      terminalClipboardAction(evt({ ctrlKey: true, code: "KeyV" }), opts(false)),
    ).toBe("paste");
  });

  it("keeps Ctrl+Shift+C as explicit copy", () => {
    expect(
      terminalClipboardAction(
        evt({ ctrlKey: true, shiftKey: true, code: "KeyC" }),
        opts(true),
      ),
    ).toBe("copy");
  });

  it("keeps Ctrl+Shift+V as explicit paste", () => {
    expect(
      terminalClipboardAction(
        evt({ ctrlKey: true, shiftKey: true, code: "KeyV" }),
        opts(false),
      ),
    ).toBe("paste");
  });

  it("never intercepts Ctrl on macOS (Cmd is the clipboard there)", () => {
    expect(
      terminalClipboardAction(
        evt({ ctrlKey: true, code: "KeyC" }),
        opts(true, true),
      ),
    ).toBeNull();
    expect(
      terminalClipboardAction(
        evt({ ctrlKey: true, code: "KeyV" }),
        opts(true, true),
      ),
    ).toBeNull();
  });

  it("ignores Ctrl+Alt combos (AltGr) so they reach the shell", () => {
    expect(
      terminalClipboardAction(
        evt({ ctrlKey: true, altKey: true, code: "KeyV" }),
        opts(true),
      ),
    ).toBeNull();
  });
});

describe("isTerminalShiftEnter", () => {
  it("matches plain Shift+Enter", () => {
    expect(isTerminalShiftEnter(evt({ key: "Enter", shiftKey: true }))).toBe(
      true,
    );
  });

  it("does not match Enter without Shift", () => {
    expect(isTerminalShiftEnter(evt({ key: "Enter" }))).toBe(false);
  });

  it("does not match Ctrl+Shift+Enter", () => {
    expect(
      isTerminalShiftEnter(evt({ key: "Enter", shiftKey: true, ctrlKey: true })),
    ).toBe(false);
  });
});
