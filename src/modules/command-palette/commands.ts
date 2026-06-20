import type { SearchTarget } from "@/modules/header";
import {
  MAX_PANES_PER_TAB,
  MAX_TOTAL_PANES_PER_TAB,
  type Tab,
} from "@/modules/tabs";
import { countTerminalLeaves, leafIds } from "@/modules/terminal";
import {
  Cancel01Icon,
  CheckListIcon,
  CommandLineIcon,
  DashboardSquare01Icon,
  FileEditIcon,
  FileSearchIcon,
  Globe02Icon,
  HierarchySquare01Icon,
  IncognitoIcon,
  KanbanIcon,
  KeyboardIcon,
  LayoutTwoColumnIcon,
  LayoutTwoRowIcon,
  MessageMultiple01Icon,
  Note01Icon,
  PaintBoardIcon,
  Search01Icon,
  Settings01Icon,
  SidebarLeftIcon,
  SourceCodeIcon,
  SparklesIcon,
  TerminalIcon,
} from "@hugeicons/core-free-icons";
import type { PaletteItem } from "./types";

export const COMMAND_GROUPS = [
  "General",
  "Spaces",
  "Tabs",
  "Panes",
  "Agents",
  "Git",
  "Search",
  "View",
  "AI",
] as const;

export type CommandPaletteActionContext = {
  tabs: Tab[];
  activeId: number;
  searchTarget: SearchTarget;
  explorerRoot: string | null;
  home: string | null;
  openNewTab: () => void;
  openNewBlock: () => void;
  openNewPrivate: () => void;
  openNewEditor: () => void;
  openNewPreview: () => void;
  openNewNotes: () => void;
  openNewBoard: () => void;
  openNewTasks: () => void;
  openDirector: () => void;
  openBrain: () => void;
  openAgentTopology: () => void;
  openMessageFlow: () => void;
  openGitGraph: () => void;
  toggleSourceControl: () => void;
  closeActiveTabOrPane: () => void;
  splitPaneRight: () => void;
  splitPaneDown: () => void;
  addTerminalPane: (dir: "row" | "col") => void;
  addNotePane: (dir: "row" | "col") => void;
  addTasksPane: (dir: "row" | "col") => void;
  focusSearch: () => void;
  focusExplorerSearch: () => void;
  toggleSidebar: () => void;
  toggleLayout: () => void;
  toggleAi: () => void;
  askAiSelection: () => void;
  openSettings: () => void;
  openKeyboardShortcuts: () => void;
  spaces: { id: string; name: string }[];
  activeSpaceId: string | null;
  openSpacesOverview: () => void;
  newSpace: () => void;
  switchSpace: (id: string) => void;
};

const noop = () => {};

