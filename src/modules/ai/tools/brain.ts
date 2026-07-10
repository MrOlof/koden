import { tool } from "ai";
import { z } from "zod";
import { brainNotes, brainSearch } from "@/modules/brain/lib/bindings";
import type { ToolContext } from "./context";

// Read-only Koden Brain tools — how the Librarian grounds its answers. No
// approval needed (they never mutate), and no curation surface on purpose:
// memory writes stay propose-only, gated by the review inbox.

const MAX_HITS = 15;

export function buildBrainTools(_ctx: ToolContext) {
  return {
    brain_search: tool({
      description:
        "Search the Koden Brain index (project files + memory notes) with a lexical query. Returns scored hits `{project, path, score}` — cite the project + path when you use one. Omit `project` to search every indexed project.",
      inputSchema: z.object({
        query: z.string().describe("Search terms (BM25 lexical, not regex)."),
        project: z
          .string()
          .nullable()
          .optional()
          .describe(
            "Project id to scope to (as returned in hits / brain_notes). Omit for all projects.",
          ),
        limit: z.number().int().min(1).max(MAX_HITS).optional(),
      }),
      execute: async ({ query, project, limit }) =>
        brainSearch(query, project ?? null, Math.min(limit ?? MAX_HITS, MAX_HITS)),
    }),

    brain_notes: tool({
      description:
        "List the structured memory notes (cards) the Koden Brain holds: id, title, type, status, path, and anchors. Omit `project` for every indexed project. Use read_file on a note's path for its full body.",
      inputSchema: z.object({
        project: z
          .string()
          .nullable()
          .optional()
          .describe("Project id to scope to. Omit for all projects."),
      }),
      execute: async ({ project }) => brainNotes(project ?? null),
    }),
  } as const;
}
