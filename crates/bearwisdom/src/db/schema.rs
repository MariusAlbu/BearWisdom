// =============================================================================
// db/schema.rs  —  DDL for all tables and indexes
//
// Key design choices vs. v1:
//   • `symbols.scope_path`    — the parent scope chain (dot-separated)
//   • `symbols.qualified_name` — full dotted path, indexed for fast lookup
//   • `imports` table         — explicit import records for resolution
//   • `routes` table          — HTTP route/handler mapping for ASP.NET
//   • `db_mappings` table     — EF Core DbSet<T> → table name mapping
//
// PRAGMA notes (important for rusqlite 0.33+):
//   In rusqlite 0.33, PRAGMA statements return result rows.  You MUST use
//   `query_row` (not `execute`) to consume them, otherwise you get an error.
//   See the `pragma` helper below.
// =============================================================================

use rusqlite::Connection;

/// Apply performance and correctness PRAGMAs.
/// `is_new`: true on first-ever open (needed for page_size).
pub fn apply_pragmas(conn: &Connection, is_new: bool) -> rusqlite::Result<()> {
    // Helper that tolerates "query returned no rows" (some PRAGMAs return
    // nothing on certain SQLite versions).
    fn pragma(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
        conn.query_row(sql, [], |_| Ok(()))
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(()),
                other => Err(other),
            })
    }

    // page_size must be set BEFORE the first page is written.
    if is_new {
        pragma(conn, "PRAGMA page_size = 8192")?;
    }

    // Write-Ahead Logging: concurrent readers + one writer, no lock contention.
    pragma(conn, "PRAGMA journal_mode = WAL")?;

    // NORMAL: fsync only at checkpoints (safe enough for an index that can be rebuilt).
    pragma(conn, "PRAGMA synchronous = NORMAL")?;

    // Wait up to 5 s before returning SQLITE_BUSY.
    pragma(conn, "PRAGMA busy_timeout = 5000")?;

    // 16 MB cache (negative value = kibibytes).
    pragma(conn, "PRAGMA cache_size = -16000")?;

    // 256 MB mmap window — OS maps the file into virtual address space so
    // reads bypass the syscall boundary on large databases.
    pragma(conn, "PRAGMA mmap_size = 268435456")?;

    // Keep all temp tables and indexes in memory rather than on disk.
    pragma(conn, "PRAGMA temp_store = MEMORY")?;

    // Enforce FK constraints — catches bugs where we insert an edge whose
    // source_id or target_id does not exist in symbols.
    pragma(conn, "PRAGMA foreign_keys = ON")?;

    Ok(())
}

/// Create all tables and indexes (idempotent — uses IF NOT EXISTS).
///
/// Also runs lightweight migrations for columns added after initial release.
pub fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    migrate(conn)?;
    Ok(())
}