export function createCommandItems(
  ctx: CommandPaletteActionContext,
): PaletteItem[] {
  const activeTab = ctx.tabs.find((tab) => tab.id === ctx.activeId);
  const activeTerminalTab = activeTab?.kind === "terminal" ? activeTab : null;
  const activePaneCount = activeTerminalTab
    ? leafIds(activeTerminalTab.paneTree).length
    : 0;
  const activeTerminalCount = activeTerminalTab
    ? countTerminalLeaves(activeTerminalTab.paneTree)
    : 0;
  const onlyOneTab = ctx.tabs.length < 2;
  const noWorkspaceRoot = !ctx.explorerRoot && !ctx.home;
  // Splitting adds a terminal (renderer-bound); note panes don't count.
  const splitDisabled = !activeTerminalTab
    ? "No terminal tab"
    : activeTerminalCount >= MAX_PANES_PER_TAB
      ? "Pane limit"
      : activePaneCount >= MAX_TOTAL_PANES_PER_TAB
        ? "Pane limit"
        : undefined;
  // Adding a note only hits the overall cap.
  const addNoteDisabled = !activeTerminalTab
    ? "No terminal tab"
    : activePaneCount >= MAX_TOTAL_PANES_PER_TAB
      ? "Pane limit"
      : undefined;
  const closeDisabled =
    onlyOneTab && activePaneCount < 2 ? "Last tab" : undefined;

  return [
    {
      id: "settings.open",
      title: "Open settings",
      group: "General",
      keywords: ["preferences", "config"],
      icon: Settings01Icon,
      shortcutId: "settings.open",
      run: ctx.openSettings,
    },
    {
      id: "theme.pick",
      title: "Change theme...",
      group: "General",
      keywords: ["theme", "appearance", "color", "dark", "light"],
      icon: PaintBoardIcon,
      run: noop,
    },
    {
      id: "shortcuts.open",
      title: "Keyboard shortcuts",
      group: "General",
      keywords: ["keys", "keybindings", "settings"],
      icon: KeyboardIcon,
      run: ctx.openKeyboardShortcuts,
    },
    {
      id: "spaces.overview",
      title: "Spaces: Overview",
      group: "Spaces",
      keywords: ["spaces", "sessions", "overview", "organize", "manage", "move"],
      icon: DashboardSquare01Icon,
      run: ctx.openSpacesOverview,
    },
    {
      id: "spaces.new",
      title: "New Space",
      group: "Spaces",
      keywords: ["space", "session", "workspace", "group", "create"],
      icon: DashboardSquare01Icon,
      run: ctx.newSpace,
    },
    ...ctx.spaces.map((sp) => ({
      id: `spaces.switch.${sp.id}`,
      title: `Switch to ${sp.name}`,
      group: "Spaces" as const,
      keywords: ["space", "switch", "session", sp.name],
      icon: DashboardSquare01Icon,
      disabledReason:
        sp.id === ctx.activeSpaceId ? "Current space" : undefined,
      run: () => ctx.switchSpace(sp.id),
    })),
    {
      id: "tab.new",
      title: "New terminal",
      group: "Tabs",
      keywords: ["shell", "terminal", "new tab"],
      icon: TerminalIcon,
      shortcutId: "tab.new",
      run: ctx.openNewTab,
    },
    {
      id: "tab.newBlock",
      title: "New block terminal",
      group: "Tabs",
      keywords: ["blocks", "warp", "command blocks", "terminal"],
      icon: DashboardSquare01Icon,
      run: ctx.openNewBlock,
    },
    {
      id: "tab.newPrivate",
      title: "New private terminal",
      group: "Tabs",
      keywords: ["privacy", "private", "incognito", "hidden from ai"],
      icon: IncognitoIcon,
      shortcutId: "tab.newPrivate",
      run: ctx.openNewPrivate,
    },
    {
      id: "tab.newEditor",
      title: "New editor tab",
      group: "Tabs",
      keywords: ["file", "editor", "create"],
      icon: FileEditIcon,
      shortcutId: "tab.newEditor",
      disabledReason: noWorkspaceRoot ? "No workspace root" : undefined,
      run: ctx.openNewEditor,
    },
    {
      id: "tab.newPreview",
      title: "New web preview",
      group: "Tabs",
      keywords: ["browser", "web", "localhost", "preview"],
      icon: Globe02Icon,
      shortcutId: "tab.newPreview",
      run: ctx.openNewPreview,
    },
    {
      id: "tab.newNotes",
      title: "New notes",
      group: "Tabs",
      keywords: ["notes", "scratchpad", "markdown", "memo"],
      icon: Note01Icon,
      run: ctx.openNewNotes,
    },
    {
      id: "tab.newBoard",
      title: "New board",
      group: "Tabs",
      keywords: ["board", "kanban", "progress", "planning"],
      icon: KanbanIcon,
      run: ctx.openNewBoard,
    },
    {
      id: "tab.newTasks",
      title: "New tasks",
      group: "Tabs",
      keywords: ["tasks", "todo", "checklist", "checkbox", "to-do", "list"],
      icon: CheckListIcon,
      run: ctx.openNewTasks,
    },
    {
      id: "orchestration.director",
      title: "Open Director",
      group: "Agents",
      keywords: ["director", "agents", "orchestrate", "spawn", "command"],
      icon: CommandLineIcon,
      run: ctx.openDirector,
    },
    {
      id: "brain.open",
      title: "Open Brain",
      group: "Agents",
      keywords: ["brain", "index", "search", "knowledge", "librarian", "code"],
      icon: Search01Icon,
      run: ctx.openBrain,
    },
    {
      id: "orchestration.topology",
      title: "Open Agent Topology",
      group: "Agents",
      keywords: ["topology", "graph", "agents", "relationships", "flow"],
      icon: HierarchySquare01Icon,
      run: ctx.openAgentTopology,
    },
    {
      id: "orchestration.flow",
      title: "Open Message Flow",
      group: "Agents",
      keywords: ["message", "flow", "timeline", "delegation", "handoff", "review"],
      icon: MessageMultiple01Icon,
      run: ctx.openMessageFlow,
    },
    {
      id: "tab.close",
      title: "Close tab or pane",
      group: "Tabs",
      keywords: ["close", "remove", "pane"],
      icon: Cancel01Icon,
      shortcutId: "tab.close",
      disabledReason: closeDisabled,
      run: ctx.closeActiveTabOrPane,
    },
    {
      id: "pane.splitRight",
      title: "Split pane right",
      group: "Panes",
      keywords: ["terminal", "pane", "split", "right", "column"],
      icon: LayoutTwoColumnIcon,
      shortcutId: "pane.splitRight",
      disabledReason: splitDisabled,
      run: ctx.splitPaneRight,
    },
    {
      id: "pane.splitDown",
      title: "Split pane down",
      group: "Panes",
      keywords: ["terminal", "pane", "split", "down", "row"],
      icon: LayoutTwoRowIcon,
      shortcutId: "pane.splitDown",
      disabledReason: splitDisabled,
      run: ctx.splitPaneDown,
    },
    {
      id: "pane.addTerminal",
      title: "Add terminal pane (below)",
      group: "Panes",
      keywords: ["terminal", "shell", "pane", "split", "down", "below", "bottom"],
      icon: TerminalIcon,
      disabledReason: splitDisabled,
      run: () => ctx.addTerminalPane("col"),
    },
    {
      id: "pane.addTerminalRight",
      title: "Add terminal pane (right)",
      group: "Panes",
      keywords: ["terminal", "shell", "pane", "split", "right", "side", "column"],
      icon: TerminalIcon,
      disabledReason: splitDisabled,
      run: () => ctx.addTerminalPane("row"),
    },
    {
      id: "pane.addNote",
      title: "Add note pane (below)",
      group: "Panes",
      keywords: ["note", "notes", "scratchpad", "pane", "markdown", "down", "below"],
      icon: Note01Icon,
      shortcutId: "pane.addNote",
      disabledReason: addNoteDisabled,
      run: () => ctx.addNotePane("col"),
    },
    {
      id: "pane.addNoteRight",
      title: "Add note pane (right)",
      group: "Panes",
      keywords: ["note", "notes", "scratchpad", "pane", "markdown", "right", "side", "column"],
      icon: Note01Icon,
      disabledReason: addNoteDisabled,
      run: () => ctx.addNotePane("row"),
    },
    {
      id: "pane.addTasks",
      title: "Add tasks pane (below)",
      group: "Panes",
      keywords: ["tasks", "todo", "checklist", "checkbox", "pane", "down", "below"],
      icon: CheckListIcon,
      disabledReason: addNoteDisabled,
      run: () => ctx.addTasksPane("col"),
    },
    {
      id: "pane.addTasksRight",
      title: "Add tasks pane (right)",
      group: "Panes",
      keywords: ["tasks", "todo", "checklist", "checkbox", "pane", "right", "side", "column"],
      icon: CheckListIcon,
      disabledReason: addNoteDisabled,
      run: () => ctx.addTasksPane("row"),
    },
    {
      id: "git.graph",
      title: "Open git graph",
      group: "Git",
      keywords: ["git", "graph", "history", "log", "commits"],
      icon: SourceCodeIcon,
      run: ctx.openGitGraph,
    },
    {
      id: "git.source",
      title: "Toggle source control",
      group: "Git",
      keywords: ["git", "source control", "changes", "staging", "diff"],
      icon: SourceCodeIcon,
      shortcutId: "pane.source",
      run: ctx.toggleSourceControl,
    },
    {
      id: "search.content",
      title: "Find content in files",
      group: "Search",
      keywords: ["grep", "ripgrep", "text", "contents", "search in files"],
      icon: FileSearchIcon,
      trailing: "#",
      run: noop,
    },
    {
      id: "history.open",
      title: "Search command history",
      group: "Search",
      keywords: ["history", "shell", "rerun", "previous commands"],
      icon: TerminalIcon,
      trailing: ">",
      run: noop,
    },
    {
      id: "search.focus",
      title: "Find in current tab",
      group: "Search",
      keywords: ["find", "terminal", "editor", "current"],
      icon: Search01Icon,
      shortcutId: "search.focus",
      disabledReason: ctx.searchTarget ? undefined : "No searchable view",
      run: ctx.focusSearch,
    },
    {
      id: "explorer.search",
      title: "Search files by name",
      group: "Search",
      keywords: ["explorer", "workspace", "file", "open"],
      icon: Search01Icon,
      shortcutId: "explorer.search",
      disabledReason: ctx.explorerRoot ? undefined : "No workspace root",
      run: ctx.focusExplorerSearch,
    },
    {
      id: "sidebar.toggle",
      title: "Toggle file explorer",
      group: "View",
      keywords: ["sidebar", "files", "explorer"],
      icon: SidebarLeftIcon,
      shortcutId: "sidebar.toggle",
      run: ctx.toggleSidebar,
    },
    {
      id: "view.toggleLayout",
      title: "Toggle tab layout (top / sidebar)",
      group: "View",
      keywords: ["layout", "tabs", "vertical", "sidebar", "top", "vscode"],
      icon: SidebarLeftIcon,
      run: ctx.toggleLayout,
    },
    {
      id: "ai.toggle",
      title: "Toggle AI agent",
      group: "AI",
      keywords: ["assistant", "chat", "agent"],
      icon: SparklesIcon,
      shortcutId: "ai.toggle",
      run: ctx.toggleAi,
    },
    {
      id: "ai.askSelection",
      title: "Ask AI about selection",
      group: "AI",
      keywords: ["selection", "explain", "assistant", "chat"],
      icon: SparklesIcon,
      shortcutId: "ai.askSelection",
      run: ctx.askAiSelection,
    },
  ];
}
