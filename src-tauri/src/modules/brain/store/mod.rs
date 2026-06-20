//! Storage/search layer. ONE SQLite file behind a `SearchIndex` trait so tantivy
//! could swap in later without schema churn (ADR-006).

pub mod migrate;
pub mod schema;
pub mod sqlite;

pub use sqlite::{
    file_count_readonly, list_notes_readonly, search_readonly, search_with_conn, SqliteIndex,
};

use crate::modules::brain::Hit;

/// The retrieval seam — one query path, two consumers (interactive search +
/// gist synthesis). Realized over SQLite/FTS5 in P0.
pub trait SearchIndex {
    fn search(&self, project: Option<&str>, query: &str, limit: usize) -> rusqlite::Result<Vec<Hit>>;
}
