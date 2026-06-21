//! Cold-start query synthesis (CONCEPT §6 Flow C step 2). When an agent launches
//! with no explicit task, synthesize an intent query from ambient project signal.
//!
//! MUST be deterministic given project state (no wall-clock / random / volatile
//! ordering) or the synthesized intent would change between identical relaunches
//! and bust the gist's prompt-cache stability. v1 signal: the project name + its
//! curated memory-note titles (sorted, the "what is this project about" signal).
//! git-HEAD / changed-files / recent-files signal is a documented refinement.

use std::path::Path;

use crate::modules::brain::store;

const MAX_NOTE_TITLES: usize = 8;

/// Synthesize a cold-start intent query. Deterministic for a given project state.
pub fn synthesize_intent(db_path: &Path, project_id: &str, project_name: &str) -> String {
    let mut parts: Vec<String> = vec![project_name.to_string()];
    if let Ok(notes) = store::list_notes_readonly(db_path, Some(project_id)) {
        // notes come back sorted by id → deterministic order.
        for n in notes.iter().take(MAX_NOTE_TITLES) {
            if !n.title.is_empty() {
                parts.push(n.title.clone());
            }
        }
    }
    parts.join(" ")
}
