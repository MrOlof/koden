import type { Theme } from "../types";

// Bespoke "Svart" ANSI 16 — the signature terminal palette. High contrast on
// true black, restrained saturation, green family = spruce. Source of truth for
// the koden-default look; mirrors globals.css :root/.dark tokens so the theme is
// self-describing (a .koden-theme export must reproduce the look).
const SVART_ANSI_DARK = [
  "#1a1c1b", "#d97a72", "#7fae8f", "#cfa964",
  "#7d9fc7", "#ab8fb5", "#82b8b4", "#d6d4cc",
  "#4a4e4b", "#e8938c", "#9ac4a8", "#e0bd7e",
  "#99b5d6", "#c0a8c9", "#9cccc8", "#f0efe9",
] as const;

// Light port: same hues, darkened until legible (WCAG-ish ≥ 4.5) on #f2f1ec.
const SVART_ANSI_LIGHT = [
  "#2b2d2c", "#a33f38", "#3f6d55", "#8a6a1f",
  "#3a5f8c", "#6f4f7a", "#356e6a", "#5c5e58",
  "#676a61", "#ad4c45", "#457560", "#806326",
  "#4a6f9c", "#7f5f8a", "#3a716d", "#2b2d2c",
] as const;

export const kodenDefault: Theme = {
  id: "koden-default",
  name: "Koden Svart",
  description:
    "Near-black monochrome canvas with a single muted spruce wire — the code, uninterrupted.",
  editorTheme: { dark: "atomone", light: "atomone" },
  variants: {
    dark: {
      colors: {
        background: "#0a0b0b",
        foreground: "#ededec",
        card: "#121313",
        cardForeground: "#ededec",
        popover: "#101111",
        popoverForeground: "#ededec",
        primary: "#5b8a6f",
        primaryForeground: "#0a0b0b",
        secondary: "#1a1b1b",
        secondaryForeground: "#ededec",
        muted: "#171818",
        mutedForeground: "#7d817f",
        accent: "#1c1e1d",
        accentForeground: "#ededec",
        destructive: "#e5706b",
        border: "rgba(237,237,236,0.12)",
        input: "rgba(237,237,236,0.16)",
        ring: "#5b8a6f",
        sidebar: "#0d0e0e",
        sidebarForeground: "#ededec",
        sidebarPrimary: "#5b8a6f",
        sidebarPrimaryForeground: "#0a0b0b",
        sidebarAccent: "#1c1e1d",
        sidebarAccentForeground: "#ededec",
        sidebarBorder: "rgba(237,237,236,0.12)",
        sidebarRing: "#5b8a6f",
        radius: "0.25rem",
      },
      terminal: {
        background: "#0a0b0b",
        foreground: "#ededec",
        cursor: "#5b8a6f",
        cursorAccent: "#0a0b0b",
        selection: "rgba(91,138,111,0.28)",
        selectionInactive: "rgba(91,138,111,0.18)",
        ansi: SVART_ANSI_DARK,
      },
    },
    light: {
      colors: {
        background: "#f2f1ec",
        foreground: "#17191a",
        card: "#faf9f4",
        cardForeground: "#17191a",
        popover: "#faf9f4",
        popoverForeground: "#17191a",
        primary: "#4f7d68",
        primaryForeground: "#f2f1ec",
        secondary: "#e7e5dd",
        secondaryForeground: "#17191a",
        muted: "#e7e5dd",
        mutedForeground: "#63655f",
        accent: "#e7e5dd",
        accentForeground: "#17191a",
        destructive: "#b8443e",
        border: "rgba(23,25,26,0.12)",
        input: "rgba(23,25,26,0.16)",
        ring: "#4f7d68",
        sidebar: "#eceae2",
        sidebarForeground: "#17191a",
        sidebarPrimary: "#4f7d68",
        sidebarPrimaryForeground: "#f2f1ec",
        sidebarAccent: "#e7e5dd",
        sidebarAccentForeground: "#17191a",
        sidebarBorder: "rgba(23,25,26,0.12)",
        sidebarRing: "#4f7d68",
        radius: "0.25rem",
      },
      terminal: {
        background: "#f2f1ec",
        foreground: "#17191a",
        cursor: "#4f7d68",
        cursorAccent: "#f2f1ec",
        selection: "rgba(79,125,104,0.28)",
        selectionInactive: "rgba(79,125,104,0.18)",
        ansi: SVART_ANSI_LIGHT,
      },
    },
  },
};
