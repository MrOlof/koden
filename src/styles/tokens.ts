export type TerminalTokens = {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent: string;
  selection: string;
  selectionForeground: string;
  selectionInactive: string;
  ansiBlack: string;
  ansiRed: string;
  ansiGreen: string;
  ansiYellow: string;
  ansiBlue: string;
  ansiMagenta: string;
  ansiCyan: string;
  ansiWhite: string;
  ansiBrightBlack: string;
  ansiBrightRed: string;
  ansiBrightGreen: string;
  ansiBrightYellow: string;
  ansiBrightBlue: string;
  ansiBrightMagenta: string;
  ansiBrightCyan: string;
  ansiBrightWhite: string;
};

const VAR_BY_KEY: Record<keyof TerminalTokens, string> = {
  background: "--terminal-background",
  foreground: "--terminal-foreground",
  cursor: "--terminal-cursor",
  cursorAccent: "--terminal-cursor-accent",
  selection: "--terminal-selection",
  selectionForeground: "--terminal-selection-foreground",
  selectionInactive: "--terminal-selection-inactive",
  ansiBlack: "--terminal-ansi-black",
  ansiRed: "--terminal-ansi-red",
  ansiGreen: "--terminal-ansi-green",
  ansiYellow: "--terminal-ansi-yellow",
  ansiBlue: "--terminal-ansi-blue",
  ansiMagenta: "--terminal-ansi-magenta",
  ansiCyan: "--terminal-ansi-cyan",
  ansiWhite: "--terminal-ansi-white",
  ansiBrightBlack: "--terminal-ansi-bright-black",
  ansiBrightRed: "--terminal-ansi-bright-red",
  ansiBrightGreen: "--terminal-ansi-bright-green",
  ansiBrightYellow: "--terminal-ansi-bright-yellow",
  ansiBrightBlue: "--terminal-ansi-bright-blue",
  ansiBrightMagenta: "--terminal-ansi-bright-magenta",
  ansiBrightCyan: "--terminal-ansi-bright-cyan",
  ansiBrightWhite: "--terminal-ansi-bright-white",
};

const KEYS = Object.keys(VAR_BY_KEY) as (keyof TerminalTokens)[];

let probe: HTMLDivElement | null = null;

function getProbe(): HTMLDivElement {
  if (probe && probe.isConnected) return probe;
  const el = document.createElement("div");
  el.setAttribute("aria-hidden", "true");
  el.style.cssText =
    "position:absolute;visibility:hidden;pointer-events:none;contain:strict;width:0;height:0;";
  document.body.appendChild(el);
  probe = el;
  return el;
}

function resolve(el: HTMLDivElement, varName: string): string {
  el.style.color = `var(${varName})`;
  return getComputedStyle(el).color;
}

export function readTerminalTokens(): TerminalTokens {
  const el = getProbe();
  const out = {} as TerminalTokens;
  for (const k of KEYS) {
    out[k] = resolve(el, VAR_BY_KEY[k]);
  }
  return out;
}

function rgbToHex(css: string): string | null {
  const m = css.match(/(\d+(?:\.\d+)?)/g);
  if (!m || m.length < 3) return null;
  const h = (n: string) =>
    Math.max(0, Math.min(255, Math.round(Number(n))))
      .toString(16)
      .padStart(2, "0");
  return `#${h(m[0])}${h(m[1])}${h(m[2])}`;
}

// xterm decoration options (overviewRulerOptions.color, SearchAddon
// decorations) structurally require literal #RRGGBB and can't take a CSS
// var, so callers resolve a token (or a color-mix() expression built from
// tokens) to hex at apply-time instead of inlining a value that goes stale
// across theme switches. No-DOM guard so this stays a safe no-op in the
// (document-less) unit test environment.
export function resolveCssColorToHex(cssColor: string, fallback: string): string {
  if (typeof document === "undefined") return fallback;
  const el = getProbe();
  el.style.color = cssColor;
  return rgbToHex(getComputedStyle(el).color) ?? fallback;
}
