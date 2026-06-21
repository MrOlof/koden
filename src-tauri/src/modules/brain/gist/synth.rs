//! Cold-start query synthesis (CONCEPT §6 Flow C step 2). When an agent launches
//! with no explicit task, synthesize an intent query from ambient project signal.
//!
//! MUST be deterministic given project state (no wall-clock / random / volatile
//! ordering) or the synthesized intent would change between identical relaunches
//! and bust the gist's prompt-cache stability. v1 signal: the project name + its
//! curated memory-note titles (sorted, the "what is this project about" signal).
//! git-HEAD / changed-files / recent-files signal is a documented refinement.

use std::path::Path;

use rusqlite::Connection;

use crate::modules::brain::store;

const MAX_NOTE_TITLES: usize = 8;

/// Synthesize a cold-start intent query via a fresh read-only connection.
/// Deterministic for a given project state.
pub fn synthesize_intent(db_path: &Path, project_id: &str, project_name: &str) -> String {
    let conn = store::open_readonly_snapshot(db_path).ok();
    synthesize_intent_on_conn(conn.as_ref(), project_id, project_name)
}

/// Synthesize a cold-start intent over a caller-supplied snapshot (`None` →
/// name-only), so the synthesis shares the gist build's single snapshot.
pub fn synthesize_intent_on_conn(
    conn: Option<&Connection>,
    project_id: &str,
    project_name: &str,
) -> String {
    let mut parts: Vec<String> = vec![project_name.to_string()];
    if let Some(notes) = conn.and_then(|c| store::list_notes_with_conn(c, Some(project_id)).ok()) {
        // notes come back sorted by id → deterministic order.
        for n in notes.iter().take(MAX_NOTE_TITLES) {
            if !n.title.is_empty() {
                parts.push(n.title.clone());
            }
        }
    }
    parts.join(" ")
}
