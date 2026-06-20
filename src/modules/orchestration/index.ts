export type {
  Agent,
  AgentConfig,
  AgentLimits,
  AgentRole,
  AgentStatus,
  FlowEvent,
  FlowKind,
  TokenUsage,
  TopologyEdge,
} from "./lib/types";
export { AGENT_ROLES, AGENT_STATUSES, FLOW_KINDS } from "./lib/types";
export { defaultConfigForRole, MODEL_ALIASES, roleBlurb } from "./lib/roles";
export {
  getAgentCommand,
  getAgentCommandWithArgs,
  setAgentCommand,
} from "./lib/agentCommand";
export {
  TEAM_TEMPLATES,
  type TeamMember,
  type TeamTemplate,
} from "./lib/templates";
export {
  countActive,
  deriveEdges,
  isActiveStatus,
  sortAgentsForDock,
  totalTokens,
} from "./lib/topology";
export { roleAccent } from "./lib/roleMeta";
export {
  terminalsToRegister,
  type TerminalAgentSeed,
} from "./lib/terminalAgents";
export {
  hydrateOrchestration,
  useOrchestrationStore,
  type SpawnInput,
} from "./store/orchestrationStore";
export { AgentDock } from "./components/AgentDock";
export { OrchestrationActivityBridge } from "./components/OrchestrationActivityBridge";
export { OrchestrationAttentionBridge } from "./components/OrchestrationAttentionBridge";
export { AgentBusBridge } from "./components/AgentBusBridge";
export { DirectorBusBridge } from "./components/DirectorBusBridge";
export type { DirectorCommand } from "./lib/bus";
export { AgentTopologyView } from "./components/AgentTopologyView";
export { MessageFlowInspector } from "./components/MessageFlowInspector";
export {
  DirectorView,
  type SpawnTerminalRequest,
} from "./components/DirectorView";