/// Lightweight schema migrations for columns added to existing tables.
///
/// Each migration checks whether the column already exists (via PRAGMA
/// table_info) before running ALTER TABLE.  This is idempotent and safe
/// to run on every open.
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    // v0.3: Add mtime + size to files for fast change detection.
    if !column_exists(conn, "files", "mtime") {
        conn.execute_batch("ALTER TABLE files ADD COLUMN mtime INTEGER")?;
    }
    if !column_exists(conn, "files", "size") {
        conn.execute_batch("ALTER TABLE files ADD COLUMN size INTEGER")?;
    }
    // v0.3: Add incoming_edge_count to symbols for materialized centrality.
    if !column_exists(conn, "symbols", "incoming_edge_count") {
        conn.execute_batch(
            "ALTER TABLE symbols ADD COLUMN incoming_edge_count INTEGER NOT NULL DEFAULT 0"
        )?;
    }
    // v0.3: Add package_id to files for monorepo/workspace support.
    if !column_exists(conn, "files", "package_id") {
        conn.execute_batch(
            "ALTER TABLE files ADD COLUMN package_id INTEGER REFERENCES packages(id) ON DELETE SET NULL"
        )?;
    }
    // Always ensure the index exists — covers both new DBs (column from CREATE
    // TABLE) and migrated DBs (column from ALTER TABLE above).
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_files_package ON files(package_id)"
    )?;
    // v0.4: Add is_service flag to packages for Dockerfile-backed service detection.
    if !column_exists(conn, "packages", "is_service") {
        conn.execute_batch(
            "ALTER TABLE packages ADD COLUMN is_service INTEGER NOT NULL DEFAULT 0"
        )?;
    }
    // v0.3 monorepo Phase A: add declared_name — the package name as stated
    // in its own manifest (package.json `name`, Cargo.toml [package].name,
    // .csproj filename stem, etc.). Distinct from `name` which is the
    // folder-derived key used for sort-stability. Needed so the TS resolver
    // can map `import { x } from '@myorg/utils'` → package_id of the
    // workspace package whose package.json declares `"name": "@myorg/utils"`.
    if !column_exists(conn, "packages", "declared_name") {
        conn.execute_batch(
            "ALTER TABLE packages ADD COLUMN declared_name TEXT"
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_packages_declared_name
         ON packages(declared_name)
         WHERE declared_name IS NOT NULL"
    )?;
    // v0.5: Add origin to files and symbols to partition internal project code
    // from externally-indexed dependency code (module cache, package sources).
    // Values: 'internal' | 'external'. User-facing queries filter origin='internal'.
    if !column_exists(conn, "files", "origin") {
        conn.execute_batch(
            "ALTER TABLE files ADD COLUMN origin TEXT NOT NULL DEFAULT 'internal'"
        )?;
    }
    if !column_exists(conn, "symbols", "origin") {
        conn.execute_batch(
            "ALTER TABLE symbols ADD COLUMN origin TEXT NOT NULL DEFAULT 'internal'"
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_files_origin ON files(origin);
         CREATE INDEX IF NOT EXISTS idx_symbols_origin ON symbols(origin);"
    )?;
    // v0.6: Add origin_language to symbols for multi-language host files
    // (Vue/Svelte/Astro/Razor/HTML/PHP). NULL = same as the file's language.
    // Populated by the indexer when it splices sub-extracted symbols back into
    // a host file; lets queries filter "show me only the TS symbols in this .vue".
    if !column_exists(conn, "symbols", "origin_language") {
        conn.execute_batch(
            "ALTER TABLE symbols ADD COLUMN origin_language TEXT"
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_symbols_origin_language
           ON symbols(origin_language) WHERE origin_language IS NOT NULL"
    )?;
    // v0.7 (M1): Per-package attribution on external_refs + unresolved_refs.
    // Populated by the resolver (M2) from the source symbol's package_id so
    // queries like "which packages in this monorepo use axios?" are answerable.
    // NULL = ref came from a file with no package (root configs, shared scripts).
    if !column_exists(conn, "external_refs", "package_id") {
        conn.execute_batch(
            "ALTER TABLE external_refs ADD COLUMN package_id INTEGER REFERENCES packages(id) ON DELETE SET NULL"
        )?;
    }
    if !column_exists(conn, "unresolved_refs", "package_id") {
        conn.execute_batch(
            "ALTER TABLE unresolved_refs ADD COLUMN package_id INTEGER REFERENCES packages(id) ON DELETE SET NULL"
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_external_refs_package   ON external_refs(package_id)   WHERE package_id IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_unresolved_refs_package ON unresolved_refs(package_id) WHERE package_id IS NOT NULL;"
    )?;
    // v0.8 (E3): Snippet-origin flag on unresolved_refs. Set to 1 for refs
    // that originate from symbols spliced in from a Markdown fenced code
    // block, Rust doctest, or Python docstring `>>>` region. Aggregate
    // resolution stats exclude these rows — snippets typically lack imports.
    if !column_exists(conn, "unresolved_refs", "from_snippet") {
        conn.execute_batch(
            "ALTER TABLE unresolved_refs ADD COLUMN from_snippet INTEGER NOT NULL DEFAULT 0"
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_unresolved_refs_snippet
           ON unresolved_refs(from_snippet) WHERE from_snippet = 1"
    )?;
    // v0.9 (T9): Record which resolver strategy produced each edge.
    // Populated by the resolution pipeline — the engine writes the language
    // resolver's named strategy (e.g. "ts_workspace_pkg", "csharp_using_directive"),
    // the heuristic writes a "heuristic_*" family. NULL for legacy rows and
    // direct DB inserts (SCIP import, tests). Lets diagnostic queries answer
    // "why is this edge 0.95?" without re-running the resolver.
    if !column_exists(conn, "edges", "strategy") {
        conn.execute_batch("ALTER TABLE edges ADD COLUMN strategy TEXT")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_edges_strategy
           ON edges(strategy) WHERE strategy IS NOT NULL"
    )?;
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(r) => r,
        Err(_) => return false,
    };
    for row in rows {
        if let Ok(name) = row {
            if name == column {
                return true;
            }
        }
    }
    false
}

const SCHEMA_SQL: &str = "
-- ============================================================
-- WORKSPACE PACKAGES
-- ============================================================
-- One row per detected package in a monorepo / workspace.
-- Single-project repos leave this table empty.

CREATE TABLE IF NOT EXISTS packages (
    id            INTEGER PRIMARY KEY,
    name          TEXT    NOT NULL UNIQUE,  -- folder-derived key, stable sort
    path          TEXT    NOT NULL UNIQUE,  -- relative to workspace root
    kind          TEXT,                     -- ecosystem hint: npm, cargo, dotnet, go, etc.
    manifest      TEXT,                     -- relative path to manifest file
    parent_id     INTEGER REFERENCES packages(id) ON DELETE SET NULL,
    is_service    INTEGER NOT NULL DEFAULT 0,  -- 1 if a Dockerfile was found in this package
    declared_name TEXT                         -- manifest-declared name (@myorg/foo, etc.)
);

CREATE INDEX IF NOT EXISTS idx_packages_path ON packages(path);
-- idx_packages_declared_name lives in migrate() — old DBs receive the
-- column via ALTER TABLE there, and the index needs to be created AFTER
-- the column exists.

-- ============================================================
-- PACKAGE DEPENDENCIES  (M3)
-- ============================================================
-- Normalized dependency graph per workspace package. Populated by
-- parse_external_sources during a full index from each declaring
-- package manifest. Enables cross-package queries like which
-- packages in this monorepo declare axios without re-reading
-- manifests at query time.
--
-- ecosystem is the locator ecosystem id (typescript, python,
-- dotnet, etc.). dep_name is the manifest-declared package name
-- (react, @tanstack/react-query, Microsoft.Extensions.Logging).
-- kind is one of runtime, dev, peer, build.
-- version is the specifier string from the manifest, or NULL if
-- the manifest did not declare a version.

CREATE TABLE IF NOT EXISTS package_deps (
    package_id INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    ecosystem  TEXT    NOT NULL,
    dep_name   TEXT    NOT NULL,
    version    TEXT,
    kind       TEXT    NOT NULL,
    PRIMARY KEY (package_id, ecosystem, dep_name)
);

CREATE INDEX IF NOT EXISTS idx_package_deps_ecosystem_name
    ON package_deps(ecosystem, dep_name);

-- ============================================================
-- FILE TRACKING
-- ============================================================

-- One row per indexed file.
-- `hash` is the SHA-256 of the file contents.  A changed hash
-- means the file must be re-indexed; an unchanged hash is skipped.
CREATE TABLE IF NOT EXISTS files (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL UNIQUE,
    hash         TEXT    NOT NULL,
    language     TEXT    NOT NULL,
    last_indexed INTEGER NOT NULL,  -- Unix timestamp (seconds since epoch)
    mtime        INTEGER,           -- file mtime (seconds since epoch), for fast change detection
    size         INTEGER,           -- file size in bytes, for fast change detection
    package_id   INTEGER REFERENCES packages(id) ON DELETE SET NULL,
    origin       TEXT    NOT NULL DEFAULT 'internal'  -- 'internal' | 'external' (e.g., $GOPATH/pkg/mod)
);

CREATE INDEX IF NOT EXISTS idx_files_language  ON files(language);
-- Covers both hash-only lookups (incremental change detection) and
-- path+hash scans — replaces the old idx_files_hash single-column index.
CREATE INDEX IF NOT EXISTS idx_files_path_hash ON files(path, hash);
-- idx_files_package + idx_files_origin are created by migrate() to handle
-- existing DBs where the columns are added via ALTER TABLE.

-- ============================================================
-- CODE GRAPH: SYMBOLS
-- ============================================================

-- One row per named symbol (class, method, property, …).
-- `qualified_name` is the full dotted path:
--   e.g. 'Microsoft.eShop.Catalog.CatalogDbContext.OnModelCreating'
-- `scope_path` is the parent chain:
--   e.g. 'Microsoft.eShop.Catalog.CatalogDbContext'
CREATE TABLE IF NOT EXISTS symbols (
    id             INTEGER PRIMARY KEY,
    file_id        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    qualified_name TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    line           INTEGER NOT NULL,
    col            INTEGER NOT NULL,
    end_line       INTEGER,
    end_col        INTEGER,
    scope_path     TEXT,
    signature      TEXT,
    doc_comment    TEXT,   -- XML doc comment (C#) or JSDoc (TS), used by FTS5
    visibility     TEXT,
    incoming_edge_count INTEGER NOT NULL DEFAULT 0,  -- materialized centrality signal
    origin         TEXT    NOT NULL DEFAULT 'internal',  -- 'internal' | 'external'; mirrors files.origin for fast filtering
    origin_language TEXT   -- sub-language id when different from files.language (NULL = same); set for symbols spliced in from embedded regions (e.g. a script block inside a Vue SFC)
);

CREATE INDEX IF NOT EXISTS idx_symbols_name      ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_qualified ON symbols(qualified_name);
-- Covering index for name-based lookups: returns file, kind, and position
-- without a table lookup.
CREATE INDEX IF NOT EXISTS idx_symbols_name_cov
    ON symbols(name, file_id, kind, line, col);
-- Covering index for file-based lookups (most common in incremental indexing):
-- returns all display columns without touching the symbols heap.
-- Replaces the old idx_symbols_file single-column index.
CREATE INDEX IF NOT EXISTS idx_symbols_file_cov
    ON symbols(file_id, name, qualified_name, kind, line, col);

-- ============================================================
-- CODE GRAPH: EDGES
-- ============================================================

-- A directed edge from source_id → target_id with a relationship kind.
-- `confidence` is 0–1; values < 1.0 come from heuristic resolution.
-- The UNIQUE constraint prevents duplicate edges from being inserted twice
-- during resolution passes.
CREATE TABLE IF NOT EXISTS edges (
    source_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    target_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL,
    source_line INTEGER,
    confidence  REAL    NOT NULL DEFAULT 1.0,
    -- T9: which resolver strategy produced this edge. NULL for legacy / direct inserts.
    strategy    TEXT,
    UNIQUE(source_id, target_id, kind, source_line)
);

-- Covering indexes for call hierarchy queries: include all columns needed
-- to resolve callers/callees without touching the edges heap.
-- These supersede the old idx_edges_source and idx_edges_target indexes.
CREATE INDEX IF NOT EXISTS idx_edges_source_cov
    ON edges(source_id, kind, target_id, confidence, source_line);
CREATE INDEX IF NOT EXISTS idx_edges_target_cov
    ON edges(target_id, kind, source_id, confidence);

-- ============================================================
-- UNRESOLVED REFERENCES
-- ============================================================

-- References that could not be resolved to a symbol ID at index time.
-- Kept for diagnostics and for re-resolution when more files are indexed.
CREATE TABLE IF NOT EXISTS unresolved_refs (
    id          INTEGER PRIMARY KEY,
    source_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    target_name TEXT    NOT NULL,
    kind        TEXT    NOT NULL,
    source_line INTEGER,
    module      TEXT,
    package_id  INTEGER REFERENCES packages(id) ON DELETE SET NULL,  -- M1/M2: per-package attribution
    from_snippet INTEGER NOT NULL DEFAULT 0                          -- E3: 1 if source symbol is from a Markdown fence / doctest
);

CREATE INDEX IF NOT EXISTS idx_unresolved_name       ON unresolved_refs(target_name);
-- Covering index for diagnostics queries: returns all display columns for a
-- given source symbol without a table lookup.
-- Supersedes the old idx_unresolved_source_kind index.
CREATE INDEX IF NOT EXISTS idx_unresolved_source_cov
    ON unresolved_refs(source_id, target_name, kind, source_line);
-- idx_unresolved_refs_package is created by migrate() to cover both new DBs
-- (column from CREATE TABLE above) and migrated DBs (column from ALTER).

-- ============================================================
-- EXTERNAL REFERENCES
-- ============================================================

-- References identified as belonging to external frameworks/libraries
-- (e.g., System.*, Microsoft.*, Newtonsoft.*). Separated from
-- unresolved_refs so diagnostics and enrichment can focus on
-- genuinely unknown project references.
CREATE TABLE IF NOT EXISTS external_refs (
    id          INTEGER PRIMARY KEY,
    source_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    target_name TEXT    NOT NULL,
    kind        TEXT    NOT NULL,
    source_line INTEGER,
    namespace   TEXT    NOT NULL,   -- inferred external namespace
    package_id  INTEGER REFERENCES packages(id) ON DELETE SET NULL  -- M1/M2: per-package attribution
);

-- Covering index for namespace analysis queries: returns target, kind, and
-- namespace without a table lookup.
-- Supersedes the old idx_external_source single-column index.
CREATE INDEX IF NOT EXISTS idx_external_source_cov
    ON external_refs(source_id, target_name, kind, namespace);
CREATE INDEX IF NOT EXISTS idx_external_ns ON external_refs(namespace);
-- idx_external_refs_package is created by migrate() to cover both new DBs
-- (column from CREATE TABLE above) and migrated DBs (column from ALTER).

-- ============================================================
-- IMPORTS
-- ============================================================

-- One row per `using` directive (C#) or `import` statement (TS).
-- Used by the 4-priority resolver to boost confidence when
-- a reference name matches an imported symbol from a known file.
CREATE TABLE IF NOT EXISTS imports (
    id             INTEGER PRIMARY KEY,
    file_id        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    imported_name  TEXT    NOT NULL,
    module_path    TEXT,   -- 'System.Linq' | './catalog-api'
    alias          TEXT,   -- 'using Db = Microsoft.EntityFrameworkCore'
    line           INTEGER
);

CREATE INDEX IF NOT EXISTS idx_imports_file ON imports(file_id);
CREATE INDEX IF NOT EXISTS idx_imports_name ON imports(imported_name);

-- ============================================================
-- HTTP ROUTES  (ASP.NET connector)
-- ============================================================

-- One row per route endpoint, extracted from [HttpGet/Post/Put/Delete/Patch],
-- [Route], or minimal-API app.MapGet/MapPost calls.
CREATE TABLE IF NOT EXISTS routes (
    id             INTEGER PRIMARY KEY,
    file_id        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    symbol_id      INTEGER REFERENCES symbols(id),
    http_method    TEXT    NOT NULL,   -- 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH'
    route_template TEXT    NOT NULL,   -- '/api/catalog/items/{id}'
    resolved_route TEXT,               -- fully qualified including controller prefix
    line           INTEGER
);

CREATE INDEX IF NOT EXISTS idx_routes_template ON routes(route_template);
CREATE INDEX IF NOT EXISTS idx_routes_method   ON routes(http_method, route_template);

-- ============================================================
-- EF CORE DB MAPPINGS  (EF Core connector)
-- ============================================================

-- Maps a C# entity class to its database table name.
-- `source`: how the table name was determined:
--   'convention' — plural of the entity class name (EF default)
--   'attribute'  — Table attribute override
--   'fluent'     — entity.ToTable call in OnModelCreating
CREATE TABLE IF NOT EXISTS db_mappings (
    id          INTEGER PRIMARY KEY,
    symbol_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    table_name  TEXT    NOT NULL,
    entity_type TEXT    NOT NULL,
    source      TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_db_mappings_table  ON db_mappings(table_name);
CREATE INDEX IF NOT EXISTS idx_db_mappings_entity ON db_mappings(entity_type);

-- ============================================================
-- FULL-TEXT SEARCH: FTS5 on symbols
-- ============================================================
-- FTS5 is a content table backed by `symbols`.
-- `content='symbols'` tells SQLite to read from that table when
-- displaying results; `content_rowid='id'` links the FTS row IDs
-- to symbols.id.
-- Because it is a content table (not a copy of the data), we need
-- triggers to keep the FTS index up-to-date whenever symbols change.

CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name, qualified_name, signature, doc_comment,
    content='symbols',
    content_rowid='id'
);

-- Triggered after INSERT into symbols: add a new FTS row.
CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
    INSERT INTO symbols_fts(rowid, name, qualified_name, signature, doc_comment)
    VALUES (new.id, new.name, new.qualified_name, new.signature, new.doc_comment);
END;

-- Triggered after DELETE from symbols: remove the FTS row.
CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, qualified_name, signature, doc_comment)
    VALUES ('delete', old.id, old.name, old.qualified_name, old.signature, old.doc_comment);
END;

-- Triggered after UPDATE on symbols: delete the old FTS row, insert new.
CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, qualified_name, signature, doc_comment)
    VALUES ('delete', old.id, old.name, old.qualified_name, old.signature, old.doc_comment);
    INSERT INTO symbols_fts(rowid, name, qualified_name, signature, doc_comment)
    VALUES (new.id, new.name, new.qualified_name, new.signature, new.doc_comment);
END;

-- ============================================================
-- KNOWLEDGE TREE: ANNOTATIONS
-- ============================================================
-- Free-form markdown notes attached to a symbol.
-- `concept` is an optional label that can group annotations
-- without requiring full concept membership (lightweight tagging).

CREATE TABLE IF NOT EXISTS annotations (
    id         INTEGER PRIMARY KEY,
    symbol_id  INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
    concept    TEXT,           -- optional label, e.g. 'authentication'
    content    TEXT    NOT NULL,
    author     TEXT    NOT NULL DEFAULT 'user',
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_annotations_symbol  ON annotations(symbol_id);
CREATE INDEX IF NOT EXISTS idx_annotations_concept ON annotations(concept);

-- ============================================================
-- KNOWLEDGE TREE: CONCEPTS
-- ============================================================
-- A concept is a named domain grouping (e.g. 'catalog-management').
-- Symbols can be assigned to concepts manually or automatically
-- via the `auto_pattern` glob that matches qualified_name prefixes.

CREATE TABLE IF NOT EXISTS concepts (
    id           INTEGER PRIMARY KEY,
    name         TEXT    NOT NULL UNIQUE,
    description  TEXT,
    auto_pattern TEXT,          -- e.g. 'eShop.Catalog.*' or 'src/Catalog/**'
    parent_id    INTEGER REFERENCES concepts(id) ON DELETE SET NULL,
    created_at   INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Members of a concept — many-to-many between concepts and symbols.
CREATE TABLE IF NOT EXISTS concept_members (
    concept_id    INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    symbol_id     INTEGER NOT NULL REFERENCES symbols(id)  ON DELETE CASCADE,
    auto_assigned INTEGER NOT NULL DEFAULT 0,  -- 1 = matched by auto_pattern
    UNIQUE(concept_id, symbol_id)
);

CREATE INDEX IF NOT EXISTS idx_concept_members_concept ON concept_members(concept_id);
CREATE INDEX IF NOT EXISTS idx_concept_members_symbol  ON concept_members(symbol_id);

-- ============================================================
-- LSP EDGE PROVENANCE
-- ============================================================
-- Tracks which edges were produced or confirmed by an LSP server.
-- The edge_rowid references the implicit rowid of the edges table.
-- When a file changes, all lsp_edge_meta rows for edges belonging
-- to symbols in that file are deleted, resetting those edges to
-- tree-sitter confidence.

CREATE TABLE IF NOT EXISTS lsp_edge_meta (
    edge_rowid  INTEGER NOT NULL,
    source      TEXT    NOT NULL DEFAULT 'lsp',
    server      TEXT,
    resolved_at INTEGER NOT NULL,
    UNIQUE(edge_rowid)
);

CREATE INDEX IF NOT EXISTS idx_lsp_meta_edge ON lsp_edge_meta(edge_rowid);

-- ============================================================
-- FULL-TEXT SEARCH: FTS5 on file content (trigram)
-- ============================================================
-- Contentless FTS5 table indexed with trigrams for instant
-- substring search across all file content.
-- `content = ''` means no original text stored (saves space).
-- `contentless_delete = 1` allows row deletion without the
-- original text (requires SQLite 3.43+).

CREATE VIRTUAL TABLE IF NOT EXISTS fts_content USING fts5(
    path,
    content,
    tokenize = 'trigram case_sensitive 0',
    content = '',
    contentless_delete = 1
);

-- ============================================================
-- CODE CHUNKS  (for embeddings)
-- ============================================================
-- AST-aware chunks of source code, aligned to symbol boundaries.
-- Each chunk is at most 512 tokens (CodeRankEmbed context window).
-- Chunks are the unit of embedding and vector search.

CREATE TABLE IF NOT EXISTS code_chunks (
    id           INTEGER PRIMARY KEY,
    file_id      INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    symbol_id    INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
    content_hash TEXT    NOT NULL,
    content      TEXT    NOT NULL,
    start_line   INTEGER NOT NULL,
    end_line     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_chunk_file ON code_chunks(file_id);
CREATE INDEX IF NOT EXISTS idx_chunk_hash ON code_chunks(content_hash);

-- ============================================================
-- CROSS-LANGUAGE FLOW EDGES
-- ============================================================
-- Directed edges between symbols across language boundaries.
-- Examples: TS fetch() -> C# controller, C# service -> SQL table,
-- gRPC client -> server, React component -> API endpoint.

CREATE TABLE IF NOT EXISTS flow_edges (
    id               INTEGER PRIMARY KEY,
    source_file_id   INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    source_line      INTEGER,
    source_symbol    TEXT,
    source_language  TEXT,
    target_file_id   INTEGER REFERENCES files(id) ON DELETE SET NULL,
    target_line      INTEGER,
    target_symbol    TEXT,
    target_language  TEXT,
    edge_type        TEXT    NOT NULL,
    protocol         TEXT,
    http_method      TEXT,
    url_pattern      TEXT,
    confidence       REAL    NOT NULL DEFAULT 0.5,
    metadata         TEXT
);

CREATE INDEX IF NOT EXISTS idx_flow_source ON flow_edges(source_file_id);
CREATE INDEX IF NOT EXISTS idx_flow_target ON flow_edges(target_file_id);
-- Covers edge_type-only filters, (edge_type, source_language) filters, and
-- cross-language pair queries — supersedes both idx_flow_type and
-- idx_flow_type_lang.
CREATE INDEX IF NOT EXISTS idx_flow_edges_type
    ON flow_edges(edge_type, source_language, target_language);
CREATE INDEX IF NOT EXISTS idx_flow_url ON flow_edges(url_pattern);

-- ============================================================
-- CONNECTION POINTS  (connector architecture)
-- ============================================================
-- One row per extracted call site or handler, produced by Connector
-- implementations. The resolution engine matches starts to stops
-- within the same protocol to produce flow_edges.

CREATE TABLE IF NOT EXISTS connection_points (
    id        INTEGER PRIMARY KEY,
    file_id   INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    symbol_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
    line      INTEGER NOT NULL,
    protocol  TEXT    NOT NULL,
    direction TEXT    NOT NULL,
    key       TEXT    NOT NULL,
    method    TEXT    NOT NULL DEFAULT '',
    framework TEXT    NOT NULL DEFAULT '',
    metadata  TEXT,
    UNIQUE(file_id, line, protocol, direction, key, method)
);

CREATE INDEX IF NOT EXISTS idx_cp_protocol_dir ON connection_points(protocol, direction);
CREATE INDEX IF NOT EXISTS idx_cp_key          ON connection_points(key);
CREATE INDEX IF NOT EXISTS idx_cp_file         ON connection_points(file_id);

-- ============================================================
-- SEARCH HISTORY
-- ============================================================
-- Tracks recent and saved searches for quick recall.

CREATE TABLE IF NOT EXISTS search_history (
    id           INTEGER PRIMARY KEY,
    query        TEXT    NOT NULL,
    query_type   TEXT    NOT NULL,
    scope        TEXT,
    is_saved     INTEGER NOT NULL DEFAULT 0,
    last_used_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    use_count    INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_history_type  ON search_history(query_type);
CREATE INDEX IF NOT EXISTS idx_history_saved ON search_history(is_saved);

-- ============================================================
-- MCP AUDIT LOG
-- ============================================================
-- Tracks every MCP tool call: session, tool name, params, response,
-- duration, and a token estimate (response bytes / 4).
-- session_id is a UUID generated once per MCP stdio connection.

CREATE TABLE IF NOT EXISTS mcp_audit (
    id             INTEGER PRIMARY KEY,
    session_id     TEXT    NOT NULL,
    tool_name      TEXT    NOT NULL,
    params_json    TEXT    NOT NULL,
    response_json  TEXT    NOT NULL,
    duration_ms    INTEGER NOT NULL,
    token_estimate INTEGER NOT NULL DEFAULT 0,
    ts             TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- Composite index covers per-session queries ordered by call sequence.
CREATE INDEX IF NOT EXISTS idx_audit_session ON mcp_audit(session_id, id);
-- Covers timestamp-based range scans for the SSE tail query.
CREATE INDEX IF NOT EXISTS idx_audit_ts ON mcp_audit(ts);

-- ============================================================
-- INDEX METADATA  (key-value)
-- ============================================================
-- Stores per-index metadata: indexed_commit (git HEAD at last
-- successful index), schema_version, etc.  Queried at reindex
-- time to select the optimal change detection strategy.

CREATE TABLE IF NOT EXISTS _bearwisdom_meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);
";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
