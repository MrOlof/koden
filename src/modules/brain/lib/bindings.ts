// Typed wrappers over the Koden Brain `#[tauri::command]` surface. DTO shapes
// mirror the Rust serde structs exactly:
//   Hit                 -> src-tauri/src/modules/brain/mod.rs (Hit)
//   Project             -> src-tauri/src/modules/brain/registry.rs (Project)
//   BrainStatus/Report  -> src-tauri/src/modules/brain/{mod.rs,commands.rs}
// Brain command params are single-word, so no snake_case/camelCase conversion.
import { invoke } from "@tauri-apps/api/core";

export type Hit = { project: string; path: string; score: number };

export type Project = { id: string; name: string; root: string };

export type BrainStatus =
  | { state: "warming"; pct: number }
  | { state: "ready" }
  | { state: "degraded"; reason: string };

export type ProjectStatus = { project: Project; files: number };

export type BrainStatusReport = { status: BrainStatus; projects: ProjectStatus[] };

/** Lexical (BM25 + weighted RRF) search. `project = null` searches every project. */
export function brainSearch(
  query: string,
  project: string | null = null,
  limit = 20,
): Promise<Hit[]> {
  return invoke<Hit[]>("brain_search", { project, query, limit });
}

export function brainIndexStatus(): Promise<BrainStatusReport> {
  return invoke<BrainStatusReport>("brain_index_status", {});
}

export function brainListProjects(): Promise<Project[]> {
  return invoke<Project[]>("brain_list_projects", {});
}

/** Register a new project root (returns the project) and trigger indexing. */
export function brainAddProject(path: string): Promise<Project> {
  return invoke<Project>("brain_add_project", { path });
}

/** Trigger a full reconcile (all projects, or one). Non-blocking on the worker. */
export function brainRescan(project: string | null = null): Promise<void> {
  return invoke<void>("brain_rescan", { project });
}

export type NoteSummary = {
  id: string;
  title: string;
  note_type: string | null;
  status: string | null;
  path: string;
  anchors: string[];
};

export type ProposalAction = "create" | "update" | "supersede" | "archive";

export type MemoryProposal = {
  project: string;
  signature: string;
  action: ProposalAction;
  target_id: string | null;
  title: string;
  detail: string;
  source: string;
  status: string;
};

/** Structured memory notes (cards). `project = null` = all. */
export function brainNotes(project: string | null = null): Promise<NoteSummary[]> {
  return invoke<NoteSummary[]>("brain_notes", { project });
}

/** Pending memory proposals (the review inbox). `project = null` = all. */
export function brainProposals(project: string | null = null): Promise<MemoryProposal[]> {
  return invoke<MemoryProposal[]>("brain_proposals", { project });
}

/** Run the memory doctor (queues proposals). `nowDate` = ISO YYYY-MM-DD. */
export function brainDoctor(
  project: string | null = null,
  nowDate: string | null = null,
): Promise<void> {
  return invoke<void>("brain_doctor", { project, nowDate });
}

/** Approve (reject=false) or decline (reject=true) a proposal. */
export function brainResolveProposal(
  project: string,
  signature: string,
  reject: boolean,
): Promise<void> {
  return invoke<void>("brain_resolve_proposal", { project, signature, reject });
}

/** Trigger a budgeted LLM reflect pass (P4 — the only token-spending path).
 *  `project = null` reflects every registered project. `nowDate` = ISO YYYY-MM-DD
 *  for the doctor findings in the digest. Off unless a ceiling > 0 is set. */
export function brainReflect(
  project: string | null = null,
  nowDate: string | null = null,
): Promise<void> {
  return invoke<void>("brain_reflect", { project, nowDate });
}

/** Set the reflect monthly spend ceiling (USD). `0` disables reflect entirely. */
export function brainSetBudget(ceilingUsd: number): Promise<void> {
  return invoke<void>("brain_set_budget", { ceilingUsd });
}

/** Run stale-ADR / memory curation (V2 Flow G). `project = null` curates all.
 *  `nowDate` = ISO YYYY-MM-DD for the staleness signal. Decisive stale notes get a
 *  $0 archive proposal; borderline ones escalate to a budget-gated LLM judgment.
 *  Archive-biased + human-gated; results land in the proposal inbox. */
export function brainCurate(
  project: string | null = null,
  nowDate: string | null = null,
): Promise<void> {
  return invoke<void>("brain_curate", { project, nowDate });
}

/** Reflect budget meter: `[ceilingUsd, spentTotalUsd]`. */
export function brainBudgetStatus(): Promise<[number, number]> {
  return invoke<[number, number]>("brain_budget_status", {});
}

/** A pane recoverable from the previous session (P4 crash-resume). Field names
 *  mirror the Rust `RecoveredPane` serde struct (snake_case). */
export type RecoveredPane = {
  key: string;
  last_kind: string;
  agent: string | null;
  cwd: string;
  project: string | null;
  claude_session_id: string | null;
};

/** Panes recoverable from the previous session — drives the resume cards. */
export function brainRecoveredPanes(): Promise<RecoveredPane[]> {
  return invoke<RecoveredPane[]>("brain_recovered_panes", {});
}

export type Gist = { bytes: string; fingerprint: string; sources: string[] };

/** Build the cache-stable gist for a project + intent (blank intent → cold-start
 *  synthesis). `null` if the index isn't ready. */
export function brainBuildGist(
  project: string,
  intent: string,
  budget = 800,
): Promise<Gist | null> {
  return invoke<Gist | null>("brain_build_gist", { project, intent, budget });
}

/** Longest-prefix match a cwd against registered project roots → project id.
 *  Case-insensitive: Windows/macOS paths fold case, and a missed match only
 *  means a silently un-injected gist, so over-matching here is the safe error. */
export async function resolveProjectForCwd(cwd: string): Promise<string | null> {
  const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  const c = norm(cwd);
  let best: { id: string; rootLen: number } | null = null;
  for (const p of await brainListProjects()) {
    const root = norm(p.root);
    if ((c === root || c.startsWith(`${root}/`)) && (!best || root.length > best.rootLen)) {
      best = { id: p.id, rootLen: root.length };
    }
  }
  return best?.id ?? null;
}
