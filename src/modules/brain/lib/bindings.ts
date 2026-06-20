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
