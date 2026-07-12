import { buildManagedAgentTools } from "./agent";
import { buildBrainTools } from "./brain";
import { buildEditTools } from "./edit";
import { buildFsTools } from "./fs";
import { buildLayoutTools } from "./layout";
import { buildSearchTools } from "./search";
import { buildShellTools } from "./shell";
import { buildSubagentTools } from "./subagent";
import { buildTerminalTools } from "./terminal";
import { buildTerminalTargetTools } from "./terminals";
import { buildTodoTools } from "./todo";
import { buildWorkspaceTools } from "./workspace";

export { resolvePath, type ToolContext } from "./context";

/**
 * AI tool definitions.
 *
 * Approval policy:
 *  - Read-only tools (`read_file`, `list_directory`, `grep`, `glob`)
 *    auto-execute, but go through the security guard which refuses obvious
 *    secret paths (.env*, .ssh/, credentials, etc.).
 *  - Mutating tools (`write_file`, `edit`, `multi_edit`, `create_directory`,
 *    `run_command`, `workspace_task_add`, `workspace_task_set_done`,
 *    `workspace_note_append`) require explicit user approval — the AI SDK
 *    pauses on tool-call and surfaces a `tool-approval-request` part that
 *    the UI renders as a confirmation card.
 *  - `edit` / `multi_edit` additionally enforce a read-before-edit invariant
 *    (the model must have called read_file on the path earlier in the
 *    session).
 *  - Layout tools (`workspace_open_tab`, `workspace_split_pane`,
 *    `workspace_focus_pane`, `workspace_layout_state`) auto-execute without
 *    approval: every action is immediately visible, reversible with one
 *    click, and non-destructive. Create/arrange only — no close/delete
 *    tools by design (ADR-017 addendum).
 *  - Terminal targeting (`workspace_list_terminals`, `terminal_read`,
 *    `terminal_send`) is tiered: list/read auto-execute (redacted, Privacy
 *    tabs refused); `terminal_send` with submit:false types without Enter and
 *    auto-executes; submit:true carries a DYNAMIC `needsApproval` — gated by
 *    the approval card unless the user armed the hands-free preference
 *    (ADR-017 addendum).
 *
 * The model sees absolute paths only after they are resolved against the
 * active terminal's cwd (provided via `getCwd`); it should not invent paths
 * outside that.
 */
export function buildTools(ctx: import("./context").ToolContext) {
  return {
    ...buildBrainTools(ctx),
    ...buildFsTools(ctx),
    ...buildEditTools(ctx),
    ...buildLayoutTools(ctx),
    ...buildSearchTools(ctx),
    ...buildShellTools(ctx),
    ...buildSubagentTools(ctx),
    ...buildTerminalTools(ctx),
    ...buildTerminalTargetTools(ctx),
    ...buildTodoTools(ctx),
    ...buildManagedAgentTools(ctx),
    ...buildWorkspaceTools(ctx),
  } as const;
}

export type ChatTools = ReturnType<typeof buildTools>;
