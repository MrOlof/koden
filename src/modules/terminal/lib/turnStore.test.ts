import { describe, expect, it } from "vitest";
import {
  addBusTurn,
  busTurnsForLeaf,
  clearBusTurns,
  TURN_LINE_BASE,
} from "./turnStore";

// Module-scope store: each test uses its own leaf id band to stay isolated.

describe("turnStore", () => {
  it("stores turns for a leaf that has no bound slot yet", () => {
    const leafId = 100_001;
    expect(addBusTurn(leafId, "hello")).toBe(true);
    expect(busTurnsForLeaf(leafId).map((t) => t.text)).toEqual(["hello"]);
  });

  it("keeps arrival order with distinct high-band ids", () => {
    const leafId = 100_002;
    addBusTurn(leafId, "one");
    addBusTurn(leafId, "two");
    const turns = busTurnsForLeaf(leafId);
    expect(turns.map((t) => t.text)).toEqual(["one", "two"]);
    expect(turns[0].id).toBeGreaterThanOrEqual(TURN_LINE_BASE);
    expect(turns[1].id).toBeGreaterThan(turns[0].id);
  });

  it("rejects empty/whitespace and trims very long prompts", () => {
    const leafId = 100_003;
    expect(addBusTurn(leafId, "   ")).toBe(false);
    expect(busTurnsForLeaf(leafId)).toHaveLength(0);
    addBusTurn(leafId, "a".repeat(900));
    expect(busTurnsForLeaf(leafId)[0].text).toHaveLength(400);
  });

  it("caps retained turns per leaf (oldest shifted out)", () => {
    const leafId = 100_004;
    for (let i = 0; i < 510; i++) addBusTurn(leafId, `turn ${i}`);
    const turns = busTurnsForLeaf(leafId);
    expect(turns).toHaveLength(500);
    expect(turns[0].text).toBe("turn 10");
    expect(turns[499].text).toBe("turn 509");
  });

  it("clears on session dispose, isolated per leaf", () => {
    const a = 100_005;
    const b = 100_006;
    addBusTurn(a, "keep");
    addBusTurn(b, "drop");
    clearBusTurns(b);
    expect(busTurnsForLeaf(a).map((t) => t.text)).toEqual(["keep"]);
    expect(busTurnsForLeaf(b)).toHaveLength(0);
  });
});
