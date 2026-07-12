import { describe, expect, it } from "vitest";
import { createSilenceDetector, pickMime, rmsOf } from "./useVoiceCapture";
import { isChordRelease } from "./voiceChord";

const QUIET = 0.001;
const SPEECH = 0.05;

describe("createSilenceDetector", () => {
  const opts = { silenceMs: 1400, noSpeechMs: 8000, threshold: 0.01 };

  it("keeps going while speech continues", () => {
    const d = createSilenceDetector(opts);
    for (let t = 0; t <= 5000; t += 100) {
      expect(d.sample(SPEECH, t)).toBe("continue");
    }
  });

  it("stops after silenceMs of quiet following speech", () => {
    const d = createSilenceDetector(opts);
    expect(d.sample(SPEECH, 0)).toBe("continue");
    expect(d.sample(QUIET, 1000)).toBe("continue");
    expect(d.sample(QUIET, 1399)).toBe("continue");
    expect(d.sample(QUIET, 1400)).toBe("stop");
  });

  it("speech resets the silence window", () => {
    const d = createSilenceDetector(opts);
    d.sample(SPEECH, 0);
    d.sample(QUIET, 1300);
    d.sample(SPEECH, 1350); // spoke again just in time
    expect(d.sample(QUIET, 2700)).toBe("continue"); // only 1350ms quiet
    expect(d.sample(QUIET, 2750)).toBe("stop");
  });

  it("cancels when nothing was ever heard for noSpeechMs", () => {
    const d = createSilenceDetector(opts);
    expect(d.sample(QUIET, 0)).toBe("continue");
    expect(d.sample(QUIET, 7999)).toBe("continue");
    expect(d.sample(QUIET, 8000)).toBe("cancel");
  });

  it("never cancels once speech was heard", () => {
    const d = createSilenceDetector({ ...opts, silenceMs: 60_000 });
    d.sample(SPEECH, 0);
    expect(d.sample(QUIET, 30_000)).toBe("continue"); // long past noSpeechMs
  });

  it("treats threshold as speech (inclusive)", () => {
    const d = createSilenceDetector(opts);
    expect(d.sample(0.01, 0)).toBe("continue");
    // Speech was seen, so long quiet stops (transcribe) — never cancels.
    expect(d.sample(QUIET, 8000)).toBe("stop");
  });
});

describe("pickMime", () => {
  it("returns the first supported candidate", () => {
    expect(pickMime((m) => m === "audio/webm")).toBe("audio/webm");
    expect(pickMime(() => true)).toBe("audio/webm;codecs=opus");
  });

  it("returns undefined when nothing is supported", () => {
    expect(pickMime(() => false)).toBeUndefined();
  });
});

describe("rmsOf", () => {
  it("is 0 for empty and silent buffers", () => {
    expect(rmsOf(new Float32Array(0))).toBe(0);
    expect(rmsOf(new Float32Array(64))).toBe(0);
  });

  it("computes RMS of a constant signal", () => {
    expect(rmsOf(new Float32Array([0.5, -0.5, 0.5, -0.5]))).toBeCloseTo(0.5);
  });
});

describe("isChordRelease", () => {
  const bindings = [{ ctrl: true, shift: true, key: "m" }];

  it("matches the bound key going up", () => {
    expect(isChordRelease({ key: "m" }, bindings)).toBe(true);
    expect(isChordRelease({ key: "M" }, bindings)).toBe(true);
  });

  it("matches a bound modifier going up (macOS ⌘ keyup quirk)", () => {
    // macOS suppresses letter keyups while ⌘ is held — releasing the
    // modifier must count as releasing the chord.
    expect(
      isChordRelease({ key: "Meta" }, [{ meta: true, shift: true, key: "m" }]),
    ).toBe(true);
    expect(isChordRelease({ key: "Control" }, bindings)).toBe(true);
    expect(isChordRelease({ key: "Shift" }, bindings)).toBe(true);
  });

  it("ignores unbound keys and modifiers", () => {
    expect(isChordRelease({ key: "a" }, bindings)).toBe(false);
    expect(isChordRelease({ key: "Alt" }, bindings)).toBe(false);
    expect(isChordRelease({ key: "Meta" }, bindings)).toBe(false);
  });
});
