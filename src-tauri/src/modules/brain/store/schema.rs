//! SQL DDL for the Brain index — ONE SQLite file unifying the FTS5 lexical index,
//! the freshness manifest, and a versioned meta table (ADR-006 storage model).
//! AST-graph, memory-note, and vector tables arrive via P1/P2/P5 migrations
//! keyed off `schema_version`.

/// Current durable schema version. Bumping it triggers `migrate`.
/// v2: added the `notes` table (P1 native memory store).
/// v3: added `proposals` + `reject_signatures` (P1 doctor → proposal review loop).
/// v4: added the AST graph — `code_nodes` + `code_imports` + `code_edges` (P2).
/// v5: `code_nodes.start_col` (getter/setter PK fix); upgrades rebuild derived
///     file tables so the AST-fed `symbols` column is backfilled.
/// v6: added `brain_budget` (singleton spend ceiling/total) + `brain_budget_ledger`
///     (P4 budgeted reflect). CANONICAL/preserved — NOT in the upgrade DROP batch.
///     NOTE: bumping SCHEMA_VERSION intentionally rotates every gist cache KEY
///     (it is folded into the blake3 key in `gist/mod.rs`), so already-indexed
///     projects take a one-time agent-prompt-cache miss on the first post-upgrade
///     relaunch — correct and expected, no stored cache to invalidate.
/// v7: added `brain_semantic_meta` (the `embedderId` header, P5). CANONICAL/preserved.
///     Empty in v1 (no embedder); set when the `semantic` feature is enabled, so a
///     later build can detect a model/dimension mismatch and rebuild. The vector
///     table (`brain_vectors`) is created LAZILY at enablement — not here. As with
///     every bump, this rotates the gist cache key (see the v6 note) — one-time
///     post-upgrade agent-prompt-cache miss, expected.
/// v8: `notes.supersedes` (the FORWARD supersession edge, V2 curation Flow G).
///     `notes` is DERIVED-from-disk (rebuilt by `scan_project_memory`), so it joins
///     the upgrade rebuild set — the next warm scan repopulates it with the column.
/// v9: `files.accessed_at_ms` + `files.accessed_count` (V2 temporal re-rank [DP-12]).
///     `files` is DERIVED (rebuilt by the index walk); the upgrade now DROPs it (was
///     DELETE) so the new columns backfill. Recency/frequency feed a deterministic,
///     snapshot-stable multiplicative boost — stamped only on a real content change,
///     so an unchanged relaunch re-derives a byte-identical gist.
pub const SCHEMA_VERSION: i64 = 9;

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
    -- V2 temporal re-rank ([DP-12]): epoch-ms of the last meaningful touch (stamped
    -- on a real content change) + a touch counter. STORED so search reads them off
    -- the pinned WAL snapshot — never `now()` on the read side — keeping the gist
    -- byte-identity gate intact. Both default 0 (no boost) for legacy/unstamped rows.
    accessed_at_ms INTEGER NOT NULL DEFAULT 0,
    accessed_count INTEGER NOT NULL DEFAULT 0,
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

-- Structured memory notes (P1). The note FILES are made searchable by the code
-- walk; this table is the typed/queryable layer for cards, anchors, the doctor,
-- and proposals. `anchors` is a JSON array. Keyed by frontmatter id per project.
CREATE TABLE IF NOT EXISTS notes (
    project_id       TEXT NOT NULL,
    id               TEXT NOT NULL,
    path             TEXT NOT NULL,
    note_type        TEXT,
    status           TEXT,
    title            TEXT,
    scope            TEXT,
    provenance       TEXT,
    created          TEXT,
    revalidate_after TEXT,
    supersedes       TEXT,   -- forward edge: this note supersedes <id> (V2 Flow G)
    superseded_by    TEXT,
    anchors          TEXT,
    hash             TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
);
CREATE INDEX IF NOT EXISTS notes_project ON notes(project_id);

-- Human-gated proposal queue (P1). Brain-owned, local-only (rebuildable by
-- re-running the doctor) — NEVER auto-applied to user files. `signature` is the
-- plain-join proposalSignature (dedup PK). Status: pending|applied|rejected.
CREATE TABLE IF NOT EXISTS proposals (
    project_id TEXT NOT NULL,
    signature  TEXT NOT NULL,
    action     TEXT NOT NULL,   -- create|update|supersede|archive
    target_id  TEXT,
    title      TEXT NOT NULL,
    detail     TEXT NOT NULL,
    source     TEXT NOT NULL,   -- doctor|reflect
    status     TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    PRIMARY KEY (project_id, signature)
);
CREATE INDEX IF NOT EXISTS proposals_project_status ON proposals(project_id, status);

-- Persisted reject signatures (djb2 over scope|action|normalized-title) so a
-- declined proposal does not reappear on the next doctor pass (CONCEPT Flow G).
CREATE TABLE IF NOT EXISTS reject_signatures (
    project_id TEXT NOT NULL,
    reject_sig TEXT NOT NULL,
    PRIMARY KEY (project_id, reject_sig)
);

