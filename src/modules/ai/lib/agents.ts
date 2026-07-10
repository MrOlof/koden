import { LazyStore } from "@tauri-apps/plugin-store";

export type AgentIconId =
  | "coder"
  | "architect"
  | "reviewer"
  | "security"
  | "designer"
  | "spark";

export type Agent = {
  id: string;
  name: string;
  description: string;
  instructions: string;
  icon: AgentIconId;
  builtIn: boolean;
};

// The one built-in persona: the Librarian — the same entity that curates the
// Koden Brain. Real coding agents run in terminal panes; the chat answers
// questions about the user's projects grounded in the brain index + memory.
export const BUILTIN_AGENTS: readonly Agent[] = [
  {
    id: "builtin:librarian",
    name: "Librarian",
    description:
      "The Koden Brain's curator. Answers about your projects from the index and memory notes.",
    icon: "architect",
    builtIn: true,
    instructions: `You are the Librarian — the curator of the Koden Brain, the local index and memory of the user's projects. You answer questions about those projects grounded in that memory.
- Ground answers in the brain: brain_search finds files and memory notes across indexed projects; brain_notes lists a project's memory cards. Read the underlying file (read_file) when a hit alone isn't enough.
- Cite the source: name the note or file (project + path) each answer came from.
- When the index and notes hold nothing on a question, say so plainly. Never invent project facts.
- You may suggest a memory update when you spot something stale or missing — but you never write memory yourself. Curation happens in the review inbox, where the user approves every change.
- Terse. No filler.`,
  },
] as const;

const STORE_PATH = "koden-ai-agents.json";
const KEY_CUSTOM = "customAgents";
const KEY_ACTIVE = "activeAgentId";

const store = new LazyStore(STORE_PATH, { defaults: {}, autoSave: 200 });

export type LoadedAgents = {
  custom: Agent[];
  activeId: string;
};

export async function loadAgents(): Promise<LoadedAgents> {
  // One IPC roundtrip via entries() instead of two sequential get()s.
  const entries = await store.entries();
  let custom: Agent[] | undefined;
  let activeId: string | undefined;
  for (const [k, v] of entries) {
    if (k === KEY_CUSTOM) custom = v as Agent[];
    else if (k === KEY_ACTIVE) activeId = v as string;
  }
  // A persisted activeId can point at a removed builtin (the five pre-Librarian
  // personas) — fall back to the Librarian; user-authored customs still resolve.
  const list = [...BUILTIN_AGENTS, ...(custom ?? [])];
  const resolved =
    activeId && list.some((a) => a.id === activeId)
      ? activeId
      : BUILTIN_AGENTS[0].id;
  return { custom: custom ?? [], activeId: resolved };
}

export async function saveCustomAgents(custom: Agent[]): Promise<void> {
  await store.set(KEY_CUSTOM, custom);
  await store.save();
}

export async function saveActiveAgentId(id: string): Promise<void> {
  await store.set(KEY_ACTIVE, id);
  await store.save();
}

export function newAgentId(): string {
  return `a-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

export function findAgent(
  agents: readonly Agent[],
  id: string | null | undefined,
): Agent {
  if (!id) return BUILTIN_AGENTS[0];
  return agents.find((a) => a.id === id) ?? BUILTIN_AGENTS[0];
}
