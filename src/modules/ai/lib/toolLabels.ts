// Friendly tool labels — single source of truth for every place the UI
// names a tool (chat chips, approval cards, the status pill). Raw tool
// names never reach the user; unknown tools fall back to a prettified
// Sentence-case form of the snake_case name.

type LabelFn = (input: Record<string, unknown>) => string;

// Static titles, Koden voice. One entry per tool in buildTools().
const TOOL_TITLES: Record<string, string> = {
  // Brain
  brain_search: "Searching the brain",
  brain_notes: "Reading memory notes",
  brain_status: "Checking the brain index",
  brain_gist: "Building a project briefing",
  brain_proposals: "Reading recent memory changes",
  brain_librarian_info: "Checking Librarian config",
  // Files
  read_file: "Reading a file",
  list_directory: "Listing a directory",
  write_file: "Writing a file",
  create_directory: "Creating a directory",
  edit: "Editing a file",
  multi_edit: "Editing a file",
  // Search
  grep: "Searching the code",
  glob: "Matching files",
  // Shell
  bash_run: "Running a command",
  bash_background: "Starting a background job",
  bash_logs: "Reading job logs",
  bash_list: "Listing background jobs",
  bash_kill: "Stopping a background job",
  // Terminal
  suggest_command: "Suggesting a command",
  get_terminal_output: "Reading the terminal",
  open_preview: "Opening a preview",
  // Agents
  run_subagent: "Spawning a subagent",
  spawn_coding_agent: "Spawning a coding agent",
  send_to_agent: "Messaging an agent",
  read_agent_output: "Reading agent output",
  // Plan
  todo_write: "Updating the plan",
  // Workspace
  workspace_tasks: "Reading workspace tasks",
  workspace_notes: "Reading workspace notes",
  workspace_boards: "Listing workspace boards",
  workspace_task_add: "Adding a task",
  workspace_task_set_done: "Checking off a task",
  workspace_note_append: "Appending to a note",
  // Layout
  workspace_open_tab: "Opening a tab",
  workspace_split_pane: "Splitting the pane",
  workspace_focus_pane: "Focusing a pane",
  workspace_layout_state: "Reading the layout",
};

// Input-aware labels for the live status pill — folds the interesting bit
// of the input into the sentence. Anything not listed falls back to the
// static title above.
const TOOL_STATUS_LABELS: Record<string, LabelFn> = {
  brain_search: (i) =>
    `Searching the brain for ${ellipsize(String(i.query ?? ""), 40)}`,
  read_file: (i) => `Reading ${shortPath(i.path)}`,
  list_directory: (i) => `Listing ${shortPath(i.path)}`,
  grep: (i) => `Grepping ${ellipsize(String(i.pattern ?? ""), 40)}`,
  glob: (i) => `Globbing ${ellipsize(String(i.pattern ?? ""), 40)}`,
  edit: (i) => `Editing ${shortPath(i.path)}`,
  multi_edit: (i) => `Editing ${shortPath(i.path)}`,
  write_file: (i) => `Writing ${shortPath(i.path)}`,
  create_directory: (i) => `Creating ${shortPath(i.path)}`,
  bash_run: (i) => `Running ${ellipsize(String(i.command ?? ""), 60)}`,
  bash_background: (i) => `Spawning ${ellipsize(String(i.command ?? ""), 60)}`,
  suggest_command: (i) =>
    `Suggesting ${ellipsize(String(i.command ?? ""), 60)}`,
  todo_write: (i) =>
    `Updating plan (${Array.isArray(i.todos) ? i.todos.length : 0} items)`,
  run_subagent: (i) => `Spawning ${String(i.type ?? "subagent")} subagent`,
  workspace_open_tab: (i) => `Opening a ${String(i.kind ?? "workspace")} tab`,
  workspace_split_pane: (i) => `Adding a ${String(i.kind ?? "new")} pane`,
};

/** Friendly static title for a tool. Never returns the raw name. */
export function toolTitle(toolName: string): string {
  return TOOL_TITLES[toolName] ?? prettifyToolName(toolName);
}

/** Friendly label with the input folded in, for the status pill. */
export function toolStatusLabel(
  toolName: string,
  input: Record<string, unknown>,
): string {
  const fn = TOOL_STATUS_LABELS[toolName];
  return fn ? fn(input) : toolTitle(toolName);
}

/** snake_case → Sentence case, for tools we don't know about. */
export function prettifyToolName(toolName: string): string {
  const s = toolName.replace(/[_-]+/g, " ").trim().toLowerCase();
  if (!s) return toolName;
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function shortPath(p: unknown): string {
  if (typeof p !== "string") return "";
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i === -1 ? p : p.slice(i + 1);
}

function ellipsize(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}
