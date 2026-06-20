import { generateObject } from "ai";
import { z } from "zod";
import { useChatStore } from "../store/chatStore";
import { buildConfiguredLanguageModel } from "./agent";

// Roles the Director may spawn (it never spawns another director, and does not
// code itself).
const PLANNED_ROLES = [
  "architect",
  "coder",
  "reviewer",
  "auditor",
  "qa",
  "devops",
  "worker",
] as const;

export type PlannedRole = (typeof PLANNED_ROLES)[number];
export type Complexity = "simple" | "medium" | "hard";

export type PlannedAgent = {
  role: PlannedRole;
  name: string;
  task: string;
  complexity: Complexity;
  /** Claude Code `--model` alias chosen from the complexity. */
  model: string;
};

export type TeamPlan = {
  reasoning: string;
  agents: PlannedAgent[];
};

const MAX_AGENTS = 6;

// simple → cheap/fast, hard → most capable. These are Claude Code `--model`
// aliases so they actually drive each spawned terminal's model.
const COMPLEXITY_TO_MODEL: Record<Complexity, string> = {
  simple: "haiku",
  medium: "sonnet",
  hard: "opus",
};

const PlanSchema = z.object({
  reasoning: z
    .string()
    .describe("One to three sentences on why this team fits the goal."),
  agents: z
    .array(
      z.object({
        role: z.enum(PLANNED_ROLES),
        name: z.string().describe("Short display name, e.g. 'Coder' or 'Auth Coder'."),
        task: z
          .string()
          .describe("A specific, actionable task scoped to this agent's role."),
        complexity: z
          .enum(["simple", "medium", "hard"])
          .describe("simple = Haiku, medium = Sonnet, hard = Opus."),
      }),
    )
    .min(1)
    .max(MAX_AGENTS),
});

const PLANNER_SYSTEM = `You are the Director of a multi-agent coding workspace. You do NOT write code yourself — you design and coordinate the team that will.

Given the user's goal, design the SMALLEST effective team to accomplish it.

Rules:
- Never propose more than ${MAX_AGENTS} agents. Prefer fewer; do not pad the team.
- Pick each agent's role from: architect, coder, reviewer, auditor, qa, devops, worker.
- Match the model to the task's difficulty, not the role: simple = Haiku (quick checks, cheap audits), medium = Sonnet (most implementation/review), hard = Opus (deep design or hard implementation only).
- For a typical coding goal this is roughly: a coder, a reviewer, a cheap auditor, and QA — adjust to the goal. Add an architect only for non-trivial design, devops only when build/deploy/infra is involved.
- Each agent gets one specific, actionable task scoped to its role.
- Do NOT include a director in the team; you are the director.`;

export async function planTeam(goal: string): Promise<TeamPlan> {
  const { selectedModelId, apiKeys } = useChatStore.getState();
  let model: Awaited<ReturnType<typeof buildConfiguredLanguageModel>>;
  try {
    model = await buildConfiguredLanguageModel(selectedModelId, apiKeys, {});
  } catch (e) {
    throw new Error(
      e instanceof Error
        ? `Director can't reach a model: ${e.message}`
        : "Director can't reach a model. Check your AI keys in Settings.",
    );
  }
  const { object } = await generateObject({
    model,
    schema: PlanSchema,
    system: PLANNER_SYSTEM,
    prompt: `Goal:\n${goal.trim()}`,
  });
  return {
    reasoning: object.reasoning,
    agents: object.agents.slice(0, MAX_AGENTS).map((a) => ({
      role: a.role,
      name: a.name,
      task: a.task,
      complexity: a.complexity,
      model: COMPLEXITY_TO_MODEL[a.complexity],
    })),
  };
}
