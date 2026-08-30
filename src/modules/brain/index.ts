export { BrainActivityBridge } from "./BrainActivityBridge";
export { BrainHeaderMenu } from "./BrainHeaderMenu";
export { BrainMapPane } from "./BrainMapPane";
export { BrainPane } from "./BrainPane";
export { BrainTabIcon } from "./BrainTabIcon";
export { RecoveredPanesBanner } from "./components/RecoveredPanes";
export { useBrainActivityStore } from "./lib/activityStore";
export type {
  BrainActivityEvent,
  BrainStatus,
  BrainStatusReport,
  Gist,
  Hit,
  MemoryProposal,
  NoteSummary,
  Project,
  ProjectStatus,
  ProposalAction,
  RecoveredPane,
  ResumePlan,
} from "./lib/bindings";
export {
  brainBudgetStatus,
  brainBuildGist,
  brainCurate,
  brainDismissRecovered,
  brainRecordTurn,
  brainRecoveredPanes,
  brainReflect,
  brainResumePlan,
  brainSetBudget,
  resolveProjectForCwd,
} from "./lib/bindings";
// Resume cards: `useRecoveredPanes(...).sections` is LauncherSectionModel-shaped
// for the launcher's `extraSections`; the banner is the standalone strip.
export {
  buildResumeCards,
  matchRecoveredPanes,
  recoveredLauncherSections,
  type RecoveredLauncherItem,
  type RecoveredLauncherSection,
  type ResumeCardModel,
} from "./lib/resumeCards";
export {
  markRecoveredPaneConsumed,
  type OpenTerminalForResume,
  useRecoveredPanes,
  useRecoveredPanesStore,
} from "./lib/useRecoveredPanes";
export { useBrainStatus } from "./lib/useBrainStatus";
export { requestBrainView } from "./lib/viewRequest";
