import {
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { PanelImperativeHandle } from "react-resizable-panels";
import type { LayoutMode } from "@/modules/tabs/lib/useLayoutMode";
import type { SidebarViewId } from "./types";

export const SIDEBAR_DEFAULT_WIDTH = 260;
export const SIDEBAR_MIN_WIDTH = 220;
// Effectively no cap — the user sets the width. react-resizable-panels bounds
// the sidebar by the sibling panels' minSize (i.e. the window), not this number,
// so a huge value just means "drag it as wide as you want".
export const SIDEBAR_MAX_WIDTH = 100000;
const SIDEBAR_WIDTH_STORAGE_KEY = "koden.sidebar.width";
const SIDEBAR_VIEW_STORAGE_KEY = "koden.sidebar.view";
// In sidebar layout mode the primary sidebar is the on-demand Files / Source
// Control column, collapsed by default so the always-on Tabs+Agents sidebar is
// the only thing showing on a fresh start. Existing users (no flag yet) also
// start collapsed because the unset value reads as `true`.
const SIDEBAR_COLLAPSED_STORAGE_KEY = "koden.sidebar.collapsed";

function clampSidebarWidth(width: number): number {
  return Math.min(
    SIDEBAR_MAX_WIDTH,
    Math.max(SIDEBAR_MIN_WIDTH, Math.round(width)),
  );
}

function readSidebarWidth(): number {
  try {
    const stored = window.localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY);
    const parsed = stored ? Number.parseInt(stored, 10) : NaN;
    return Number.isFinite(parsed)
      ? clampSidebarWidth(parsed)
      : SIDEBAR_DEFAULT_WIDTH;
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

function readSidebarView(): SidebarViewId {
  try {
    const stored = window.localStorage.getItem(SIDEBAR_VIEW_STORAGE_KEY);
    if (
      stored === "explorer" ||
      stored === "source-control" ||
      stored === "agents"
    )
      return stored;
  } catch {
    // ignore
  }
  return "explorer";
}

function readSidebarCollapsed(): boolean {
  try {
    // Only "false" keeps the primary sidebar open; anything else (including the
    // unset flag for existing users) defaults to collapsed.
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY) !== "false";
  } catch {
    return true;
  }
}

/**
 * In sidebar layout mode the primary sidebar only ever shows Files or Source
 * Control — Agents live in the dedicated Tabs+Agents column. Coerce a persisted
 * "agents" view to "explorer" so users whose localStorage still says "agents"
 * self-correct to Files instead of duplicating the Agents panel.
 */
export function effectiveSidebarView(
  view: SidebarViewId,
  layoutMode: LayoutMode,
): SidebarViewId {
  if (layoutMode === "sidebar" && view === "agents") return "explorer";
  return view;
}

type FocusableExplorer = {
  focus: () => void;
  isFocused: () => boolean;
};

export function useSidebarPanel(
  explorerRef: RefObject<FocusableExplorer | null>,
  layoutMode: LayoutMode,
) {
  const sidebarRef = useRef<PanelImperativeHandle | null>(null);
  const sidebarWidthRef = useRef(readSidebarWidth());
  const sidebarWidthWriteTimerRef = useRef(0);
  const explorerReturnFocusRef = useRef<HTMLElement | null>(null);
  const [sidebarView, setSidebarViewState] =
    useState<SidebarViewId>(readSidebarView);

  const persistSidebarCollapsed = useCallback((collapsed: boolean) => {
    try {
      window.localStorage.setItem(
        SIDEBAR_COLLAPSED_STORAGE_KEY,
        collapsed ? "true" : "false",
      );
    } catch {
      // storage may fail in private mode
    }
  }, []);

  // On a fresh start in sidebar mode, collapse the primary (Files / Source
  // Control) column so the always-on Tabs+Agents sidebar is the only thing
  // showing. Runs once on mount; user-driven expand/collapse afterwards
  // persists via persistSidebarCollapsed and is respected here. We snapshot
  // layoutMode into a ref so this stays genuinely mount-only — re-running on a
  // layoutMode change would fight the user's later expand/collapse, which is
  // owned by toggleSidebar / cycleSidebarView (those persist the flag).
  const initialLayoutModeRef = useRef(layoutMode);
  useLayoutEffect(() => {
    if (initialLayoutModeRef.current !== "sidebar") return;
    if (!readSidebarCollapsed()) return;
    const panel = sidebarRef.current;
    if (panel && panel.getSize().asPercentage > 0) panel.collapse();
  }, []);

  const persistSidebarView = useCallback((view: SidebarViewId) => {
    setSidebarViewState(view);
    try {
      window.localStorage.setItem(SIDEBAR_VIEW_STORAGE_KEY, view);
    } catch {
      // storage may fail in private mode
    }
  }, []);

  const toggleSidebar = useCallback(() => {
    const p = sidebarRef.current;
    if (!p) return;
    if (p.getSize().asPercentage <= 0) {
      p.expand();
      persistSidebarCollapsed(false);
    } else {
      p.collapse();
      persistSidebarCollapsed(true);
    }
  }, [persistSidebarCollapsed]);

  const cycleSidebarView = useCallback(
    (view: SidebarViewId) => {
      // In sidebar mode Agents has no rail item; coerce defensively so a stray
      // "agents" request lands on Files instead of an empty/duplicate view.
      const target = effectiveSidebarView(view, layoutMode);
      const current = effectiveSidebarView(sidebarView, layoutMode);
      const panel = sidebarRef.current;
      const collapsed = panel ? panel.getSize().asPercentage <= 0 : false;
      if (collapsed) {
        // Expand to the requested view (the on-demand Files / Source Control).
        if (panel) panel.resize(`${sidebarWidthRef.current}px`);
        persistSidebarCollapsed(false);
        if (target !== sidebarView) persistSidebarView(target);
        return;
      }
      if (target === current) {
        // Clicking the active rail item again collapses the column away.
        panel?.collapse();
        persistSidebarCollapsed(true);
        return;
      }
      persistSidebarView(target);
    },
    [persistSidebarView, persistSidebarCollapsed, sidebarView, layoutMode],
  );

  const persistSidebarWidth = useCallback((next: number) => {
    sidebarWidthRef.current = next;
    if (sidebarWidthWriteTimerRef.current) {
      window.clearTimeout(sidebarWidthWriteTimerRef.current);
    }
    sidebarWidthWriteTimerRef.current = window.setTimeout(() => {
      sidebarWidthWriteTimerRef.current = 0;
      try {
        window.localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(next));
      } catch {
        // ignore
      }
    }, 200);
  }, []);

  useEffect(() => {
    return () => {
      if (sidebarWidthWriteTimerRef.current) {
        window.clearTimeout(sidebarWidthWriteTimerRef.current);
      }
    };
  }, []);

  const toggleExplorerFocus = useCallback(() => {
    const explorer = explorerRef.current;
    const panel = sidebarRef.current;
    const collapsed = panel ? panel.getSize().asPercentage <= 0 : false;
    if (sidebarView !== "explorer" || collapsed) {
      if (panel && collapsed) {
        panel.resize(`${sidebarWidthRef.current}px`);
        persistSidebarCollapsed(false);
      }
      if (sidebarView !== "explorer") persistSidebarView("explorer");
      const active = document.activeElement;
      explorerReturnFocusRef.current =
        active instanceof HTMLElement && active !== document.body
          ? active
          : null;
      requestAnimationFrame(() => explorerRef.current?.focus());
      return;
    }
    if (!explorer) return;
    if (explorer.isFocused()) {
      const target = explorerReturnFocusRef.current;
      explorerReturnFocusRef.current = null;
      if (target && document.body.contains(target)) {
        target.focus();
      } else {
        (document.activeElement as HTMLElement | null)?.blur?.();
      }
      return;
    }
    const active = document.activeElement;
    explorerReturnFocusRef.current =
      active instanceof HTMLElement && active !== document.body ? active : null;
    explorer.focus();
  }, [explorerRef, persistSidebarView, persistSidebarCollapsed, sidebarView]);

  // The view actually rendered: in sidebar mode "agents" is coerced to Files so
  // the primary sidebar never duplicates the Tabs+Agents column's Agent dock.
  const renderedSidebarView = effectiveSidebarView(sidebarView, layoutMode);

  return {
    sidebarRef,
    sidebarWidthRef,
    sidebarView: renderedSidebarView,
    persistSidebarView,
    toggleSidebar,
    cycleSidebarView,
    persistSidebarWidth,
    toggleExplorerFocus,
  };
}
