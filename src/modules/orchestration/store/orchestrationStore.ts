import { create } from "zustand";
import { defaultConfigForRole } from "../lib/roles";
import type {
  Agent,
  AgentConfig,
  AgentRole,
  AgentStatus,
  FlowEvent,
  FlowKind,
  TokenUsage,
} from "../lib/types";

const MAX_FLOW = 500;

function uid(prefix: string): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

function now(): number {
  return Date.now();
}

export type SpawnInput = {
  role: AgentRole;
  name?: string;
  task?: string | null;
  parentId?: string | null;
  config?: Partial<AgentConfig>;
  leafId?: number | null;
  tabId?: number | null;
  /** Director's native Claude Code subagent (no Koden terminal). */
  native?: boolean;
};

type OrchestrationState = {
  agents: Record<string, Agent>;
  flow: FlowEvent[];
  hydrated: boolean;

  spawn: (input: SpawnInput) => string;
  updateConfig: (id: string, patch: Partial<AgentConfig>) => void;
  setStatus: (id: string, status: AgentStatus) => void;
  setTask: (id: string, task: string | null) => void;
  rename: (id: string, name: string) => void;
  addTokens: (id: string, delta: TokenUsage, cost?: number) => void;
  linkTerminal: (id: string, link: { leafId: number; tabId: number }) => void;
  /** Clear an agent's terminal link when its leaf is closed/disposed. */
  unlinkByLeaf: (leafId: number) => void;
  /** Remove the agent whose terminal leaf was closed/disposed. */
  removeByLeaf: (leafId: number) => void;
  remove: (id: string) => void;
  /** Remove an agent and its direct children (e.g. a Director and its team). */
  removeWithChildren: (id: string) => void;
  reset: () => void;

  logFlow: (e: {
    kind: FlowKind;
    fromId: string;
    toId?: string | null;
    summary: string;
    detail?: string;
  }) => void;
  /** Director assigns/routes a task to an agent: sets task + logs delegation. */
  assign: (fromId: string, toId: string, task: string) => void;
};

function touch(agents: Record<string, Agent>, id: string): Record<string, Agent> {
  const a = agents[id];
  if (!a) return agents;
  return { ...agents, [id]: { ...a, lastActivityAt: now() } };
}

export const useOrchestrationStore = create<OrchestrationState>((set, get) => ({
  agents: {},
  flow: [],
  hydrated: false,

  spawn: (input) => {
    const id = uid("ag");
    const ts = now();
    const role = input.role;
    const agent: Agent = {
      id,
      name: input.name?.trim() || defaultNameForRole(role, get().agents),
      role,
      status: "spawning",
      task: input.task ?? null,
      config: { ...defaultConfigForRole(role), ...input.config },
      tokens: { input: 0, output: 0 },
      cost: 0,
      parentId: input.parentId ?? null,
      leafId: input.leafId ?? null,
      tabId: input.tabId ?? null,
      ...(input.native && { native: true }),
      createdAt: ts,
      lastActivityAt: ts,
    };
    set((s) => ({ agents: { ...s.agents, [id]: agent } }));
    if (input.parentId && get().agents[input.parentId]) {
      get().logFlow({
        kind: "delegation",
        fromId: input.parentId,
        toId: id,
        summary: input.task
          ? `Spawned ${agent.name}: ${input.task}`
          : `Spawned ${agent.name}`,
      });
    }
    return id;
  },

  updateConfig: (id, patch) =>
    set((s) => {
      const a = s.agents[id];
      if (!a) return s;
      return {
        agents: {
          ...s.agents,
          [id]: { ...a, config: { ...a.config, ...patch } },
        },
      };
    }),

  setStatus: (id, status) =>
    set((s) => {
      const a = s.agents[id];
      if (!a || a.status === status) return s;
      return {
        agents: { ...s.agents, [id]: { ...a, status, lastActivityAt: now() } },
      };
    }),

  setTask: (id, task) =>
    set((s) => {
      const a = s.agents[id];
      if (!a) return s;
      return {
        agents: { ...s.agents, [id]: { ...a, task, lastActivityAt: now() } },
      };
    }),

  rename: (id, name) =>
    set((s) => {
      const a = s.agents[id];
      const trimmed = name.trim();
      if (!a || !trimmed) return s;
      return { agents: { ...s.agents, [id]: { ...a, name: trimmed } } };
    }),

  addTokens: (id, delta, cost) =>
    set((s) => {
      const a = s.agents[id];
      if (!a) return s;
      return {
        agents: {
          ...s.agents,
          [id]: {
            ...a,
            tokens: {
              input: a.tokens.input + delta.input,
              output: a.tokens.output + delta.output,
            },
            cost: a.cost + (cost ?? 0),
            lastActivityAt: now(),
          },
        },
      };
    }),

  linkTerminal: (id, link) =>
    set((s) => {
      const a = s.agents[id];
      if (!a) return s;
      return { agents: { ...s.agents, [id]: { ...a, ...link } } };
    }),

  unlinkByLeaf: (leafId) =>
    set((s) => {
      const a = Object.values(s.agents).find((x) => x.leafId === leafId);
      if (!a) return s;
      return {
        agents: {
          ...s.agents,
          [a.id]: { ...a, leafId: null, tabId: null },
        },
      };
    }),

  removeByLeaf: (leafId) =>
    set((s) => {
      const a = Object.values(s.agents).find((x) => x.leafId === leafId);
      if (!a) return s;
      const next = { ...s.agents };
      delete next[a.id];
      return { agents: next };
    }),

  remove: (id) =>
    set((s) => {
      if (!s.agents[id]) return s;
      const next = { ...s.agents };
      delete next[id];
      return { agents: next };
    }),

  removeWithChildren: (id) =>
    set((s) => {
      if (!s.agents[id]) return s;
      const next = { ...s.agents };
      delete next[id];
      for (const a of Object.values(s.agents)) {
        if (a.parentId === id) delete next[a.id];
      }
      return { agents: next };
    }),

  reset: () => set({ agents: {}, flow: [] }),

  logFlow: (e) =>
    set((s) => {
      const event: FlowEvent = {
        id: uid("fl"),
        ts: now(),
        kind: e.kind,
        fromId: e.fromId,
        toId: e.toId ?? null,
        summary: e.summary,
        ...(e.detail !== undefined && { detail: e.detail }),
      };
      const flow = [...s.flow, event];
      return {
        flow: flow.length > MAX_FLOW ? flow.slice(flow.length - MAX_FLOW) : flow,
        agents: touch(s.agents, e.fromId),
      };
    }),

  assign: (fromId, toId, task) => {
    const { setTask, setStatus, logFlow } = get();
    setTask(toId, task);
    setStatus(toId, "working");
    logFlow({ kind: "delegation", fromId, toId, summary: task });
  },
}));

function defaultNameForRole(
  role: AgentRole,
  agents: Record<string, Agent>,
): string {
  const sameRole = Object.values(agents).filter((a) => a.role === role).length;
  const label = role.charAt(0).toUpperCase() + role.slice(1);
  return sameRole === 0 ? label : `${label} ${sameRole + 1}`;
}

// Orchestration state is intentionally session-scoped: agents live with their
// terminals, which don't survive a restart, so persisting them would just
// resurrect stale, dead-linked entries. Each session starts with an empty
// roster; the Director and its team appear only as they're started/spawned.
export async function hydrateOrchestration(): Promise<void> {
  if (useOrchestrationStore.getState().hydrated) return;
  useOrchestrationStore.setState({ hydrated: true });
}
