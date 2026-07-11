import { tool } from "ai";
import { z } from "zod";
import {
  brainBudgetStatus,
  brainBuildGist,
  brainIndexStatus,
  brainLibrarianActivity,
  brainLibrarianStatus,
  brainMemoryChanges,
  brainNotes,
  brainProposals,
  brainSearch,
} from "@/modules/brain/lib/bindings";
import type { ToolContext } from "./context";

// Read-only Koden Brain tools — how the Librarian grounds its answers. No
// approval needed (they never mutate), and no curation surface on purpose:
// the curation ENGINE runs autonomously on the Rust worker with snapshot undo
// (ADR-018), but CHAT can only read — reverting a change or switching curation
// mode happens in the Brain pane / Settings, never from here (ADR-017 holds
// for the chat toolset).

const MAX_HITS = 15;

export function buildBrainTools(_ctx: ToolContext) {
  return {
    brain_search: tool({
      description:
        "Search the Koden Brain index (project files + memory notes) with a lexical query. Returns scored hits `{project, path, score}` — cite the project + path when you use one. Omit `project` to search every indexed project. Hit paths are RELATIVE TO THAT PROJECT'S ROOT (roots come from brain_status) — join them before read_file; a bare relative path resolves against the terminal cwd, which may be a different project.",
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
        "List the structured memory notes (cards) the Koden Brain holds: id, title, type, status, path, and anchors. Omit `project` for every indexed project. For a note's full body, join its project's root (from brain_status) with the note path before read_file — note paths are project-relative, and bare relative paths resolve against the terminal cwd.",
      inputSchema: z.object({
        project: z
          .string()
          .nullable()
          .optional()
          .describe("Project id to scope to. Omit for all projects."),
      }),
      execute: async ({ project }) => brainNotes(project ?? null),
    }),

    brain_status: tool({
      description:
        "Report what the Koden Brain currently has indexed: overall state (`warming` with %, `ready`, or `degraded`) and, per project, its id, name, root path, and indexed file count. The brain is a LIVE index (a file watcher keeps it current) — there is no discrete 'index run' timestamp; this is the up-to-the-moment state. Use it to answer 'what's indexed', 'how many files', 'which projects', or 'is the brain ready'.",
      inputSchema: z.object({}),
      execute: async () => {
        const report = await brainIndexStatus();
        return {
          state: report.status,
          note: "Live watcher-maintained index (no discrete run timestamp); counts are current.",
          projects: report.projects.map((p) => ({
            id: p.project.id,
            name: p.project.name,
            root: p.project.root,
            files: p.files,
          })),
          totalFiles: report.projects.reduce((n, p) => n + p.files, 0),
        };
      },
    }),

    brain_gist: tool({
      description:
        "Build the Koden Brain's curated briefing for a project: a compact, freshness-aware summary grounded in the index + memory notes, plus the exact source files/notes it drew from. Use this for 'what do we know about this project', 'give me a briefing/overview', or to orient before answering. `intent` focuses the gist (a topic/question); omit for a general overview. Cite the returned `sources`.",
      inputSchema: z.object({
        project: z
          .string()
          .nullable()
          .optional()
          .describe(
            "Project id (from brain_status / search hits). Omit when there is only one indexed project.",
          ),
        intent: z
          .string()
          .optional()
          .describe("Topic or question to focus the briefing on. Omit for a general overview."),
      }),
      execute: async ({ project, intent }) => {
        // Resolve the project: use the given id, else the sole indexed one, else
        // ask the model to pick (never guess across multiple projects).
        const report = await brainIndexStatus();
        const ids = report.projects.map((p) => p.project.id);
        let pid = project ?? null;
        if (!pid) {
          if (ids.length === 1) pid = ids[0];
          else
            return {
              error:
                ids.length === 0
                  ? "No indexed projects."
                  : "Multiple projects indexed — pass `project`.",
              projects: report.projects.map((p) => ({ id: p.project.id, name: p.project.name })),
            };
        }
        if (!ids.includes(pid))
          return { error: `Unknown project '${pid}'.`, projects: ids };
        const gist = await brainBuildGist(pid, intent ?? "overview of this project");
        if (!gist) return { error: `No gist available for '${pid}' (index may be warming).` };
        return { project: pid, briefing: gist.bytes, sources: gist.sources };
      },
    }),

    brain_proposals: tool({
      description:
        "List the Brain's memory-change activity: recent APPLIED changes (the Librarian curates autonomously by default — each auto-applied with an undo snapshot, plus any the user reverted) and anything still PENDING review (review-first mode, or a change that couldn't apply). Each entry has action (create/update/archive/supersede), title, detail, and status. Read-only — reverting a change or approving a pending one happens in the Brain pane, not here. Omit `project` for every project.",
      inputSchema: z.object({
        project: z
          .string()
          .nullable()
          .optional()
          .describe("Project id to scope to. Omit for all projects."),
      }),
      execute: async ({ project }) => {
        const [proposals, changes] = await Promise.all([
          brainProposals(project ?? null),
          brainMemoryChanges(project ?? null).catch(() => []),
        ]);
        return {
          pending: proposals.length,
          proposals: proposals.map((p) => ({
            project: p.project,
            action: p.action,
            title: p.title,
            detail: p.detail,
            status: p.status,
          })),
          recentChanges: changes.slice(0, 20).map((c) => ({
            project: c.project,
            action: c.action,
            title: c.title,
            status: c.status, // "applied" | "reverted"
            autoApplied: c.auto_applied,
            revertible: c.revertible,
            // Stacked changes on one note revert newest-first; this marks the gated older ones.
            blockedByNewer: c.blocked_by_newer,
          })),
          note: "Read-only. Reverts and approvals happen in the Brain pane's Memory tab, not here.",
        };
      },
    }),

    brain_librarian_info: tool({
      description:
        "Report the Librarian's own configuration and spend: the LLM provider + model it uses for memory curation, its curation mode ('autonomous' = applies changes itself with undo, 'review' = changes wait in the inbox), the budget ceiling and cumulative spend (USD), pending-proposal count, and recent paid calls. Use for 'what model are you / how much have you spent / what have you done lately'. Read-only.",
      inputSchema: z.object({}),
      execute: async () => {
        const [status, [ceilingUsd, spentUsd], activity] = await Promise.all([
          brainLibrarianStatus(),
          brainBudgetStatus(),
          brainLibrarianActivity(),
        ]);
        return {
          provider: status.provider,
          model: status.model,
          curationMode: status.curation_mode,
          ceilingUsd,
          spentUsd,
          pendingProposals: activity.pending_proposals,
          recentCalls: activity.calls.slice(0, 10),
        };
      },
    }),
  } as const;
}
