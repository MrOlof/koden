import { describe, expect, it } from "vitest";
import {
  chordPressAction,
  chordReleaseAction,
  chordReleaseKind,
  HOLD_TO_TALK_MS,
} from "./voiceChord";
import {
  escActionFor,
  miniCloseActionFor,
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

  it("session lane re-arms with the window CLOSED (headless voice, ADR-017)", () => {
    expect(shouldRearmVoice({ ...TURN_DONE, miniOpen: false })).toBe(true);
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

describe("miniCloseActionFor (transition-only — THE headless interaction)", () => {
  it("a headless take with the window simply CLOSED is untouched (no self-stop)", () => {
    // The effect can run while a take records and the window was never open
    // (mount, dep churn). prevOpen=false means no transition — never act.
    expect(
      miniCloseActionFor({
        prevOpen: false,
        open: false,
        sessionActive: false,
        recording: true,
      }),
    ).toBe("none");
  });

  it("a headless SESSION with the window closed keeps running", () => {
    expect(
      miniCloseActionFor({
        prevOpen: false,
        open: false,
        sessionActive: true,
        recording: false,
      }),
    ).toBe("none");
    expect(
      miniCloseActionFor({
        prevOpen: false,
        open: false,
        sessionActive: true,
        recording: true,
      }),
    ).toBe("none");
  });

  it("a genuine open → close ends a live session", () => {
    expect(
      miniCloseActionFor({
        prevOpen: true,
        open: false,
        sessionActive: true,
        recording: false,
      }),
    ).toBe("end-session");
    // Session outranks a live capture — same order as the effect always had.
    expect(
      miniCloseActionFor({
        prevOpen: true,
        open: false,
        sessionActive: true,
        recording: true,
      }),
    ).toBe("end-session");
  });

  it("a genuine open → close stops + DELIVERS a live non-session take", () => {
    expect(
      miniCloseActionFor({
        prevOpen: true,
        open: false,
        sessionActive: false,
        recording: true,
      }),
    ).toBe("deliver-take");
  });

  it("open → close with nothing live is inert", () => {
    expect(
      miniCloseActionFor({
        prevOpen: true,
        open: false,
        sessionActive: false,
        recording: false,
      }),
    ).toBe("none");
  });

  it("never acts while the window is (or just became) open", () => {
    expect(
      miniCloseActionFor({
        prevOpen: false,
        open: true,
        sessionActive: true,
        recording: true,
      }),
    ).toBe("none");
    expect(
      miniCloseActionFor({
        prevOpen: true,
        open: true,
        sessionActive: true,
        recording: true,
      }),
    ).toBe("none");
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

  it("a quicker release is a tap (one Wispr-style take)", () => {
    expect(chordReleaseKind(0)).toBe("tap");
    expect(chordReleaseKind(HOLD_TO_TALK_MS - 1)).toBe("tap");
  });
});

describe("chordPressAction", () => {
  const IDLE = { enabled: true, recording: false, transcribing: false };

  it("press while idle starts a manual take", () => {
    expect(chordPressAction(IDLE)).toBe("start-take");
  });

  it("press while THAT take records stops it (tap #2 = send)", () => {
    expect(chordPressAction({ ...IDLE, recording: true })).toBe("stop-capture");
  });

  it("press during a session-loop capture stops + delivers it too", () => {
    // Same seam on purpose: any live capture stops + submits on tap — the
    // session's post-turn re-arm (shouldRearmVoice) then resumes the loop.
    expect(chordPressAction({ ...IDLE, recording: true })).toBe("stop-capture");
  });

  it("ignores presses while disabled or transcribing", () => {
    expect(chordPressAction({ ...IDLE, enabled: false })).toBe("ignore");
    expect(chordPressAction({ ...IDLE, transcribing: true })).toBe("ignore");
    expect(
      chordPressAction({ ...IDLE, recording: true, transcribing: true }),
    ).toBe("ignore");
  });
});

describe("chordReleaseAction", () => {
  it("hold release stops + transcribes (classic PTT, unchanged)", () => {
    expect(
      chordReleaseAction({
        started: true,
        heldMs: HOLD_TO_TALK_MS,
        recording: true,
      }),
    ).toBe("stop-capture");
    expect(
      chordReleaseAction({
        started: true,
        heldMs: HOLD_TO_TALK_MS + 5000,
        recording: true,
      }),
    ).toBe("stop-capture");
  });

  it("hold release is a no-op when the capture already ended", () => {
    expect(
      chordReleaseAction({
        started: true,
        heldMs: HOLD_TO_TALK_MS + 500,
        recording: false,
      }),
    ).toBe("none");
  });

  it("tap release leaves the manual take recording (no session arming)", () => {
    expect(
      chordReleaseAction({ started: true, heldMs: 80, recording: true }),
    ).toBe("none");
  });

  it("release of a stop-press is inert — quick or held", () => {
    expect(
      chordReleaseAction({ started: false, heldMs: 80, recording: false }),
    ).toBe("none");
    expect(
      chordReleaseAction({ started: false, heldMs: 900, recording: true }),
    ).toBe("none");
  });

  it("tap flow end-to-end: idle → manual take → still recording → send", () => {
    // Tap 1 down: idle → start the take.
    expect(
      chordPressAction({ enabled: true, recording: false, transcribing: false }),
    ).toBe("start-take");
    // Tap 1 up (quick): the take keeps recording — pause as long as you like.
    expect(
      chordReleaseAction({ started: true, heldMs: 120, recording: true }),
    ).toBe("none");
    // Tap 2 down: stop + transcribe + submit.
    expect(
      chordPressAction({ enabled: true, recording: true, transcribing: false }),
    ).toBe("stop-capture");
    // Tap 2 up: inert.
    expect(
      chordReleaseAction({ started: false, heldMs: 120, recording: false }),
    ).toBe("none");
  });
});
