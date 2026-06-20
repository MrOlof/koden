import type { AgentRole } from "./types";

export type TeamMember = {
  role: AgentRole;
  name: string;
  task: string;
};

export type TeamTemplate = {
  id: string;
  name: string;
  description: string;
  /** Members spawned under the Director. The Director itself is implicit. */
  members: TeamMember[];
};

/**
 * Curated starting rosters. Models come from each role's defaults
 * (`defaultConfigForRole`) so high-capability roles get capable models and
 * review/audit roles get cheaper ones; the user can override any model per
 * agent after applying a template.
 */
export const TEAM_TEMPLATES: TeamTemplate[] = [
  {
    id: "best-coding",
    name: "Best Coding Team",
    description:
      "General-purpose coding crew under the Director: an architect to plan, a coder to implement, a reviewer and a cheap auditor to check the work, and QA to verify.",
    members: [
      {
        role: "architect",
        name: "Architect",
        task: "Plan the approach and break the work into concrete tasks.",
      },
      {
        role: "coder",
        name: "Coder",
        task: "Implement the assigned tasks.",
      },
      {
        role: "reviewer",
        name: "Reviewer",
        task: "Review diffs for correctness, edge cases and quality.",
      },
      {
        role: "auditor",
        name: "Auditor",
        task: "Cheap second-pass review to catch anything the reviewer missed.",
      },
      {
        role: "qa",
        name: "QA",
        task: "Write and run tests; verify the behavior end to end.",
      },
    ],
  },
];
