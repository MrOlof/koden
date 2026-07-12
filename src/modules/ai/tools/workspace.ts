import { tool } from "ai";
import { z } from "zod";
import { flushDocs, hydrateDocs, useDocsStore } from "@/modules/workspace-docs";
import type { ToolContext } from "./context";

// Workspace-docs tools: the Librarian's window into the Tasks/Notes/Board
// panes. Reads auto-execute; every write sets needsApproval so the user
// confirms each action in-chat (same gate as write_file / bash_run). Writes
// go through the docs store only (primary + staggered backup persistence,
// ADR-001), never raw file IO, and flush to disk before reporting success.
// No delete surface on purpose.

const MAX_NOTE_CHARS = 20_000;

async function docsState() {
  if (!useDocsStore.getState().hydrated) await hydrateDocs();
  return useDocsStore.getState();
}

function noteBody(content: string) {
  return content.length > MAX_NOTE_CHARS
    ? {
        chars: content.length,
        truncated: true,
        content: content.slice(0, MAX_NOTE_CHARS),
      }
    : { chars: content.length, content };
}

export function buildWorkspaceTools(_ctx: ToolContext) {
  return {
    workspace_tasks: tool({
      description:
        "List the workspace Tasks panes: every task list with its items {id, text, done}. Doc ids are opaque keys; identify a list by its items. Call before any workspace_task_* write. Auto-executes.",
      inputSchema: z.object({}),
      execute: async () => {
        const s = await docsState();
        const lists = Object.entries(s.tasks).map(([docId, list]) => ({
          docId,
          updatedAt: list.updatedAt,
          open: list.items.filter((t) => !t.done).length,
          done: list.items.filter((t) => t.done).length,
          items: list.items.map((t) => ({
            id: t.id,
            text: t.text,
            done: t.done,
          })),
        }));
        return { lists };
      },
    }),

    workspace_notes: tool({
      description:
        "Read the workspace Notes panes (markdown scratchpads). Pass docId for one note; omit for all. Auto-executes.",
      inputSchema: z.object({
        docId: z
          .string()
          .nullable()
          .optional()
          .describe("Note doc id (from a previous call). Omit for all notes."),
      }),
      execute: async ({ docId }) => {
        const s = await docsState();
        if (docId != null) {
          const doc = s.notes[docId];
          if (!doc)
            return {
              error: `unknown note '${docId}'`,
              known: Object.keys(s.notes),
            };
          return { docId, updatedAt: doc.updatedAt, ...noteBody(doc.content) };
        }
        return {
          notes: Object.entries(s.notes).map(([id, doc]) => ({
            docId: id,
            updatedAt: doc.updatedAt,
            ...noteBody(doc.content),
          })),
        };
      },
    }),

    workspace_boards: tool({
      description:
        "List the workspace Board panes: columns in order, each with its cards {id, text}. Auto-executes.",
      inputSchema: z.object({}),
      execute: async () => {
        const s = await docsState();
        return {
          boards: Object.entries(s.boards).map(([docId, board]) => ({
            docId,
            updatedAt: board.updatedAt,
            columns: board.columns.map((col) => ({
              id: col.id,
              title: col.title,
              cards: col.cardIds.flatMap((cid) => {
                const card = board.cards[cid];
                return card ? [{ id: card.id, text: card.text }] : [];
              }),
            })),
          })),
        };
      },
    }),

    workspace_task_add: tool({
      description:
        "Add a task to a workspace Tasks list. Omit docId when exactly one list exists. Cannot create lists; the user opens Tasks panes. Asks for user approval.",
      inputSchema: z.object({
        text: z.string().min(1).describe("Task text. Concrete and actionable."),
        docId: z
          .string()
          .nullable()
          .optional()
          .describe(
            "Target list (from workspace_tasks). Omit when only one exists.",
          ),
      }),
      needsApproval: true,
      execute: async ({ text, docId }) => {
        const trimmed = text.trim();
        if (!trimmed) return { error: "empty task text" };
        const s = await docsState();
        const ids = Object.keys(s.tasks);
        let target = docId ?? null;
        if (target == null) {
          if (ids.length === 1) target = ids[0];
          else if (ids.length === 0)
            return {
              error:
                "no task list exists yet; ask the user to open a Tasks pane first",
            };
          else return { error: "multiple task lists; pass docId", lists: ids };
        } else if (!s.tasks[target]) {
          return { error: `unknown task list '${target}'`, lists: ids };
        }
        useDocsStore.getState().addTask(target, trimmed);
        const items = useDocsStore.getState().tasks[target]?.items ?? [];
        const added = items[items.length - 1];
        if (!added || added.text !== trimmed) return { error: "add failed" };
        await flushDocs();
        return {
          ok: true,
          docId: target,
          task: { id: added.id, text: added.text },
        };
      },
    }),

    workspace_task_set_done: tool({
      description:
        "Mark a workspace task done or not done by its id (from workspace_tasks). Idempotent. Asks for user approval.",
      inputSchema: z.object({
        id: z.string().describe("Task id from workspace_tasks."),
        done: z.boolean(),
      }),
      needsApproval: true,
      execute: async ({ id, done }) => {
        const s = await docsState();
        for (const [listId, list] of Object.entries(s.tasks)) {
          const item = list.items.find((t) => t.id === id);
          if (!item) continue;
          if (item.done === done)
            return { ok: true, id, done, changed: false, text: item.text };
          useDocsStore.getState().toggleTask(listId, id);
          await flushDocs();
          return { ok: true, id, done, changed: true, text: item.text };
        }
        return { error: `unknown task id '${id}'; call workspace_tasks first` };
      },
    }),

    workspace_note_append: tool({
      description:
        "Append markdown to an existing workspace note (docId from workspace_notes). Append-only: never rewrites or deletes existing content. Asks for user approval.",
      inputSchema: z.object({
        docId: z.string().describe("Note doc id from workspace_notes."),
        markdown: z.string().min(1).describe("Markdown to append."),
      }),
      needsApproval: true,
      execute: async ({ docId, markdown }) => {
        const s = await docsState();
        if (!s.notes[docId])
          return {
            error: `unknown note '${docId}'`,
            known: Object.keys(s.notes),
          };
        // Read fresh and set in one synchronous block so keystrokes the user
        // typed while the approval card was open are never clobbered.
        const cur = useDocsStore.getState().notes[docId]?.content ?? "";
        const next =
          cur === ""
            ? markdown
            : cur + (cur.endsWith("\n") ? "\n" : "\n\n") + markdown;
        useDocsStore.getState().setNote(docId, next);
        await flushDocs();
        return {
          ok: true,
          docId,
          appendedChars: markdown.length,
          totalChars: next.length,
        };
      },
    }),
  } as const;
}
