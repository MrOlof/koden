import type { PaneNode } from "@/modules/terminal/lib/panes";

/** Tab kinds the chat may open. 'brain' = the Koden Brain view (singleton). */
export type LayoutTabKind =
  | "terminal"
  | "notes"
  | "board"
  | "tasks"
  | "editor"
  | "library"
  | "brain";

/** Pane kinds that can live in a split — PaneTreeView's SplitPaneType. */
export type LayoutSplitKind = "terminal" | "note" | "tasks";

/** Structurally identical to the pane model's SplitSide. */
export type LayoutSplitSide = "left" | "right" | "top" | "bottom";

export type LayoutOpenTabResult =
  | { tabId: number; action: "opened" | "focused"; title: string }
  | { error: string };

export type LayoutSplitResult =
  | { tabId: number; paneId: number }
  | { error: string };

export type LayoutFocusResult =
  | { focused: true; tabId: number; paneId: number }
  | { error: string };

/**
 * One terminal pane as the targeting tools see it — every space, every tab
 * (the layout snapshot covers only the active space; this list never filters).
 */
export type TerminalTargetInfo = {
  paneId: number;
  tabId: number;
  /** Space NAME (falls back to the space id when unnamed). */
  space: string;
  /** Display title: the pane's own title when set, else the tab's label. */
  title: string;
  /** Owning tab's label (custom name, else cwd basename) — a fallback match tier. */
  tabTitle: string;
  cwd: string | null;
  /** Agent registered on this pane (orchestration agent, or a managed 'claude'). */
  agent: { name: string; status: string } | null;
  /** This pane is the focused pane of the ACTIVE tab. */
  active: boolean;
  /** This pane is its own tab's focused leaf (tab-name matches collapse here). */
  tabActive: boolean;
  /** Privacy-mode tab: buffer reads and sends are refused. */
  private: boolean;
  /** Restored but never activated — no live PTY until the user opens it. */
  cold: boolean;
};

/** Raw layout snapshot from the app; the layout tool shapes it for the model. */
export type LayoutSnapshot = {
  activeTabId: number | null;
  /** Tabs in the active space, bar order. paneTree only on terminal-kind tabs. */
  tabs: Array<{
    tabId: number;
    kind: string;
    title: string;
    active: boolean;
    paneTree?: PaneNode;
    activeLeafId?: number;
  }>;
  /** leafId → user-visible pane label (from the pane title store). */
  paneTitles: Record<number, string>;
};

export type ToolContext = {
  /** Active terminal tab cwd, used to resolve relative paths. Null = home. */
  getCwd: () => string | null;
  /** Workspace root (explorer root). Used by tools that operate over the project. */
  getWorkspaceRoot: () => string | null;
  /** Last N lines of the active terminal buffer (or null if not a terminal tab). */
  getTerminalContext: () => string | null;
  isActiveTerminalPrivate: () => boolean;
  /**
   * Type a string into the active terminal at the prompt — without executing.
   * Returns false if there is no active terminal tab to inject into.
   */
  injectIntoActivePty: (text: string) => boolean;
  /** Open a new preview tab (in-app iframe) at the given URL. */
  openPreview: (url: string) => boolean;
  /** Spawn a Claude Code agent in a new terminal tab, bound to this session. */
  spawnAgent: (prompt: string) => { tabId: number; leafId: number } | null;
  /** Read the terminal scrollback tail of a managed agent's leaf. */
  readAgentOutput: (leafId: number) => string | null;
  readCache: Map<string, { size: number; hash: number }>;
  /** Active chat session id — used by tools that persist per-session state (todos). */
  getSessionId: () => string | null;
  // Workspace layout (create/arrange only — no close/delete surface, ADR-017).
  /** Open a workspace tab; singleton kinds (library/brain) focus an existing one. */
  openWorkspaceTab: (
    kind: LayoutTabKind,
    opts?: { title?: string; path?: string },
  ) => LayoutOpenTabResult;
  /** Split the active pane of the active tab; focus follows the new pane. */
  splitWorkspacePane: (
    kind: LayoutSplitKind,
    side: LayoutSplitSide,
    title?: string,
  ) => LayoutSplitResult;
  /** Focus a pane by leaf id, activating its tab. */
  focusWorkspacePane: (paneId: number) => LayoutFocusResult;
  /** Raw layout state of the active space. */
  getWorkspaceLayout: () => LayoutSnapshot;
  // Terminal targeting (ADR-017 addendum): list/read free, type free,
  // submit approval-gated unless the user armed hands-free mode.
  /** Every terminal pane across ALL spaces. */
  listTerminalTargets: () => TerminalTargetInfo[];
  /** Redacted scrollback tail of any leaf. Privacy is checked by the caller against the target list. */
  readTerminalBuffer: (leafId: number) => string | null;
  /** Raw pty write; submit=true sends Enter as a separate delayed chunk. Never moves focus. */
  sendToTerminal: (leafId: number, data: string, submit: boolean) => boolean;
  /** Shell-vs-TUI discriminator: true when a foreground app owns the pty. */
  terminalHasForegroundProcess: (leafId: number) => Promise<boolean>;
  /** Hands-free pref: armed = terminal_send submits skip the approval card. */
  isHandsFreeArmed: () => boolean;
};

export function resolvePath(rawPath: string, cwd: string | null): string {
  if (rawPath.startsWith("/") || /^[a-zA-Z]:[\\/]/.test(rawPath))
    return rawPath;
  if (!cwd)
    throw new Error(
      `cannot resolve relative path "${rawPath}": no active terminal cwd. Pass an absolute path.`,
    );
  const sep = cwd.includes("\\") && !cwd.includes("/") ? "\\" : "/";
  return cwd.endsWith(sep) ? `${cwd}${rawPath}` : `${cwd}${sep}${rawPath}`;
}
