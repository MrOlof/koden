export type PaletteName = "muted" | "vibrant" | "pastel";

const GOLDEN_ANGLE = 137.508;

// OKLCH bands, not HSL. HSL lightness is a geometric channel midpoint with no
// model of human luminance, so a fixed HSL `l` makes blue/purple read far darker
// than yellow/green and fall below readable contrast on a dark bg. OKLCH `L` is
// perceptually uniform, so holding L constant per palette makes EVERY hue read
// equally bright — L is the contrast guarantee. Variety comes from hue (golden
// angle) + small chroma jitter; L barely moves. Bands target WCAG >= ~7:1 for
// every hue against the dark `--card` (#161b1d) the title sits on.
type OklchBand = {
  lBase: number;
  lJitter: number;
  cBase: number;
  cJitter: number;
};
const PALETTES: Record<PaletteName, OklchBand> = {
  // low chroma, quiet tints
  muted: { lBase: 0.72, lJitter: 0.02, cBase: 0.055, cJitter: 0.015 },
  // same readable L band, chroma pushed to the sRGB gamut edge (gamut-mapped)
  vibrant: { lBase: 0.74, lJitter: 0.02, cBase: 0.14, cJitter: 0.02 },
  // higher L + soft chroma: airy, the most contrast headroom
  pastel: { lBase: 0.84, lJitter: 0.02, cBase: 0.075, cJitter: 0.015 },
};

const EPS = 1e-4;

function linearToSrgb(c: number): number {
  return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
}

// OKLab(L,a,b) -> linear sRGB (may be out of [0,1]). Ottosson reference matrices.
function oklabToLinearSrgb(
  L: number,
  a: number,
  b: number,
): [number, number, number] {
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.291485548 * b;
  const l = l_ * l_ * l_;
  const m = m_ * m_ * m_;
  const s = s_ * s_ * s_;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

function oklchToLinearSrgb(
  L: number,
  C: number,
  hDeg: number,
): [number, number, number] {
  const h = (hDeg * Math.PI) / 180;
  return oklabToLinearSrgb(L, C * Math.cos(h), C * Math.sin(h));
}

function inGamut([r, g, b]: [number, number, number]): boolean {
  return (
    r >= -EPS &&
    r <= 1 + EPS &&
    g >= -EPS &&
    g <= 1 + EPS &&
    b >= -EPS &&
    b <= 1 + EPS
  );
}

// Hold L and H; binary-search the largest chroma that stays in sRGB. Reducing
// chroma (not clipping channels) preserves hue + lightness, so the contrast
// guarantee survives gamut mapping in the cyan/blue/violet zone (H~180-270).
function gamutMapChroma(
  L: number,
  C: number,
  hDeg: number,
): [number, number, number] {
  if (L <= 0) return [0, 0, 0];
  if (L >= 1) return [1, 1, 1];
  const direct = oklchToLinearSrgb(L, C, hDeg);
  if (inGamut(direct)) return direct;
  let lo = 0;
  let hi = C;
  for (let i = 0; i < 25; i++) {
    const mid = (lo + hi) / 2;
    if (inGamut(oklchToLinearSrgb(L, mid, hDeg))) lo = mid;
    else hi = mid;
  }
  return oklchToLinearSrgb(L, lo, hDeg);
}

/** OKLCH(L in 0..1, C >= 0, H in degrees) -> sRGB hex. Dependency-free. */
export function oklchToHex(L: number, C: number, hDeg: number): string {
  const lin = gamutMapChroma(L, C, hDeg);
  const enc = (v: number): string =>
    Math.round(linearToSrgb(Math.min(1, Math.max(0, v))) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${enc(lin[0])}${enc(lin[1])}${enc(lin[2])}`;
}

// Deterministic generator: the same (palette, index, seed) always yields the
// same hex, so tests can lock contrast + variety without touching module state.
// `seed` shifts the whole hue sequence per session.
export function paneColorAt(
  palette: PaletteName,
  index: number,
  seed: number,
): string {
  const { lBase, lJitter, cBase, cJitter } = PALETTES[palette];
  const hue = (seed + index * GOLDEN_ANGLE) % 360;
  // L wobbles only +/- a hair so it stays the contrast guarantee; the visible
  // variety lives in hue + chroma.
  const l = lBase + ((index % 3) - 1) * (lJitter / 2);
  const c = cBase + (index % 3) * (cJitter / 2);
  return oklchToHex(l, c, hue);
}

let index = 0;
const seed = Math.random() * 360;

export function nextPaneColor(palette: PaletteName): string {
  return paneColorAt(palette, index++, seed);
}
