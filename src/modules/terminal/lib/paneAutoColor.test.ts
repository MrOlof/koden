import { describe, expect, it } from "vitest";
import { oklchToHex, paneColorAt, type PaletteName } from "./paneAutoColor";

const HEX = /^#[0-9a-f]{6}$/;
const PALETTES: PaletteName[] = ["muted", "vibrant", "pastel"];

// The dark surface a pane title actually renders on (--card in the .dark theme).
const CARD_BG = "#161b1d";

function channels(hex: string): [number, number, number] {
  return [
    parseInt(hex.slice(1, 3), 16) / 255,
    parseInt(hex.slice(3, 5), 16) / 255,
    parseInt(hex.slice(5, 7), 16) / 255,
  ];
}

// WCAG 2.x relative luminance + contrast ratio.
function luminance(hex: string): number {
  const lin = channels(hex).map((c) =>
    c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}

function contrast(a: string, b: string): number {
  const la = luminance(a);
  const lb = luminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

// Crude chroma proxy from sRGB (max-min), enough to separate palettes.
function chromaProxy(hex: string): number {
  const [r, g, b] = channels(hex);
  return Math.max(r, g, b) - Math.min(r, g, b);
}

describe("oklchToHex", () => {
  it("converts the theme card token to its known sRGB hex", () => {
    // oklch(0.218 0.008 223.9) is the .dark --card; must land on #161b1d.
    expect(oklchToHex(0.218, 0.008, 223.9)).toBe(CARD_BG);
  });

  it("gamut-maps out-of-gamut chroma instead of producing garbage", () => {
    // Absurd chroma in the cyan/blue zone must still yield a valid in-range hex.
    expect(oklchToHex(0.74, 0.5, 230)).toMatch(HEX);
  });
});

describe("paneAutoColor", () => {
  it("produces valid #rrggbb output across palettes", () => {
    for (const p of PALETTES) {
      for (let i = 0; i < 8; i++) expect(paneColorAt(p, i, 0)).toMatch(HEX);
    }
  });

  // The whole point of the OKLCH switch: EVERY generated color stays readable on
  // the dark card, including the blue/purple hues that failed under HSL. This is
  // the test that would have caught the unreadable purple title.
  it("every generated color clears WCAG 4.5:1 on the dark card, for all hues", () => {
    for (const p of PALETTES) {
      for (let seed = 0; seed < 360; seed += 23) {
        for (let i = 0; i < 4; i++) {
          const hex = paneColorAt(p, i, seed);
          expect(contrast(hex, CARD_BG)).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });

  it("vibrant carries clearly more chroma than muted", () => {
    const mutedAvg =
      [0, 1, 2, 3].reduce((s, i) => s + chromaProxy(paneColorAt("muted", i, 0)), 0) /
      4;
    const vibrantAvg =
      [0, 1, 2, 3].reduce(
        (s, i) => s + chromaProxy(paneColorAt("vibrant", i, 0)),
        0,
      ) / 4;
    expect(vibrantAvg).toBeGreaterThan(mutedAvg + 0.08);
  });

  it("consecutive colors are distinct (golden-angle spread, no clustering)", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 8; i++) seen.add(paneColorAt("vibrant", i, 12));
    expect(seen.size).toBe(8);
  });
});
