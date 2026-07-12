import { describe, expect, it } from "vitest";
import {
  hudPhaseFor,
  VOICE_REPLY_PREVIEW_MAX,
  voiceReplyPreview,
} from "./voiceHud";

describe("voiceReplyPreview (reply-toast gate)", () => {
  const assistant = (parts: Array<{ type: string; text?: string }>) => ({
    role: "assistant",
    parts,
  });

  it("substantive prose gets a preview", () => {
    expect(
      voiceReplyPreview(assistant([{ type: "text", text: "Two tests fail." }])),
    ).toBe("Two tests fail.");
  });

  it("action-only turns (tool chatter, no prose) get NO toast", () => {
    expect(
      voiceReplyPreview(
        assistant([
          { type: "step-start" },
          { type: "tool-run_command" },
          { type: "tool-read_file" },
        ]),
      ),
    ).toBeNull();
  });

  it("whitespace-only text is not substantive", () => {
    expect(
      voiceReplyPreview(assistant([{ type: "text", text: "  \n\t " }])),
    ).toBeNull();
  });

  it("reasoning parts are chatter, not prose", () => {
    expect(
      voiceReplyPreview(
        assistant([{ type: "reasoning", text: "thinking about it" }]),
      ),
    ).toBeNull();
  });

  it("joins multiple text parts and collapses whitespace to one line", () => {
    expect(
      voiceReplyPreview(
        assistant([
          { type: "text", text: "First.\n\nSecond   line." },
          { type: "tool-fs_grep" },
          { type: "text", text: "Third." },
        ]),
      ),
    ).toBe("First. Second line. Third.");
  });

  it("truncates to ~140 chars with an ellipsis", () => {
    const long = "x".repeat(400);
    const p =
      voiceReplyPreview(assistant([{ type: "text", text: long }])) ?? "";
    expect(p).not.toBe("");
    expect(p.length).toBeLessThanOrEqual(VOICE_REPLY_PREVIEW_MAX);
    expect(p.endsWith("…")).toBe(true);
  });

  it("short replies are untouched (no ellipsis)", () => {
    const exact = "y".repeat(VOICE_REPLY_PREVIEW_MAX);
    expect(voiceReplyPreview(assistant([{ type: "text", text: exact }]))).toBe(
      exact,
    );
  });

  it("ignores non-assistant or missing messages", () => {
    expect(
      voiceReplyPreview({
        role: "user",
        parts: [{ type: "text", text: "hello" }],
      }),
    ).toBeNull();
    expect(voiceReplyPreview(undefined)).toBeNull();
    expect(voiceReplyPreview(null)).toBeNull();
  });
});

describe("hudPhaseFor", () => {
  it("a live capture always wins: recording → listening, then transcribing", () => {
    expect(
      hudPhaseFor({
        captureState: "recording",
        voiceTurnActive: false,
        agentStatus: "idle",
      }),
    ).toBe("listening");
    expect(
      hudPhaseFor({
        captureState: "transcribing",
        voiceTurnActive: true,
        agentStatus: "thinking",
      }),
    ).toBe("transcribing");
  });

  it("a running VOICE turn shows working — including awaiting-approval", () => {
    for (const agentStatus of [
      "thinking",
      "streaming",
      "awaiting-approval",
    ] as const) {
      expect(
        hudPhaseFor({ captureState: "idle", voiceTurnActive: true, agentStatus }),
      ).toBe("working");
    }
  });

  it("typed turns never show working", () => {
    expect(
      hudPhaseFor({
        captureState: "idle",
        voiceTurnActive: false,
        agentStatus: "streaming",
      }),
    ).toBe("hidden");
  });

  it("settled or idle voice state hides the pill", () => {
    expect(
      hudPhaseFor({
        captureState: "idle",
        voiceTurnActive: true,
        agentStatus: "idle",
      }),
    ).toBe("hidden");
    expect(
      hudPhaseFor({
        captureState: "idle",
        voiceTurnActive: true,
        agentStatus: "error",
      }),
    ).toBe("hidden");
    expect(
      hudPhaseFor({
        captureState: "idle",
        voiceTurnActive: false,
        agentStatus: "idle",
      }),
    ).toBe("hidden");
  });
});
