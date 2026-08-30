import type { LayoutSplitKind, LayoutSplitSide } from "./context";

// Split-kind / direction normalization shared by the Librarian's layout tools
// (layout.ts) and the koden CLI bridge (modules/cli). No ai/zod imports here:
// the CLI bridge is in the eager graph (src/app/eager-budget.test.ts).

/** Pane kinds that can live in a split (recon truth: PaneTreeView's
 * SplitPaneType / PaneNode leaf `content`). Everything else is tab-only. */
export const SPLIT_KINDS = ["terminal", "note", "tasks"] as const;

/** Model-facing 'up'/'down' → the pane model's top/bottom SplitSide. */
export function sideForDirection(
  direction: "left" | "right" | "up" | "down",
): LayoutSplitSide {
  if (direction === "up") return "top";
  if (direction === "down") return "bottom";
  return direction;
}

/** Case-insensitive; accepts the plural alias 'notes'. Unknown kinds → null —
 * the caller must error listing SPLIT_KINDS, never silently substitute. */
export function normalizeSplitKind(kind: string): LayoutSplitKind | null {
  const k = kind.trim().toLowerCase();
  if (k === "notes") return "note";
  return (SPLIT_KINDS as readonly string[]).includes(k)
    ? (k as LayoutSplitKind)
    : null;
}
