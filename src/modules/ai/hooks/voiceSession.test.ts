import { describe, expect, it } from "vitest";
import { chordReleaseKind, HOLD_TO_TALK_MS } from "./voiceChord";
import {
  escActionFor,
  type RearmInput,
  shouldRearmVoice,
} from "./voiceSession";

/** A turn just completed, session on, everything permissive. */
const TURN_DONE: RearmInput = {
  prevStatus: "streaming",
  status: "idle",
  sessionActive: true,
  handsFreeArmed: false,
  miniOpen: true,
  suspended: false,
  captureState: "idle",
  supported: true,
  hasKey: true,
  hasDraft: false,
  windowFocused: true,
};

describe("shouldRearmVoice", () => {
  it("re-arms for a session even with hands-free OFF (the new lane)", () => {
    expect(shouldRearmVoice(TURN_DONE)).toBe(true);
    expect(shouldRearmVoice({ ...TURN_DONE, prevStatus: "thinking" })).toBe(
      true,
    );
  });

  it("session lane ignores the suspend latch (Esc discards the take, not the loop)", () => {
    expect(shouldRearmVoice({ ...TURN_DONE, suspended: true })).toBe(true);
  });

  it("preserves the legacy hands-free lane exactly", () => {
    const legacy: RearmInput = {
      ...TURN_DONE,
      sessionActive: false,
      handsFreeArmed: true,
    };
    expect(shouldRearmVoice(legacy)).toBe(true);
    // The prior gates still hold for legacy users:
    expect(shouldRearmVoice({ ...legacy, suspended: true })).toBe(false);
    expect(shouldRearmVoice({ ...legacy, miniOpen: false })).toBe(false);
    expect(shouldRearmVoice({ ...legacy, handsFreeArmed: false })).toBe(false);
  });

  it("never re-arms with both lanes off", () => {
    expect(
      shouldRearmVoice({
        ...TURN_DONE,
        sessionActive: false,
        handsFreeArmed: false,
      }),
    ).toBe(false);
  });

  it("only fires on a turn-completion transition", () => {
    expect(shouldRearmVoice({ ...TURN_DONE, status: "thinking" })).toBe(false);
    expect(shouldRearmVoice({ ...TURN_DONE, status: "streaming" })).toBe(false);
    expect(
      shouldRearmVoice({ ...TURN_DONE, status: "awaiting-approval" }),
    ).toBe(false);
    // idle → idle (no turn ran) must not loop the mic:
    expect(shouldRearmVoice({ ...TURN_DONE, prevStatus: "idle" })).toBe(false);
    expect(shouldRearmVoice({ ...TURN_DONE, prevStatus: "error" })).toBe(false);
  });

  it("session lane still requires the Librarian window (close ends the session)", () => {
    expect(shouldRearmVoice({ ...TURN_DONE, miniOpen: false })).toBe(false);
  });

  it("stays out of the way of a keyboard draft", () => {
    expect(shouldRearmVoice({ ...TURN_DONE, hasDraft: true })).toBe(false);
  });

  it("never arms in the background or mid-capture or without a key", () => {
    expect(shouldRearmVoice({ ...TURN_DONE, windowFocused: false })).toBe(
      false,
    );
    expect(shouldRearmVoice({ ...TURN_DONE, captureState: "recording" })).toBe(
      false,
    );
    expect(
      shouldRearmVoice({ ...TURN_DONE, captureState: "transcribing" }),
    ).toBe(false);
    expect(shouldRearmVoice({ ...TURN_DONE, hasKey: false })).toBe(false);
    expect(shouldRearmVoice({ ...TURN_DONE, supported: false })).toBe(false);
  });
});

describe("escActionFor", () => {
  it("first Esc discards the live take — session or not", () => {
    expect(escActionFor({ capturing: true, sessionActive: true })).toBe(
      "cancel-capture",
    );
    expect(escActionFor({ capturing: true, sessionActive: false })).toBe(
      "cancel-capture",
    );
  });

  it("second Esc (or Esc between captures) ends the session", () => {
    expect(escActionFor({ capturing: false, sessionActive: true })).toBe(
      "end-session",
    );
  });

  it("falls through to the window close handler with neither", () => {
    expect(escActionFor({ capturing: false, sessionActive: false })).toBe(
      "none",
    );
  });
});

describe("chordReleaseKind", () => {
  it("a hold past the threshold is one push-to-talk take", () => {
    expect(chordReleaseKind(HOLD_TO_TALK_MS)).toBe("hold");
    expect(chordReleaseKind(HOLD_TO_TALK_MS + 500)).toBe("hold");
  });

  it("a quick tap arms the session", () => {
    expect(chordReleaseKind(0)).toBe("tap");
    expect(chordReleaseKind(HOLD_TO_TALK_MS - 1)).toBe("tap");
  });
});
