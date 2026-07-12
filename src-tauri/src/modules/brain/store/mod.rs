//! Storage/search layer. ONE SQLite file behind a `SearchIndex` trait so tantivy
//! could swap in later without schema churn (ADR-006).

pub mod changes;
pub mod graph;
pub mod journal;
pub mod migrate;
pub mod plan;
pub mod schema;
pub mod sqlite;
pub mod temporal;

pub use changes::{detect_changes_readonly, AffectedFile, DetectMode, DetectedChanges};
pub use graph::{graph_readonly, BrainGraph};
pub use plan::{plan_context_readonly, PlanAdvisory, PlanContext};
pub use temporal::{
    changed_between_readonly, hotspots_readonly, ChangedBetween, ChangedFile, HotspotRow, Hotspots,
};

pub use sqlite::{
    budget_state_readonly, code_impact_readonly, file_count_readonly, file_count_with_conn,
    file_touch_with_conn, get_symbol_readonly, gist_notes_with_conn, librarian_config_readonly,
    librarian_ledger_readonly, recent_activity_with_conn, ActivityRow,
    list_memory_changes_readonly, list_notes_readonly, list_notes_with_conn,
    list_proposals_readonly, open_readonly_snapshot, pending_proposals_readonly,
    prepare_file, PreparedFile,
    project_fingerprint_readonly,
    project_fingerprint_with_conn,
    project_temporal_digest_with_conn, search_excluding_tests_with_conn, search_readonly,
    search_readonly_excluding_tests, search_with_conn, search_with_weights,
    semantic_meta_readonly,
    symbols_for_path_readonly, symbols_for_path_with_conn, GistNote, SearchWeights, SqliteIndex,
    SEARCH_LEG_LABELS,
};

use crate::modules::brain::Hit;

/// The retrieval seam — one query path, two consumers (interactive search +
/// gist synthesis). Realized over SQLite/FTS5 in P0.
pub trait SearchIndex {
    fn search(&self, project: Option<&str>, query: &str, limit: usize) -> rusqlite::Result<Vec<Hit>>;
}
