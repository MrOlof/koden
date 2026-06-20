import { readTerminalTokens } from "@/styles/tokens";
import type { ITheme } from "@xterm/xterm";

export function buildTerminalTheme(): ITheme {
  const t = readTerminalTokens();
  return {
    background: t.background,
    foreground: t.foreground,
    cursor: t.cursor,
    cursorAccent: t.cursorAccent,
    // Strong, legible selection that matches the app chat's ::selection: a
    // near-white fill with dark glyphs, instead of the faint themed tint that
    // read as a dim grey block. t.foreground/t.background are already resolved
    // to rgb() by the token probe, so xterm parses them fine (color-mix() would
    // NOT parse here). Inactive (unfocused) selection stays the dimmer themed
    // value when present.
    selectionBackground: t.foreground,
    selectionForeground: t.background,
    ...(t.selectionInactive
      ? { selectionInactiveBackground: t.selectionInactive }
      : {}),
    black: t.ansiBlack,
    red: t.ansiRed,
    green: t.ansiGreen,
    yellow: t.ansiYellow,
    blue: t.ansiBlue,
    magenta: t.ansiMagenta,
    cyan: t.ansiCyan,
    white: t.ansiWhite,
    brightBlack: t.ansiBrightBlack,
    brightRed: t.ansiBrightRed,
    brightGreen: t.ansiBrightGreen,
    brightYellow: t.ansiBrightYellow,
    brightBlue: t.ansiBrightBlue,
    brightMagenta: t.ansiBrightMagenta,
    brightCyan: t.ansiBrightCyan,
    brightWhite: t.ansiBrightWhite,
  };
}
