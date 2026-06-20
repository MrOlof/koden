import { create } from "zustand";

export type PaneTitle = {
  label: string;
  /** Locked titles (e.g. the Director) can't be renamed by the user. */
  locked: boolean;
  /** Optional accent color (e.g. an agent role's color). */
  color?: string;
};

type PaneTitleState = {
  titles: Record<number, PaneTitle>;
  /** Set a pane's label, e.g. an agent name. locked = not user-renamable. */
  setPaneTitle: (
    leafId: number,
    label: string,
    locked?: boolean,
    color?: string,
  ) => void;
  /** User rename (no-op on locked panes). */
  renamePane: (leafId: number, label: string) => void;
  /** Override a pane's accent color (no-op on locked panes). */
  setPaneColor: (leafId: number, color: string) => void;
  clearPaneTitle: (leafId: number) => void;
};

export const usePaneTitleStore = create<PaneTitleState>((set) => ({
  titles: {},
  setPaneTitle: (leafId, label, locked = false, color) =>
    set((s) => ({
      titles: { ...s.titles, [leafId]: { label, locked, color } },
    })),
  renamePane: (leafId, label) =>
    set((s) => {
      const existing = s.titles[leafId];
      if (existing?.locked) return s;
      const trimmed = label.trim();
      if (!trimmed) {
        const next = { ...s.titles };
        delete next[leafId];
        return { titles: next };
      }
      // Keep the existing color (and any future fields); only the label changes.
      return {
        titles: {
          ...s.titles,
          [leafId]: { ...existing, label: trimmed, locked: false },
        },
      };
    }),
  setPaneColor: (leafId, color) =>
    set((s) => {
      const existing = s.titles[leafId];
      if (existing?.locked) return s;
      return {
        titles: {
          ...s.titles,
          [leafId]: {
            label: existing?.label ?? "",
            locked: false,
            color,
          },
        },
      };
    }),
  clearPaneTitle: (leafId) =>
    set((s) => {
      if (!s.titles[leafId]) return s;
      const next = { ...s.titles };
      delete next[leafId];
      return { titles: next };
    }),
}));

export function basenameOf(cwd?: string): string {
  if (!cwd) return "shell";
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "shell";
}
