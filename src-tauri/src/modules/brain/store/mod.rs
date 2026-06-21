//! Storage/search layer. ONE SQLite file behind a `SearchIndex` trait so tantivy
//! could swap in later without schema churn (ADR-006).

pub mod migrate;
pub mod schema;
pub mod sqlite;

pub use sqlite::{
    code_impact_readonly, file_count_readonly, file_count_with_conn, get_symbol_readonly,
    list_notes_readonly, list_notes_with_conn, list_proposals_readonly, open_readonly_snapshot,
    project_fingerprint_readonly, project_fingerprint_with_conn, search_readonly, search_with_conn,
    symbols_for_path_readonly, symbols_for_path_with_conn, SqliteIndex,
};

use crate::modules::brain::Hit;

/// The retrieval seam — one query path, two consumers (interactive search +
/// gist synthesis). Realized over SQLite/FTS5 in P0.
pub trait SearchIndex {
    fn search(&self, project: Option<&str>, query: &str, limit: usize) -> rusqlite::Result<Vec<Hit>>;
}
