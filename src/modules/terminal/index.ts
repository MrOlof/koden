export type { CommandMark, CommandMinimapData } from "./lib/commandMarks";
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
  type SplitDir,
  type SplitSide,
  sideToSplit,
  terminalLeaves,
} from "./lib/panes";
export { usePaneTitleStore } from "./lib/paneTitles";
// Dev/test harness buffer-read seams (see src/dev/testBus.ts).
export { readLeafBuffer, serializeLeaf } from "./lib/rendererPool";
export { useTerminalFileDrop } from "./lib/useTerminalFileDrop";
export {
  addTurnForLeaf,
  clearFocusedTerminal,
  disposeSession,
  getCommandMarksForLeaf,
  getSearchAddonForLeaf,
  leafHasForegroundProcess,
  leafIdForPty,
  navigateFocusedBlocks,
  ptyIdForLeaf,
  respawnSession,
  scrollToCommandForLeaf,
  submitToLeaf,
  subscribeCommandsForLeaf,
  whenSessionReady,
  writeToSession,
} from "./lib/useTerminalSession";
export {
  PaneTreeView,
  type SplitDirection,
  type SplitPaneType,
} from "./PaneTreeView";
export { TerminalPane, type TerminalPaneHandle } from "./TerminalPane";
export { TerminalStack } from "./TerminalStack";
