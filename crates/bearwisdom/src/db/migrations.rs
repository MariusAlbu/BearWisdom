// =============================================================================
// db/migrations.rs — lightweight schema migrations for existing databases
//
// Each step checks whether its column/index already exists before altering,
// so the whole pass is idempotent and safe to run on every open. New DBs get
// the final shape from SCHEMA_SQL directly; these steps bring older files to
// the same shape.
// =============================================================================

use rusqlite::Connection;

/// Lightweight schema migrations for columns added to existing tables.
///
/// Each migration checks whether the column already exists (via PRAGMA
/// table_info) before running ALTER TABLE.  This is idempotent and safe
/// to run on every open.
pub(super) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
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
            "ALTER TABLE symbols ADD COLUMN incoming_edge_count INTEGER NOT NULL DEFAULT 0",
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
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_files_package ON files(package_id)")?;
    // v0.4: Add is_service flag to packages for Dockerfile-backed service detection.
    if !column_exists(conn, "packages", "is_service") {
        conn.execute_batch(
            "ALTER TABLE packages ADD COLUMN is_service INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    // Faithful TypeId persistence: canonical id columns alongside the legacy
    // string forms. Raw arena indices, valid against the restored arena snapshot.
    if !column_exists(conn, "symbol_type_info", "field_type_id") {
        conn.execute_batch("ALTER TABLE symbol_type_info ADD COLUMN field_type_id INTEGER")?;
    }
    if !column_exists(conn, "symbol_type_info", "return_type_id") {
        conn.execute_batch("ALTER TABLE symbol_type_info ADD COLUMN return_type_id INTEGER")?;
    }
    // v0.3 monorepo Phase A: add declared_name — the package name as stated
    // in its own manifest (package.json `name`, Cargo.toml [package].name,
    // .csproj filename stem, etc.). Distinct from `name` which is the
    // folder-derived key used for sort-stability. Needed so the TS resolver
    // can map `import { x } from '@myorg/utils'` → package_id of the
    // workspace package whose package.json declares `"name": "@myorg/utils"`.
    if !column_exists(conn, "packages", "declared_name") {
        conn.execute_batch("ALTER TABLE packages ADD COLUMN declared_name TEXT")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_packages_declared_name
         ON packages(declared_name)
         WHERE declared_name IS NOT NULL",
    )?;
    // v0.5: Add origin to files and symbols to partition internal project code
    // from externally-indexed dependency code (module cache, package sources).
    // Values: 'internal' | 'external'. User-facing queries filter origin='internal'.
    if !column_exists(conn, "files", "origin") {
        conn.execute_batch("ALTER TABLE files ADD COLUMN origin TEXT NOT NULL DEFAULT 'internal'")?;
    }
    if !column_exists(conn, "symbols", "origin") {
        conn.execute_batch(
            "ALTER TABLE symbols ADD COLUMN origin TEXT NOT NULL DEFAULT 'internal'",
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_files_origin ON files(origin);
         CREATE INDEX IF NOT EXISTS idx_symbols_origin ON symbols(origin);",
    )?;
    // v0.6: Add origin_language to symbols for multi-language host files
    // (Vue/Svelte/Astro/Razor/HTML/PHP). NULL = same as the file's language.
    // Populated by the indexer when it splices sub-extracted symbols back into
    // a host file; lets queries filter "show me only the TS symbols in this .vue".
    if !column_exists(conn, "symbols", "origin_language") {
        conn.execute_batch("ALTER TABLE symbols ADD COLUMN origin_language TEXT")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_symbols_origin_language
           ON symbols(origin_language) WHERE origin_language IS NOT NULL",
    )?;
    // external_refs is retired: nothing writes it since the legacy resolver's
    // deletion — external supply resolves to real edges instead. Shed the
    // table (and its indexes) from existing DBs.
    conn.execute_batch("DROP TABLE IF EXISTS external_refs;")?;
    // v0.7 (M1): Per-package attribution on unresolved_refs. Populated by the
    // resolver from the source symbol's package_id so queries like "which
    // packages in this monorepo use axios?" are answerable. NULL = ref came
    // from a file with no package (root configs, shared scripts).
    if !column_exists(conn, "unresolved_refs", "package_id") {
        conn.execute_batch(
            "ALTER TABLE unresolved_refs ADD COLUMN package_id INTEGER REFERENCES packages(id) ON DELETE SET NULL"
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_unresolved_refs_package ON unresolved_refs(package_id) WHERE package_id IS NOT NULL;"
    )?;
    // v0.8 (E3): Snippet-origin flag on unresolved_refs. Set to 1 for refs
    // that originate from symbols spliced in from a Markdown fenced code
    // block, Rust doctest, or Python docstring `>>>` region. Aggregate
    // resolution stats exclude these rows — snippets typically lack imports.
    if !column_exists(conn, "unresolved_refs", "from_snippet") {
        conn.execute_batch(
            "ALTER TABLE unresolved_refs ADD COLUMN from_snippet INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_unresolved_refs_snippet
           ON unresolved_refs(from_snippet) WHERE from_snippet = 1",
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
           ON edges(strategy) WHERE strategy IS NOT NULL",
    )?;
    // v0.10 + v0.12 packages-table rebuild. Both gated on
    // `PRAGMA user_version < 12` so they can't re-run on already-migrated
    // DBs. The earlier `LIKE '%name%TEXT%NOT NULL%UNIQUE%'` check
    // spuriously matches the v0.12 schema (because of the table-level
    // `UNIQUE(path, kind)` clause containing the keyword UNIQUE), so the
    // pattern alone is unreliable as a stop condition. user_version is
    // the canonical SQLite-native version counter.
    //
    // Migration body is the v0.12 form (composite UNIQUE, kind NOT NULL
    // DEFAULT 'unknown'). Very old DBs that still carry the v0.10
    // `name UNIQUE` constraint pass through this rebuild and end up with
    // the v0.12 shape directly — v0.10's intermediate `path UNIQUE` form
    // is short-lived and not worth a separate hop.
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    if user_version < 12 {
        conn.execute_batch(
            "BEGIN;
             UPDATE packages SET kind = 'unknown' WHERE kind IS NULL;
             CREATE TABLE packages_v12 (
                 id            INTEGER PRIMARY KEY,
                 name          TEXT    NOT NULL,
                 path          TEXT    NOT NULL,
                 kind          TEXT    NOT NULL DEFAULT 'unknown',
                 manifest      TEXT,
                 parent_id     INTEGER REFERENCES packages_v12(id) ON DELETE SET NULL,
                 is_service    INTEGER NOT NULL DEFAULT 0,
                 declared_name TEXT,
                 UNIQUE(path, kind)
             );
             INSERT INTO packages_v12 (id, name, path, kind, manifest, parent_id, is_service, declared_name)
                 SELECT id, name, path, COALESCE(kind, 'unknown'), manifest, parent_id, is_service, declared_name FROM packages;
             DROP TABLE packages;
             ALTER TABLE packages_v12 RENAME TO packages;
             CREATE INDEX IF NOT EXISTS idx_packages_path ON packages(path);
             CREATE INDEX IF NOT EXISTS idx_packages_declared_name
                 ON packages(declared_name)
                 WHERE declared_name IS NOT NULL;
             PRAGMA user_version = 12;
             COMMIT;",
        )?;
    }
    // v0.13: Add is_publishable to packages. Drives the ExportedApi
    // entry-point contributor — a non-publishable package's public symbols
    // are not auto-rooted as library-API entry points. Default 1 preserves
    // prior behavior; manifest contributors flip to 0 for Cargo
    // `publish = false`, npm `"private": true`, Maven absent
    // `<distributionManagement>`, etc.
    if !column_exists(conn, "packages", "is_publishable") {
        conn.execute_batch(
            "ALTER TABLE packages ADD COLUMN is_publishable INTEGER NOT NULL DEFAULT 1",
        )?;
    }
    // v0.11: Indexes on FK columns referencing symbols(id) so cascade
    // DELETE doesn't trigger full table scans. On a 280k-symbol index
    // (aspnetcore) deleting 7 symbol rows took 80s without these
    // indexes — `code_chunks` and `db_mappings` each scanned their
    // entire table per cascading delete because their `symbol_id`
    // column had no covering index. Adding these makes incremental
    // save-latency O(rows_per_symbol) instead of
    // O(rows_in_table * symbols_deleted).
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_code_chunks_symbol
            ON code_chunks(symbol_id) WHERE symbol_id IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_db_mappings_symbol
            ON db_mappings(symbol_id);",
    )?;

    // Phase H (post-connector-kill): dedup + UNIQUE indexes on flow_edges
    // and routes. Pre-Phase H pipelines had multiple parallel write paths
    // (registry matcher + resolver pairer + routes-table bridge) that wrote
    // the same logical row through different channels; neither table had a
    // unique constraint so the rows compounded. The DELETE keeps the lowest
    // id per logical group, then CREATE UNIQUE INDEX locks future writes.
    // Both statements are idempotent — once the indexes exist the DELETE
    // becomes a no-op and the CREATE is a no-op.
    if !index_exists(conn, "idx_flow_edges_unique") {
        conn.execute_batch(
            "DELETE FROM flow_edges WHERE id NOT IN (
                SELECT MIN(id) FROM flow_edges
                GROUP BY
                    source_file_id,
                    COALESCE(source_line, -1),
                    COALESCE(source_symbol, ''),
                    COALESCE(target_file_id, -1),
                    COALESCE(target_line, -1),
                    COALESCE(target_symbol, ''),
                    edge_type,
                    COALESCE(url_pattern, '')
            );
            CREATE UNIQUE INDEX idx_flow_edges_unique
                ON flow_edges(
                    source_file_id,
                    COALESCE(source_line, -1),
                    COALESCE(source_symbol, ''),
                    COALESCE(target_file_id, -1),
                    COALESCE(target_line, -1),
                    COALESCE(target_symbol, ''),
                    edge_type,
                    COALESCE(url_pattern, '')
                );",
        )?;
    }
    if !index_exists(conn, "idx_routes_unique") {
        conn.execute_batch(
            "DELETE FROM routes WHERE id NOT IN (
                SELECT MIN(id) FROM routes
                GROUP BY
                    file_id,
                    http_method,
                    route_template,
                    COALESCE(line, -1)
            );
            CREATE UNIQUE INDEX idx_routes_unique
                ON routes(file_id, http_method, route_template, COALESCE(line, -1));",
        )?;
    }
    // Drain flag on unresolved_refs: 1 when a rule declined the ref before the
    // strategy ladder ran because its target names a language builtin or other
    // non-project construct (`LanguageProfile::builtin_skip`), not a missing
    // project symbol. Resolution-rate aggregates (`CODE_REF_FILTER`) exclude
    // these rows; the row itself is kept so the drain stays diagnosable.
    if !column_exists(conn, "unresolved_refs", "drained") {
        conn.execute_batch(
            "ALTER TABLE unresolved_refs ADD COLUMN drained INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_unresolved_refs_drained
           ON unresolved_refs(drained) WHERE drained = 1",
    )?;
    // First-uncaptured-type cause columns, added empty here; the resolve
    // pipeline populates them only on the failure path (see pipeline.rs).
    if !column_exists(conn, "unresolved_refs", "cause_symbol_id") {
        conn.execute_batch(
            "ALTER TABLE unresolved_refs ADD COLUMN cause_symbol_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL",
        )?;
    }
    if !column_exists(conn, "unresolved_refs", "cause_kind") {
        conn.execute_batch("ALTER TABLE unresolved_refs ADD COLUMN cause_kind TEXT")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_unresolved_refs_cause
           ON unresolved_refs(cause_kind, cause_symbol_id) WHERE cause_kind IS NOT NULL",
    )?;
    // Extractors may emit the same logical ref more than once (double-visited
    // constructs, per-node-kind coverage double-emits). `edges` collapses
    // duplicates through its UNIQUE constraint; without the mirror constraint
    // here the failure side multiplies and resolution-rate aggregates
    // undercount. DELETE keeps the lowest id per logical ref, then the UNIQUE
    // index locks future writes (the pipeline inserts with OR IGNORE).
    if !index_exists(conn, "idx_unresolved_refs_unique") {
        conn.execute_batch(
            "DELETE FROM unresolved_refs WHERE id NOT IN (
                SELECT MIN(id) FROM unresolved_refs
                GROUP BY
                    source_id,
                    target_name,
                    kind,
                    COALESCE(source_line, -1),
                    COALESCE(module, '')
            );
            CREATE UNIQUE INDEX idx_unresolved_refs_unique
                ON unresolved_refs(
                    source_id,
                    target_name,
                    kind,
                    COALESCE(source_line, -1),
                    COALESCE(module, '')
                );",
        )?;
    }
    // Symbol-identity refactor (SYMBOL-IDENTITY.md): stable key, multi-location
    // containment. Columns are added empty here; the indexer populates them.
    if !column_exists(conn, "symbols", "symbol_key") {
        conn.execute_batch("ALTER TABLE symbols ADD COLUMN symbol_key TEXT")?;
    }
    if !column_exists(conn, "symbols", "mergeable") {
        conn.execute_batch("ALTER TABLE symbols ADD COLUMN mergeable INTEGER NOT NULL DEFAULT 0")?;
    }
    if !column_exists(conn, "symbols", "containing_id") {
        conn.execute_batch(
            "ALTER TABLE symbols ADD COLUMN containing_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL"
        )?;
    }
    // Survivor-matching keys (Stage 2) — partial indexes on the NON-FK
    // symbol_key column are safe. The members index on `containing_id` is
    // deferred: an index on that self-referential FK column deadlocks the
    // full-index pipeline (SQLite self-FK + indexed-FK-column interaction).
    // It belongs to Stage 3 (member lookup) and will be added once the self-FK
    // is resolved (most likely by dropping the FK and keeping containing_id a
    // plain indexed integer, with integrity maintained by survivor-matching).
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_symbols_key_global ON symbols(symbol_key) WHERE mergeable = 1;
         CREATE INDEX IF NOT EXISTS idx_symbols_key_local  ON symbols(file_id, symbol_key) WHERE mergeable = 0;
         CREATE TABLE IF NOT EXISTS symbol_locations (
             symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
             file_id   INTEGER NOT NULL REFERENCES files(id)   ON DELETE CASCADE,
             line      INTEGER NOT NULL,
             col       INTEGER NOT NULL,
             PRIMARY KEY (symbol_id, file_id)
         );
         CREATE INDEX IF NOT EXISTS idx_symloc_file ON symbol_locations(file_id);"
    )?;
    Ok(())
}

fn index_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
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
