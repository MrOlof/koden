import type { Tab } from "./useTabs";

/**
 * The label shown on a tab. Non-terminal tabs use their stored title; terminal
 * tabs prefer a user-set custom name, then fall back to the last segment of the
 * cwd. Keeping this pure makes the "custom name survives a cd" invariant
 * testable without rendering the bar.
 */
export function labelFor(t: Tab): string {
  // Terminal tabs prefer a user-set custom name, then the last cwd segment.
  if (t.kind === "terminal") {
    if (t.customTitle) return t.customTitle;
    if (!t.cwd) return t.title;
    const parts = t.cwd.split(/[\\/]/).filter(Boolean);
    return parts.length ? parts[parts.length - 1] : "/";
  }
  // Every other tab kind carries its own stored title.
  return t.title;
}
