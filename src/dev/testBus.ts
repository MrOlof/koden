// =============================================================================
// Koden dev/test harness control bus — window.__KODEN_TEST__
// -----------------------------------------------------------------------------
// DEVELOPER-ONLY. This module is only ever imported behind an
// `import.meta.env.DEV` guard (see src/app/App.tsx). Vite strips the dead branch
// from production builds, so the bus never reaches a release artifact. After any
// build, grep `dist` for `__KODEN_TEST__` to confirm it is absent.
//
// The bus is the primary control surface for the autonomous E2E harness: it lets
// a WebdriverIO spec execute any registered command by id, fire keybinding-only
// actions, read terminal scrollback (the WebGL buffer is DOM-unreadable),
// snapshot tab/pane state, and read the Zustand stores for assertions — without
// scraping fragile DOM. See .memory/test-harness-design-2026-06-20.md.
// =============================================================================

import { useRetryStore } from "@/modules/agents/store/retryStore";
import { useAgentsStore } from "@/modules/ai/store/agentsStore";
import { useChatStore } from "@/modules/ai/store/chatStore";
import { usePlanStore } from "@/modules/ai/store/planStore";
import { useSnippetsStore } from "@/modules/ai/store/snippetsStore";
import { useOrchestrationStore } from "@/modules/orchestration/store/orchestrationStore";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  setAutostart,
  setBackgroundKind,
  setDefaultModel,
  setTheme,
  setThemeId,
} from "@/modules/settings/store";
import type { ShortcutId } from "@/modules/shortcuts/shortcuts";
import type { ShortcutHandlers } from "@/modules/shortcuts/lib/useGlobalShortcuts";
import type { Tab } from "@/modules/tabs";
import { useTabStatusStore } from "@/modules/tabs/lib/tabStatus";
import {
  getCommandMarksForLeaf,
  getSearchAddonForLeaf,
  leafIds,
  readLeafBuffer,
  serializeLeaf,
  submitToLeaf,
  usePaneTitleStore,
  whenSessionReady,
} from "@/modules/terminal";
import { useDocsStore } from "@/modules/workspace-docs/store/docsStore";

/** Minimal structural shape of a command-palette item — { id, run }. */
type CommandItemLike = { id: string; run: () => void };
/** Loose function box — accepts any concrete function without `any`. */
type AnyFn = (...args: never[]) => unknown;

/** Live handles passed in from App.tsx on every relevant render. */
export interface TestBusHandles {
  /** Live palette items (empty unless the palette is open — call openPalette first). */
  commandItems: readonly CommandItemLike[];
  shortcutHandlers: ShortcutHandlers;
  setPaletteOpen: (open: boolean) => void;
  tabs: readonly Tab[];
  activeId: number;
  newGridTab: (
    rows: number,
    cols: number,
    cwd?: string,
  ) => { tabId: number; leafIds: number[] };
  reorderTab: AnyFn;
  duplicateTab: AnyFn;
  moveTabToSpace: AnyFn;
  /** Real effects for the palette's three mode-switch items that are `run: noop`. */
  commandOverrides?: Record<string, () => void>;
}

let current: TestBusHandles | null = null;

function call(fn: AnyFn | undefined, ...args: unknown[]): unknown {
  return (fn as ((...a: unknown[]) => unknown) | undefined)?.(...args);
}

function requireHandles(): TestBusHandles {
  if (!current) throw new Error("__KODEN_TEST__: bus not installed yet");
  return current;
}

function storeSnapshots(): Record<string, unknown> {
  return {
    preferences: usePreferencesStore.getState(),
    orchestration: useOrchestrationStore.getState(),
    docs: useDocsStore.getState(),
    chat: useChatStore.getState(),
    agents: useAgentsStore.getState(),
    snippets: useSnippetsStore.getState(),
    plan: usePlanStore.getState(),
    retry: useRetryStore.getState(),
    tabStatus: useTabStatusStore.getState(),
    paneTitles: usePaneTitleStore.getState(),
  };
}

function tabsSnapshot() {
  const h = requireHandles();
  const active = h.tabs.find((t) => t.id === h.activeId) ?? null;
  const isTerminal = active?.kind === "terminal";
  const tree = isTerminal ? active.paneTree : null;
  const leaves = tree ? leafIds(tree) : [];
  return {
    tabCount: h.tabs.length,
    activeId: h.activeId,
    activeKind: active?.kind ?? null,
    activeTitle: active?.title ?? null,
    activeLeafId: isTerminal ? active.activeLeafId : null,
    paneCount: leaves.length,
    leafIds: leaves,
    tabs: h.tabs.map((t) => ({
      id: t.id,
      kind: t.kind,
      title: t.title,
      spaceId: t.spaceId,
    })),
  };
}

