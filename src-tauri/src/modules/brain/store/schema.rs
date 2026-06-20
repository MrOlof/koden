//! SQL DDL for the Brain index — ONE SQLite file unifying the FTS5 lexical index,
//! the freshness manifest, and a versioned meta table (ADR-006 storage model).
//! AST-graph, memory-note, and vector tables arrive via P1/P2/P5 migrations
//! keyed off `schema_version`.

/// Current durable schema version. Bumping it triggers `migrate`.
pub const SCHEMA_VERSION: i64 = 1;

/// Idempotent base DDL (safe to run on every open).
pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS brain_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Per-file freshness manifest. `path` is root-relative, forward-slash normalized
-- (MegaSync-portable). `fts_rowid` links the row to its FTS document so a changed
-- file's old document can be deleted in O(1) before re-insert.
CREATE TABLE IF NOT EXISTS files (
    project_id TEXT NOT NULL,
    path       TEXT NOT NULL,
    hash       TEXT NOT NULL,
    size       INTEGER NOT NULL,
    fts_rowid  INTEGER NOT NULL,
    PRIMARY KEY (project_id, path)
);
-- UNIQUE: one FTS document per file row; the search JOIN assumes a 1:1 mapping.
CREATE UNIQUE INDEX IF NOT EXISTS files_fts_rowid ON files(fts_rowid);
CREATE INDEX IF NOT EXISTS files_project   ON files(project_id);

-- Lexical FTS5 index over PRE-TOKENIZED streams (CONCEPT [DP-3]); columns carry
-- already split/stemmed token streams. The `ascii` tokenizer is a near-passthrough
-- (split on non-alnum, no folding) so it re-splits our space-joined ASCII token
-- stream into the exact same terms on both the index and query sides — required
-- by EXECUTION_PLAN §0.6 (unicode61's folding/diacritic handling would desync the
-- two sides once a stored token contains non-ASCII). Per-column bm25() weights
-- give the first-class path/symbol/content field weighting. `symbols` is empty
-- until P2 (tree-sitter) populates it.
CREATE VIRTUAL TABLE IF NOT EXISTS code_fts USING fts5(
    path,
    symbols,
    content,
    tokenize = 'ascii'
);
"#;
