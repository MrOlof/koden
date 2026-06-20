export const AGENT_ROLES = [
  "director",
  "architect",
  "coder",
  "reviewer",
  "auditor",
  "qa",
  "devops",
  "worker",
] as const;

export type AgentRole = (typeof AGENT_ROLES)[number];

export const AGENT_STATUSES = [
  "spawning",
  "idle",
  "ready",
  "working",
  "reviewing",
  "waiting",
  "blocked",
  "done",
  "error",
] as const;

export type AgentStatus = (typeof AGENT_STATUSES)[number];

export type TokenUsage = {
  input: number;
  output: number;
};

export type AgentLimits = {
  /** Max context window tokens this agent may use. null = provider default. */
  contextLimit: number | null;
  /** Soft USD spend ceiling for this agent. null = unlimited. */
  costLimit: number | null;
};

export type AgentConfig = {
  model: string;
  limits: AgentLimits;
  /** Capability grants, e.g. "fs.write", "shell.run", "git.commit", "net". */
  permissions: string[];
  /** Tool ids the agent is allowed to call. */
  tools: string[];
};

export type Agent = {
  id: string;
  name: string;
  role: AgentRole;
  status: AgentStatus;
  /** Current task summary, null when idle. */
  task: string | null;
  config: AgentConfig;
  tokens: TokenUsage;
  /** Accumulated spend in USD. */
  cost: number;
  /** Owning agent id (the director or a delegating worker). null = root. */
  parentId: string | null;
  /** Terminal leaf running this agent, when it is a terminal coding-agent. */
  leafId: number | null;
  /** Tab hosting the agent's terminal, for activation. */
  tabId: number | null;
  /**
   * True for the Director's native Claude Code subagents (spawned via its Task
   * tool). These have no Koden terminal of their own and are tracked purely
   * through the command bus lifecycle hooks.
   */
  native?: boolean;
  createdAt: number;
  lastActivityAt: number;
};

export const FLOW_KINDS = [
  "message",
  "delegation",
  "handoff",
  "decision",
  "review",
  "audit",
  "approval",
] as const;

export type FlowKind = (typeof FLOW_KINDS)[number];

export type FlowEvent = {
  id: string;
  ts: number;
  kind: FlowKind;
  fromId: string;
  /** null = broadcast / addressed to the whole workspace. */
  toId: string | null;
  summary: string;
  detail?: string;
};

/** A directed relationship between two agents, derived for the topology view. */
export type TopologyEdge = {
  fromId: string;
  toId: string;
  /** "owns" = spawned/manages; "flow" = recent message traffic. */
  kind: "owns" | "flow";
  /** Recent traffic count for "flow" edges. */
  weight: number;
};
