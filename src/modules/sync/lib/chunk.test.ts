import { describe, expect, it } from "vitest";
import {
  chunkEnvelope,
  decodeEnvelope,
  encodeEnvelope,
  joinParts,
  PART_DATA_CHARS,
} from "./chunk";

describe("encode/decode", () => {
  it("round-trips unicode and control characters", () => {
    const value = { note: 'åäö 🌱 tab\there "quotes" \\slash\n', n: 42 };
    expect(decodeEnvelope(encodeEnvelope(value))).toEqual(value);
  });
});

describe("chunkEnvelope + joinParts", () => {
  it("round-trips a small envelope in one part", () => {
    const { index, parts } = chunkEnvelope({ a: 1 }, 111, "dev-x");
    expect(parts).toHaveLength(1);
    expect(index.of).toBe(1);
    expect(joinParts(index, parts)).toEqual({ a: 1 });
  });

  it("round-trips a multi-part envelope and tolerates shuffled parts", () => {
    const big = { text: "x".repeat(PART_DATA_CHARS * 3) };
    const { index, parts } = chunkEnvelope(big, 222, "dev-x");
    expect(parts.length).toBeGreaterThan(2);
    const shuffled = [...parts].reverse();
    expect(joinParts(index, shuffled)).toEqual(big);
  });

  it("rejects a gen mix (racing writer)", () => {
    const { index, parts } = chunkEnvelope(
      { text: "y".repeat(PART_DATA_CHARS * 2) },
      333,
      "dev-x",
    );
    const stale = parts.map((p) => (p.part === 1 ? { ...p, gen: 999 } : p));
    expect(joinParts(index, stale)).toBeNull();
  });

  it("rejects a missing part and a tampered payload", () => {
    const { index, parts } = chunkEnvelope(
      { text: "z".repeat(PART_DATA_CHARS * 2) },
      444,
      "dev-x",
    );
    expect(joinParts(index, parts.slice(1))).toBeNull();
    const tampered = parts.map((p, i) =>
      i === 0 ? { ...p, data: `AA${p.data.slice(2)}` } : p,
    );
    expect(joinParts(index, tampered)).toBeNull();
  });

  it("handles an empty envelope", () => {
    const { index, parts } = chunkEnvelope("", 1, "dev-x");
    expect(parts).toHaveLength(1);
    expect(joinParts(index, parts)).toBe("");
  });
});
