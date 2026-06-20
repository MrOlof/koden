export { TerminalPane, type TerminalPaneHandle } from "./TerminalPane";
export { TerminalStack } from "./TerminalStack";
export {
  clearFocusedTerminal,
  disposeSession,
  getCommandMarksForLeaf,
  getSearchAddonForLeaf,
  leafHasForegroundProcess,
  leafIdForPty,
  navigateFocusedBlocks,
  respawnSession,
  scrollToCommandForLeaf,
  submitToLeaf,
  subscribeCommandsForLeaf,
  whenSessionReady,
  writeToSession,
} from "./lib/useTerminalSession";
export type { CommandMark, CommandMinimapData } from "./lib/commandMarks";
export { useTerminalFileDrop } from "./lib/useTerminalFileDrop";
// Dev/test harness buffer-read seams (see src/dev/testBus.ts).
export { readLeafBuffer, serializeLeaf } from "./lib/rendererPool";
export { usePaneTitleStore } from "./lib/paneTitles";
export { nextPaneColor } from "./lib/paneAutoColor";
export {
  countTerminalLeaves,
  findLeaf,
  findLeafCwd,
  hasLeaf,
  isLeaf,
  leafIds,
  type PaneId,
  type PaneNode,
  sideToSplit,
  type SplitDir,
  type SplitSide,
  terminalLeaves,
} from "./lib/panes";
export {
  PaneTreeView,
  type SplitDirection,
  type SplitPaneType,
} from "./PaneTreeView";
