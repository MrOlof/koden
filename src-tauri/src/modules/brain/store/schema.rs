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
/// v10: no DDL change — extraction/query semantics changed (ADR-010 cluster 7):
///     TS/TSX definitions are scope-anchored (function-locals / object-literal
///     methods no longer index as symbols) and query terms are deduped before
///     MATCH. `code_nodes` + the AST-fed `symbols` column are DERIVED, so the bump
///     forces the rebuild that re-derives them cleanly — and rotates the gist cache
///     key so a gist never mixes pre- and post-anchor ranking (the byte-identity
///     gate holds per key).
/// v11: no DDL change — gist LAYOUT changed (ADR-011 gist upgrades): a
///     known-unknowns section (empty retrieval legs stated explicitly) and
///     per-claim freshness labels on memory notes (current / possibly-stale /
///     historical(superseded)). The bump rotates every gist cache key (see the
///     v6 note) so one key never mixes pre- and post-layout bytes — one-time
///     post-upgrade agent-prompt-cache miss, expected.
/// v12: no DDL change — RANKING semantics changed (V3 multi-token coverage
///     re-rank: blend + relative gate in `search_with_weights`), which changes the
///     gist "Relevant files" selection/order. As with v10, the bump rotates every
///     gist cache key so one key never mixes pre- and post-coverage ranking bytes
///     (the byte-identity gate holds per key) — one-time post-upgrade
///     agent-prompt-cache miss, expected.
/// v13: perf pair (deferred from ADR-010 cluster 6). `code_imports.base` (the
///     normalized resolution base of a relative spec, '' for external/escaping)
///     + its (project_id, base) index — powers the delta edge relink
///     (`relink_edges_delta`: only imports whose base an appearing/disappearing
///     file can serve are re-resolved). Also `files_recency` (project_id,
///     accessed_at_ms) so the temporal-boost `ref_ms` is an index seek, not a
///     files-table scan. RANKING semantics are UNCHANGED (equivalence pinned by
///     `temporal_boost_bounded_probe_matches_full_scan`); the bump exists for the
///     DDL change — `code_imports` is DERIVED, so the upgrade drop/rebuild
///     backfills `base`. Key rotation side effect as per the v6 note.
/// v14: no DDL change — gist SELECTION semantics changed (gauntlet S2
///     `no-test-exclusion-in-gist-search`): conventional test paths
///     (`is_test_path`, the code_impact `exclude_tests` idiom) are now excluded
///     from the gist's "Relevant files" budget. As with v12, the bump rotates
///     every gist cache key so one key never mixes pre- and post-filter bytes
///     (the byte-identity gate holds per key) — one-time post-upgrade
///     agent-prompt-cache miss, expected.
/// v15: no DDL change — EXTRACTION semantics changed (gauntlet defect
///     `rust-imports-no-ast-dependents`): Rust `use` declarations are now
///     extracted into `code_imports` (expanded groups/aliases/wildcards) and
///     resolved to file edges (`rust_use_base`), so `ast_dependents` covers the
///     Rust surface. `code_imports`/`code_edges` are DERIVED, but the worker
///     hash-skips unchanged files and `rebuild_edges` only reads `code_imports`
///     — without the bump every pre-existing store would keep zero Rust import
///     rows forever (the original defect persisting). As with v10, the bump
///     forces the derived-table drop/rebuild that re-derives them, and rotates
///     the gist cache key — one-time post-upgrade agent-prompt-cache miss,
///     expected.
///
/// v15 + ADR-018 (NO bump): autonomous curation added COLUMNS TO CANONICAL tables
///     (`proposals` apply/undo state, `brain_librarian.curation_mode`). Canonical
///     tables are preserved across bumps (never dropped/rebuilt), so a version bump
///     cannot deliver new columns to an existing store — instead the idempotent
///     `migrate::ensure_additive_columns` issues a guarded `ALTER TABLE ... ADD
///     COLUMN` per missing column on every open (the additive-canonical idiom,
///     extending the `brain_librarian_pin` new-table precedent below). No gist key
///     rotation: gist bytes are unaffected by proposal/undo state.
pub const SCHEMA_VERSION: i64 = 15;

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
-- V2 temporal re-rank perf: MAX(accessed_at_ms) per project (the boost's ref_ms)
-- becomes an index seek instead of a project-wide scan on every search.
CREATE INDEX IF NOT EXISTS files_recency   ON files(project_id, accessed_at_ms);

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

-- Proposal queue + undo ledger (P1, ADR-018). Brain-owned, local-only. `signature`
-- is the plain-join proposalSignature (dedup PK). Status: pending|applied|rejected|
-- reverted. Under ADR-018 autonomous curation the worker APPLIES pending proposals
-- itself (in 'review' mode they wait for a human, the pre-ADR-018 behavior); either
-- way the apply snapshots its INVERSE into the undo_* columns BEFORE any file write,
-- so `brain_revert_proposal` can restore the prior state. The apply/undo columns are
-- ADDITIVE-canonical: fresh stores get them from this DDL, existing stores from the
-- guarded `ALTER TABLE ... ADD COLUMN` in `migrate::ensure_additive_columns` — a
-- SCHEMA_VERSION bump could NOT add them (bumps only rebuild DERIVED tables; this
-- table is canonical/preserved) and would rotate every gist cache key for nothing.
CREATE TABLE IF NOT EXISTS proposals (
    project_id TEXT NOT NULL,
    signature  TEXT NOT NULL,
    action     TEXT NOT NULL,   -- create|update|supersede|archive
    target_id  TEXT,
    title      TEXT NOT NULL,
    detail     TEXT NOT NULL,
    source     TEXT NOT NULL,   -- doctor|reflect|curate
    status     TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    -- ADR-018 apply/undo state (keep in lockstep with migrate::ADDITIVE_CANONICAL_COLUMNS):
    applied_ms       INTEGER,   -- when the apply landed (epoch ms)
    reverted_ms      INTEGER,   -- when a revert landed (epoch ms)
    auto_applied     INTEGER NOT NULL DEFAULT 0, -- 1 = applied by the autonomous worker
    undo_created_id  TEXT,      -- create/supersede: the minted note id (revert deletes it)
    undo_prior_path  TEXT,      -- archive/update/supersede: target note rel path
    undo_prior_bytes TEXT,      -- FULL prior file content (revert restores it verbatim)
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
    updated_at    INTEGER NOT NULL DEFAULT 0,
    -- ADR-018: 'autonomous' (worker applies proposals itself, snapshot-undo
    -- recorded) or 'review' (proposals wait in the inbox). Additive-canonical —
    -- existing stores get it via migrate::ensure_additive_columns, no version bump.
    curation_mode TEXT NOT NULL DEFAULT 'autonomous'
);
INSERT OR IGNORE INTO brain_librarian (id) VALUES (1);

-- Librarian delta-gate pin, PER PROJECT (LIB-SPEND-01). `digest_hash` is the hash
-- of the last digest a completed autonomous/manual round reflected on; the worker
-- short-circuits reflect to Unchanged ($0) when the freshly built digest hashes to
-- the pinned value (reflect/mod.rs `reflect_auto_with_client`). The live in-memory
-- pin (`worker::LibrarianAuto.digest_hash`) lives in a HashMap that is rebuilt EMPTY
-- on every worker boot, so without this durable copy the first post-restart round
-- for a project re-pays a byte-identical digest. CANONICAL/preserved (spend-integrity
-- state, not re-derivable) — ABSENT from the upgrade DROP batch, salvaged on a
-- corrupt-cache rebuild. No seed row: populated at runtime, per project, on the first
-- round. Additive via IF NOT EXISTS (like `brain_librarian`) so no SCHEMA_VERSION bump
-- is needed — the idempotent DDL creates it on the next open of any existing store.
CREATE TABLE IF NOT EXISTS brain_librarian_pin (
    project_id  TEXT PRIMARY KEY,
    digest_hash TEXT NOT NULL,
    updated_at  INTEGER NOT NULL DEFAULT 0
);

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
    -- v13: the spec's normalized resolution base (importer-dir + spec, `.`/`..`
    -- folded), '' for external/root-escaping specs. Stored so the delta relink
    -- can find, via the (project_id, base) index, exactly the imports whose
    -- resolution an appearing/disappearing file could change — the EXTS
    -- candidates of a base are `base` + fixed suffixes, so any file able to
    -- satisfy an import serves that import's base (see `serveable_bases`).
    base       TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, src_path, spec)
);
CREATE INDEX IF NOT EXISTS code_imports_src  ON code_imports(project_id, src_path);
CREATE INDEX IF NOT EXISTS code_imports_base ON code_imports(project_id, base);

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
