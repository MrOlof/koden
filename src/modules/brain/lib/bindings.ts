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