/** The shape assigned to `window.__KODEN_TEST__`. */
export interface KodenTestBus {
  readonly version: number;
  ready(): boolean;
  // --- commands & shortcuts ---
  openPalette(open?: boolean): void;
  commandCount(): number;
  commandIds(): string[];
  runCommandById(id: string): void;
  runShortcut(id: ShortcutId, index?: number): void;
  // --- tabs / panes / grids ---
  tabsSnapshot(): ReturnType<typeof tabsSnapshot>;
  newGridTab(
    rows: number,
    cols: number,
    cwd?: string,
  ): { tabId: number; leafIds: number[] };
  reorderTab(tabId: number, targetTabId: number, edge: string): unknown;
  duplicateTab(id: number): void;
  moveTabToSpace(tabId: number, spaceId: string): unknown;
  // --- terminal ---
  submitToLeaf(leafId: number, command: string): unknown;
  whenSessionReady(leafId: number): Promise<unknown>;
  serialize(leafId: number): string | null;
  getBuffer(leafId: number): string | null;
  searchResultCount(leafId: number): number | null;
  commandMarkCount(leafId: number): number | null;
  // --- stores (read for assertions; actions are live) ---
  getStores(): Record<string, unknown>;
  // --- settings setters (the separate-webview bypass) ---
  settings: {
    setTheme: typeof setTheme;
    setThemeId: typeof setThemeId;
    setAutostart: typeof setAutostart;
    setDefaultModel: typeof setDefaultModel;
    setBackgroundKind: typeof setBackgroundKind;
  };
}

const bus: KodenTestBus = {
  version: 1,
  ready: () => current !== null,

  openPalette(open = true) {
    requireHandles().setPaletteOpen(open);
  },
  commandCount() {
    return current?.commandItems.length ?? 0;
  },
  commandIds() {
    return (current?.commandItems ?? []).map((i) => i.id);
  },
  runCommandById(id) {
    const h = requireHandles();
    const override = h.commandOverrides?.[id];
    if (override) {
      override();
      return;
    }
    const item = h.commandItems.find((i) => i.id === id);
    if (!item) {
      throw new Error(
        `__KODEN_TEST__: command '${id}' not found (count=${h.commandItems.length}). ` +
          "Palette items only exist while the palette is open — call openPalette(true) " +
          "and poll commandCount() > 0 first.",
      );
    }
    item.run();
  },
  runShortcut(id, index) {
    const handler = requireHandles().shortcutHandlers[id];
    if (!handler) throw new Error(`__KODEN_TEST__: no shortcut handler '${id}'`);
    (handler as (arg?: unknown) => void)(index);
  },

  tabsSnapshot,
  newGridTab(rows, cols, cwd) {
    return requireHandles().newGridTab(rows, cols, cwd);
  },
  reorderTab(tabId, targetTabId, edge) {
    return call(requireHandles().reorderTab, tabId, targetTabId, edge);
  },
  duplicateTab(id) {
    call(requireHandles().duplicateTab, id);
  },
  moveTabToSpace(tabId, spaceId) {
    return call(requireHandles().moveTabToSpace, tabId, spaceId);
  },

  submitToLeaf(leafId, command) {
    return submitToLeaf(leafId, command);
  },
  whenSessionReady(leafId) {
    return whenSessionReady(leafId);
  },
  serialize(leafId) {
    return serializeLeaf(leafId);
  },
  getBuffer(leafId) {
    return readLeafBuffer(leafId);
  },
  searchResultCount(leafId) {
    const addon = getSearchAddonForLeaf(leafId) as
      | { resultCount?: number }
      | null
      | undefined;
    return addon?.resultCount ?? null;
  },
  commandMarkCount(leafId) {
    const marks = getCommandMarksForLeaf(leafId) as
      | { length: number }
      | null
      | undefined;
    return marks?.length ?? null;
  },

  getStores: storeSnapshots,
  settings: {
    setTheme,
    setThemeId,
    setAutostart,
    setDefaultModel,
    setBackgroundKind,
  },
};

/**
 * Install/refresh the test bus. Called from a DEV-gated effect in App.tsx on
 * every render whose handles changed; the window object is defined once and all
 * methods read the latest `current`, so closures never go stale.
 */
export function installTestBus(handles: TestBusHandles): void {
  current = handles;
  if (typeof window === "undefined") return;
  const w = window as unknown as { __KODEN_TEST__?: KodenTestBus };
  if (!w.__KODEN_TEST__) {
    w.__KODEN_TEST__ = bus;
    console.info("[koden-test] __KODEN_TEST__ bus installed (DEV only)");
  }
}