-- Budgeted-reflect spend state (P4). CANONICAL/preserved (human/spend state, NOT
-- derivable from the file walk) — never listed in the upgrade DROP batch. Default-
-- OFF: ceiling 0.0 disables reflect entirely. `brain_budget` is a GLOBAL singleton
-- (one daemon-wide cumulative ceiling), keyed by the CHECK (id=1) row.
-- All `*_at` columns here are epoch MILLISECONDS (every Rust write uses
-- now_epoch_ms()); the seed below matches that unit.
CREATE TABLE IF NOT EXISTS brain_budget (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    ceiling_usd     REAL NOT NULL DEFAULT 0.0,
    spent_total_usd REAL NOT NULL DEFAULT 0.0,
    updated_at      INTEGER NOT NULL
);
INSERT OR IGNORE INTO brain_budget (id, ceiling_usd, spent_total_usd, updated_at)
    VALUES (1, 0.0, 0.0, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- Append-only spend ledger (P4). A row is reserved BEFORE the network call and
-- reconciled AFTER, so a crash mid-call leaves a 'reserved' row that the boot
-- sweep charges at its estimate (over-counts a crash, never leaks free spend).
CREATE TABLE IF NOT EXISTS brain_budget_ledger (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    status          TEXT NOT NULL CHECK (status IN ('reserved','spent')),
    est_cost_usd    REAL NOT NULL,
    actual_cost_usd REAL,            -- NULL until reconcile
    model           TEXT NOT NULL,
    reserved_at     INTEGER NOT NULL,
    reconciled_at   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_ledger_reserved
    ON brain_budget_ledger (status) WHERE status = 'reserved';

-- Librarian LLM selection: which provider/model the budgeted reflect+curate path
-- uses. CANONICAL/preserved singleton (a human choice, NOT derived from disk) — so
-- it lives in the base DDL (created + seeded on every open via INSERT OR IGNORE) and
-- is absent from the upgrade DROP batch. Defaults to the historical Anthropic Haiku
-- path so existing installs are byte-for-byte unchanged. `*_rate_mtok` are
-- $/million-tokens (the frontend MODEL_PRICING unit; reflect converts to $/token);
-- local/free providers store 0. The API key is read at call time from the per-
-- provider `koden-ai` keyring account (e.g. openai-api-key) — never stored here.
CREATE TABLE IF NOT EXISTS brain_librarian (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    provider      TEXT NOT NULL DEFAULT 'anthropic',
    model         TEXT NOT NULL DEFAULT 'claude-haiku-4-5',
    base_url      TEXT NOT NULL DEFAULT '',
    in_rate_mtok  REAL NOT NULL DEFAULT 1.0,
    out_rate_mtok REAL NOT NULL DEFAULT 5.0,
    updated_at    INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO brain_librarian (id) VALUES (1);

-- Semantic embedderId header (P5). CANONICAL/preserved singleton. Empty in v1 (no
-- embedder compiled); set when the `semantic` feature is enabled so a later build
-- can detect a model/dimension change and rebuild the vector index rather than
-- serve stale embeddings. The vector table itself is created LAZILY at enablement.
CREATE TABLE IF NOT EXISTS brain_semantic_meta (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    embedder_id TEXT NOT NULL DEFAULT '',
    dims        INTEGER NOT NULL DEFAULT 0,
    built_at    INTEGER
);
INSERT OR IGNORE INTO brain_semantic_meta (id, embedder_id, dims) VALUES (1, '', 0);

-- AST graph (P2). `code_nodes` = tree-sitter definitions; `code_imports` = raw
-- per-file import specifiers; `code_edges` = RESOLVED file→file import edges,
-- rebuilt as a pure function of (code_imports, indexed file set) so incremental
-- relink and a full rebuild provably converge.
CREATE TABLE IF NOT EXISTS code_nodes (
    project_id TEXT NOT NULL,
    path       TEXT NOT NULL,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    start_col  INTEGER NOT NULL,
    PRIMARY KEY (project_id, path, name, kind, start_line, start_col)
);
CREATE INDEX IF NOT EXISTS code_nodes_name ON code_nodes(project_id, name);
CREATE INDEX IF NOT EXISTS code_nodes_path ON code_nodes(project_id, path);

CREATE TABLE IF NOT EXISTS code_imports (
    project_id TEXT NOT NULL,
    src_path   TEXT NOT NULL,
    spec       TEXT NOT NULL,
    PRIMARY KEY (project_id, src_path, spec)
);
CREATE INDEX IF NOT EXISTS code_imports_src ON code_imports(project_id, src_path);

CREATE TABLE IF NOT EXISTS code_edges (
    project_id TEXT NOT NULL,
    src_path   TEXT NOT NULL,
    dst_path   TEXT NOT NULL,
    kind       TEXT NOT NULL,
    PRIMARY KEY (project_id, src_path, dst_path, kind)
);
CREATE INDEX IF NOT EXISTS code_edges_dst ON code_edges(project_id, dst_path);
CREATE INDEX IF NOT EXISTS code_edges_src ON code_edges(project_id, src_path);
"#;
