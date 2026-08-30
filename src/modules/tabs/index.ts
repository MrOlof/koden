export { TabBar, TabIcon } from "./TabBar";
export { VerticalTabs } from "./VerticalTabs";
export { GridDialog } from "./GridDialog";
export { useLayoutMode, type LayoutMode } from "./lib/useLayoutMode";
export { useTabStatusStore, type TabStatus } from "./lib/tabStatus";
export { labelFor } from "./lib/tabLabel";
export {
  MAX_PANES_PER_TAB,
  MAX_TOTAL_PANES_PER_TAB,
  DEFAULT_SPACE_ID,
  useTabs,
  type Tab,
  type TerminalTab,
  type EditorTab,
  type PreviewTab,
  type MarkdownTab,
  type AiDiffTab,
  type GitDiffTab,
  type GitHistoryTab,
  type GitCommitFileDiffTab,
  type NotesTab,
  type BoardTab,
  type LibraryTab,
  type LauncherTab,
  type OrchestrationTab,
  type OrchestrationView,
  type AiDiffStatus,
  type TabPatch,
  isRenamableKind,
} from "./lib/useTabs";
export { useWorkspaceCwd } from "./lib/useWorkspaceCwd";
export { useWindowTitle } from "./lib/useWindowTitle";
