// =============================================================================
// indexer/full.rs  —  full index pipeline
//
// Pipeline:
//   1. Walk the project tree (respect .gitignore) via changeset::full_scan.
//   2. Read + hash + parse each file with tree-sitter (parallel via Rayon).
//   3. Write files + symbols via shared write pipeline.
//   4. Run cross-file resolution (match unresolved refs to symbol IDs).
//   5. Index content for FTS5 + chunk for embeddings.
//   6. Run connector registry + non-flow post-steps.
//   7. Store indexed_commit in metadata (for git-aware reindex).
// =============================================================================

use crate::db::Database;
use crate::indexer::changeset;
use crate::indexer::ref_cache::RefCache;
use crate::indexer::resolve;
use crate::indexer::write;
use crate::languages::{self, LanguageRegistry};
use crate::types::{IndexStats, ParsedFile};
use crate::walker::WalkedFile;
use anyhow::{Context, Result};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info, warn};

/// Progress callback invoked at each pipeline step.
///
/// Arguments: `(step_label, progress_0_to_1, optional_detail_text)`
///
/// Step labels: `"scanning"`, `"parsing"`, `"resolving"`, `"indexing_content"`,
/// `"connectors"`.  Callers may also emit their own labels after `full_index`
/// returns (e.g. `"concepts"`, `"embedding"`).
pub type ProgressFn = Box<dyn Fn(&str, f64, Option<&str>) + Send>;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Index all source files under `project_root` and write the results to `db`.
///
/// This is a full (non-incremental) index: existing data for re-indexed files
/// is deleted via the CASCADE constraint and replaced.
///
/// `progress` is an optional callback invoked at each pipeline phase boundary.
/// Pass `None` to suppress progress notifications (CLI, tests).
///
/// `pre_walked` allows the caller to supply an already-walked file list (e.g.
/// from `bearwisdom_profile::walk_files` performed during project scanning)
/// to avoid a redundant directory traversal.  Pass `None` to walk inline.
pub fn full_index(
    db: &mut Database,
    project_root: &Path,
    progress: Option<ProgressFn>,
    pre_walked: Option<Vec<WalkedFile>>,
    ref_cache: Option<&Arc<Mutex<RefCache>>>,
) -> Result<IndexStats> {
    let emit = |step: &str, pct: f64, detail: Option<&str>| {
        if let Some(ref cb) = progress {
            cb(step, pct, detail);
        }
    };

    let start = Instant::now();
    info!("Starting full index of {}", project_root.display());

    // --- Step 1: Change detection (FullScan) ---
    emit("scanning", 0.0, None);
    let cs = changeset::full_scan(project_root, pre_walked)?;
    let file_count = cs.added.len();
    info!("Found {} source files", file_count);
    emit("scanning", 1.0, Some(&format!("{} files found", file_count)));

    // --- Step 1b: Clear existing data ---
    // For full reindex: DROP + CREATE core tables instead of DELETE.
    // DELETE on a large indexed table is O(n log n) due to index maintenance;
    // DROP + CREATE is O(1) and lets SQLite reclaim pages immediately.
    // Virtual tables (symbols_fts, fts_content, vec_chunks) are handled
    // separately to avoid leaving their internal state pointing at stale rowids.
    {
        let count: i64 = db.conn().query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap_or(0);
        if count > 0 {
            info!("Dropping and recreating index tables for full rebuild ({} existing files)", count);

            // Drop vec_chunks first (virtual table — not CASCADE-covered).
            if crate::search::vector_store::vec_table_exists(db.conn()) {
                let _ = db.conn().execute_batch("DELETE FROM vec_chunks");
            }

            // Drop the FTS trigger + virtual table so their internal rowid state
            // doesn't point at stale symbols after we drop and recreate symbols.
            // The triggers and table will be recreated by create_schema below.
            let _ = db.conn().execute_batch(
                "DROP TRIGGER IF EXISTS symbols_ai;
                 DROP TRIGGER IF EXISTS symbols_ad;
                 DROP TRIGGER IF EXISTS symbols_au;
                 DROP TABLE IF EXISTS symbols_fts;",
            );

            // Drop core tables (FK-ordered: dependents first).
            // Disable FK enforcement so we can drop in any order.
            // Derived tables (routes, flow_edges, connection_points, db_mappings,
            // code_chunks, lsp_edge_meta) must also be cleared — they reference
            // file/symbol IDs that become stale after DROP TABLE files/symbols.
            let _ = db.conn().execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP TABLE IF EXISTS lsp_edge_meta;
                 DROP TABLE IF EXISTS flow_edges;
                 DROP TABLE IF EXISTS connection_points;
                 DROP TABLE IF EXISTS routes;
                 DROP TABLE IF EXISTS db_mappings;
                 DROP TABLE IF EXISTS code_chunks;
                 DROP TABLE IF EXISTS edges;
                 DROP TABLE IF EXISTS imports;
                 DROP TABLE IF EXISTS unresolved_refs;
                 DROP TABLE IF EXISTS external_refs;
                 DROP TABLE IF EXISTS symbols;
                 DROP TABLE IF EXISTS files;
                 DROP TABLE IF EXISTS packages;
                 PRAGMA foreign_keys = ON;",
            );

            // Recreate all tables, indexes, triggers, and virtual tables
            // using the canonical schema.
            crate::db::schema::create_schema(db.conn())
                .context("Failed to recreate schema after drop")?;

            info!("Index tables recreated");
        }
    }

    // --- Steps 2-3: Read + parse (parallel via Rayon) ---
    let registry = languages::default_registry();
    let files = cs.added; // FullScan puts everything in `added`
    emit("parsing", 0.0, Some(&format!("0/{} files", files.len())));
    let results: Vec<Result<ParsedFile>> =
        files.par_iter().map(|w| parse_file(w, registry)).collect();

    let mut parsed: Vec<ParsedFile> = Vec::with_capacity(files.len());
    let mut files_with_errors = 0u32;

    for (walked, result) in files.iter().zip(results) {
        match result {
            Ok(pf) => {
                if pf.has_errors {
                    files_with_errors += 1;
                    debug!("Syntax errors in {}", walked.relative_path);
                }
                parsed.push(pf);
            }
            Err(e) => {
                warn!("Failed to parse {}: {e}", walked.relative_path);
            }
        }
    }
    info!("Parsed {} files ({} with syntax errors)", parsed.len(), files_with_errors);
    emit("parsing", 1.0, Some(&format!("{} files parsed", parsed.len())));

    // --- Step 3b: Detect workspace packages ---
    let (packages, workspace_kind) = detect_packages(project_root);
    let _packages = if !packages.is_empty() {
        let written = write::write_packages(db, &packages)
            .context("Failed to write packages")?;
        info!("Detected {} workspace packages", written.len());
        write::assign_package_ids(&mut parsed, &written);
        if let Some(ref kind) = workspace_kind {
            if let Err(e) = changeset::set_meta(db, "workspace_kind", kind) {
                warn!("Failed to store workspace_kind: {e}");
            }
        }

        // Mark packages that contain a Dockerfile as deployable services.
        let dockerfile_pairs =
            crate::languages::dockerfile::connectors::detect_dockerfiles(db.conn(), project_root);
        if !dockerfile_pairs.is_empty() {
            mark_service_packages(db.conn(), &dockerfile_pairs);
            info!(
                "Marked {} package(s) as services (Dockerfile detected)",
                dockerfile_pairs.len()
            );
        }

        written
    } else {
        Vec::new()
    };

    // --- Step 4: Write files + symbols (shared pipeline) ---
    let (file_id_map, mut symbol_id_map) =
        write::write_parsed_files(db, &parsed).context("Failed to write index to database")?;
    info!(
        "Wrote {} symbols across {} files",
        symbol_id_map.len(),
        file_id_map.len()
    );

    // --- Step 4b: Discover + index external dependencies (MVP: Go only) ---
    //
    // External dep sources (e.g. `$GOMODCACHE/github.com/foo/bar@v1.2.3/`) are
    // parsed through the exact same pipeline and written with origin='external'
    // so user-facing queries filter them out. The resolver picks them up via
    // the SymbolIndex so that Tier 1.5 can turn `ext:github.com/foo` refs into
    // real edges instead of opaque `external_refs` rows.
    let external_parsed = parse_external_sources(project_root, registry);
    if !external_parsed.is_empty() {
        info!(
            "Parsed {} external files from dependency sources",
            external_parsed.len()
        );
        let (_ext_file_map, ext_symbol_map) =
            write::write_parsed_files_with_origin(db, &external_parsed, "external")
                .context("Failed to write external index")?;
        info!(
            "Wrote {} external symbols",
            ext_symbol_map.len()
        );
        symbol_id_map.extend(ext_symbol_map);
    }

    // Combined slice the resolver sees. External files are skipped by the
    // ref-iteration loop in resolve_and_write but their symbols are still
    // indexed as lookup targets.
    let mut combined_parsed: Vec<ParsedFile> = Vec::with_capacity(parsed.len() + external_parsed.len());
    combined_parsed.extend(parsed);
    combined_parsed.extend(external_parsed);
    let parsed = combined_parsed;

    // --- Step 5: Cross-file resolution + edge writing ---
    emit("resolving", 0.0, None);
    let project_ctx = super::project_context::build_project_context(project_root);
    let rstats = resolve::resolve_and_write(db, &parsed, &symbol_id_map, Some(&project_ctx))
        .context("Failed to resolve references")?;
    info!(
        "Wrote {} edges, {} external, {} unresolved references",
        rstats.resolved, rstats.external, rstats.unresolved
    );
    emit("resolving", 1.0, Some(&format!("{} edges resolved", rstats.resolved)));

    // --- Step 6a: FTS content index (shared pipeline) ---
    emit("indexing_content", 0.0, Some("Building search index"));
    let fts_count = write::update_fts_content(db, &parsed, &file_id_map)?;
    info!("Indexed {} files for FTS5 content search", fts_count);

    // --- Step 6b: Code chunking (shared pipeline) ---
    let total_chunks = write::update_chunks(db, &parsed, &file_id_map, true)?;
    info!("Created {total_chunks} code chunks");
    emit("indexing_content", 1.0, Some(&format!("{total_chunks} chunks created")));

    // --- Step 7a: Flow connectors (registry pipeline) ---
    //
    // All cross-framework flow connectors run through the ConnectorRegistry:
    //   detect → extract ConnectionPoints → match start↔stop → write flow_edges
    //
    // 18 connectors: REST, gRPC, MQ, GraphQL, events, IPC (Tauri + Electron),
    // DI (.NET + Angular + Spring), routes (Spring, Django, FastAPI, Go, Rails,
    // Laravel, NestJS, Next.js).
    emit("connectors", 0.0, Some("Running connectors"));
    let connector_start = Instant::now();

    // Enrich routes written by tree-sitter extractors (set resolved_route where NULL).
    if let Err(e) = db.conn().execute(
        "UPDATE routes SET resolved_route = route_template WHERE resolved_route IS NULL",
        [],
    ) {
        warn!("Route enrichment failed: {e}");
    }

    let connector_registry = crate::connectors::registry::build_default_registry();
    match connector_registry.run(db.conn(), project_root, &project_ctx) {
        Ok(flow_count) => info!(
            "Connectors: {flow_count} flow edges in {:.2}s",
            connector_start.elapsed().as_secs_f64()
        ),
        Err(e) => warn!("Connector registry failed: {e}"),
    }

    // --- Step 7b: Non-flow post-index hooks ---
    //
    // Each language plugin can implement `post_index()` for enrichment that
    // writes to tables other than flow_edges (e.g. db_mappings, concepts).
    // The default implementation is a no-op, so this is safe to call on all
    // registered plugins.
    for plugin in registry.all() {
        plugin.post_index(db, project_root, &project_ctx);
    }

    emit("connectors", 1.0, None);

    // ANALYZE for query planner accuracy.
    if let Err(e) = db.conn().execute("ANALYZE", []) {
        warn!("ANALYZE failed (non-fatal): {e}");
    }

    // --- Step 8: Store indexed commit for git-aware reindex ---
    if let Some(commit) = cs.commit {
        if let Err(e) = changeset::set_meta(db, "indexed_commit", &commit) {
            warn!("Failed to store indexed_commit: {e}");
        }
    }

    let duration = start.elapsed();

    let stats = read_stats(db.conn(), files_with_errors, duration.as_millis() as u64)?;
    info!(
        "Full index complete in {:.2}s: {} files, {} symbols, {} edges, {} routes, {} db_mappings, {} packages",
        duration.as_secs_f64(),
        stats.file_count,
        stats.symbol_count,
        stats.edge_count,
        stats.route_count,
        stats.db_mapping_count,
        stats.package_count,
    );

    // Populate the pool-level ref cache (if the caller supplied one) so
    // incremental reindex can skip re-parsing unchanged dependent files on the
    // next pass.  The lock is held only long enough to drain parsed into the
    // cache; the pool connection that ran full_index is irrelevant after this.
    if let Some(rc) = ref_cache {
        let mut guard = rc.lock().unwrap();
        guard.store_all(&parsed);
        debug!("RefCache populated: {} files", parsed.len());
    }

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Parse a single file
// ---------------------------------------------------------------------------

/// Discover and parse external dependency sources for the project.
///
/// Covers Go (`$GOMODCACHE`) and Python (`site-packages`). Extending to
/// TypeScript (`node_modules`), Java (Maven local), etc. is a matter of
/// adding discovery functions in `indexer::externals` and appending to
/// the walked-files list here.
///
/// Returns an empty vec if no externals are discovered. Individual parse
/// failures are logged but don't abort.
fn parse_external_sources(project_root: &Path, registry: &LanguageRegistry) -> Vec<ParsedFile> {
    use crate::indexer::externals;

    // Collect walked files from every supported ecosystem in one batch
    // so the rayon parallel parse sees them all at once.
    let mut walked: Vec<WalkedFile> = Vec::new();

    let go_roots = externals::discover_go_externals(project_root);
    if !go_roots.is_empty() {
        info!("Discovered {} external Go dependency roots", go_roots.len());
        for dep in &go_roots {
            walked.extend(externals::walk_external_root(dep));
        }
    }

    let py_roots = externals::discover_python_externals(project_root);
    if !py_roots.is_empty() {
        info!(
            "Discovered {} external Python dependency roots",
            py_roots.len()
        );
        for dep in &py_roots {
            walked.extend(externals::walk_python_external_root(dep));
        }
    }

    let ts_roots = externals::discover_ts_externals(project_root);
    if !ts_roots.is_empty() {
        info!(
            "Discovered {} external TypeScript dependency roots",
            ts_roots.len()
        );
        for dep in &ts_roots {
            walked.extend(externals::walk_ts_external_root(dep));
        }
    }

    if walked.is_empty() {
        return Vec::new();
    }
    debug!("Walking {} external source files total", walked.len());

    let results: Vec<Result<ParsedFile>> =
        walked.par_iter().map(|w| parse_file(w, registry)).collect();

    let mut parsed = Vec::with_capacity(results.len());
    let mut errors = 0usize;
    for (walked, res) in walked.iter().zip(results) {
        match res {
            Ok(mut pf) => {
                // TS declaration files lack a package-level scope, so the
                // extractor yields bare qualified names (`Button`). The TS
                // Tier 1 resolver looks up `{import_module}.{target}`, so
                // we rewrite external TS symbols to `{package}.{name}` here.
                let pkg = ts_package_from_virtual_path(&pf.path).map(str::to_string);
                if let Some(pkg) = pkg {
                    externals::prefix_ts_external_symbols(&mut pf, &pkg);
                }
                parsed.push(pf)
            }
            Err(e) => {
                errors += 1;
                debug!("External parse failed for {}: {e}", walked.relative_path);
            }
        }
    }
    if errors > 0 {
        debug!("{errors} external files failed to parse (non-fatal)");
    }
    parsed
}

/// Extract the package name from a virtual path like `ext:ts:fake-ui/index.d.ts`
/// or `ext:ts:@tanstack/react-query/dist/index.d.ts`. Returns `None` for
/// non-TS virtual paths.
fn ts_package_from_virtual_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("ext:ts:")?;
    // Scoped package: `@foo/bar/...` — the package name is the first two
    // slash-separated segments joined.
    if rest.starts_with('@') {
        let mut parts = rest.splitn(3, '/');
        let scope = parts.next()?;
        let name = parts.next()?;
        let end_byte = scope.len() + 1 + name.len();
        Some(&rest[..end_byte])
    } else {
        let slash = rest.find('/')?;
        Some(&rest[..slash])
    }
}

pub(crate) fn parse_file(walked: &WalkedFile, registry: &LanguageRegistry) -> Result<ParsedFile> {
    let bytes = std::fs::read(&walked.absolute_path)
        .with_context(|| format!("Cannot read {}", walked.relative_path))?;

    // SHA-256 of the raw bytes for change detection.
    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    };

    let content = String::from_utf8(bytes)
        .with_context(|| format!("Non-UTF-8 content in {}", walked.relative_path))?;

    let size = content.len() as u64;
    let line_count = content.lines().count() as u32;

    // Capture mtime for fast change detection on next incremental pass.
    let mtime = std::fs::metadata(&walked.absolute_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    // Dispatch to the language plugin (dedicated or generic fallback).
    let plugin = registry.get(walked.language);
    let mut r = plugin.extract(
        &content,
        &walked.relative_path,
        walked.language,
    );

    // Run locals.scm query to filter out locally-resolved references.
    // This removes local variables, parameters, and other intra-scope names
    // that don't need cross-file resolution.
    filter_local_refs(&content, walked.language, plugin, &r.symbols, &mut r.refs);

    Ok(ParsedFile {
        path: walked.relative_path.clone(),
        language: walked.language.to_string(),
        content_hash: hash,
        size,
        line_count,
        mtime,
        package_id: None, // assigned later by assign_package_ids
        symbols: r.symbols,
        refs: r.refs,
        routes: r.routes,
        db_sets: r.db_sets,
        content: Some(content),
        has_errors: r.has_errors,
    })
}

// ---------------------------------------------------------------------------
// Local scope resolution — filters out intra-scope refs via locals.scm
// ---------------------------------------------------------------------------

fn filter_local_refs(
    source: &str,
    lang_id: &str,
    plugin: &dyn crate::languages::LanguagePlugin,
    symbols: &[crate::types::ExtractedSymbol],
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    use crate::parser::local_resolver::LocalResolver;

    // Get the locals.scm query for this language.
    let Some(locals_scm) = crate::indexer::query_builtins::locals_scm_for_language(lang_id)
    else {
        return;
    };

    // Get the grammar to compile the query.
    let Some(grammar) = plugin.grammar(lang_id) else {
        return;
    };

    // Compile the locals query (consumes a clone of grammar).
    let Some(resolver) = LocalResolver::new(locals_scm, grammar.clone()) else {
        return;
    };

    // Parse the file with tree-sitter (fast — typically <1ms).
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return;
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return;
    };

    // Run local resolution.
    let resolution = resolver.resolve(&tree, source.as_bytes());

    if resolution.resolved_count() == 0 {
        return;
    }

    // Filter out refs whose source position falls on a locally-resolved identifier.
    // We match by (line, name) since ExtractedRef stores a 0-based line number.
    // Build a set of (line, name) pairs that are locally resolved.
    let local_names_by_line = {
        let mut set = rustc_hash::FxHashSet::default();
        let line_offsets: Vec<usize> = std::iter::once(0)
            .chain(source.bytes().enumerate().filter_map(|(i, b)| {
                if b == b'\n' { Some(i + 1) } else { None }
            }))
            .collect();

        for &byte_offset in &resolution.locally_resolved {
            if byte_offset >= source.len() {
                continue;
            }
            // Convert byte offset to 0-based line number.
            // partition_point returns count of line starts <= byte_offset (1-indexed);
            // saturating_sub to get 0-based line matching ExtractedRef.line.
            let line = (line_offsets.partition_point(|&off| off <= byte_offset) as u32).saturating_sub(1);
            // Extract the identifier name at this offset.
            let end = source[byte_offset..]
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .map(|i| byte_offset + i)
                .unwrap_or(source.len());
            let name = &source[byte_offset..end];
            if !name.is_empty() {
                set.insert((line, name.to_string()));
            }
        }
        set
    };

    let before = refs.len();
    refs.retain(|r| {
        // Keep refs that have a module (imports) — those are never local.
        if r.module.is_some() {
            return true;
        }
        // Keep refs that have a chain — member access is cross-scope.
        if r.chain.is_some() {
            return true;
        }
        // Keep type refs — they reference types/classes, not local variables.
        if matches!(
            r.kind,
            crate::types::EdgeKind::TypeRef
                | crate::types::EdgeKind::Inherits
                | crate::types::EdgeKind::Implements
                | crate::types::EdgeKind::Instantiates
        ) {
            return true;
        }
        // Keep refs to names that start with uppercase — likely types/classes.
        if r.target_name.starts_with(|c: char| c.is_uppercase()) {
            return true;
        }
        // Filter out locally-resolved call refs to lowercase names (variables/params).
        !local_names_by_line.contains(&(r.line, r.target_name.clone()))
    });

    let filtered = before - refs.len();
    if filtered > 0 {
        tracing::debug!(
            lang = lang_id,
            filtered,
            remaining = refs.len(),
            "Filtered locally-resolved refs via locals.scm"
        );
    }
}

// ---------------------------------------------------------------------------
// Package detection
// ---------------------------------------------------------------------------

/// Detect workspace packages from monorepo patterns and manifest files.
///
/// Returns the package list and the workspace kind string (e.g. `"cargo-workspace"`,
/// `"pnpm-workspace"`) so the caller can persist it as `workspace_kind` metadata.
///
/// Uses bearwisdom-profile's monorepo detection first (Cargo workspace,
/// npm workspaces, Turborepo, Nx, Lerna), then falls back to scanning
/// for manifest files in immediate subdirectories.
fn detect_packages(project_root: &Path) -> (Vec<crate::types::PackageInfo>, Option<String>) {
    use crate::types::PackageInfo;

    // 1. Try bearwisdom-profile monorepo detection.
    if let Some(mono) = bearwisdom_profile::scanner::monorepo::detect_monorepo(project_root) {
        let workspace_kind = mono.kind.clone();
        let kind_hint = match mono.kind.as_str() {
            "cargo-workspace" => "cargo",
            "npm-workspaces" | "pnpm-workspace" | "turborepo" | "lerna" => "npm",
            "nx" => "npm",
            other => other,
        };

        let mut packages: Vec<PackageInfo> = Vec::new();

        if mono.packages.is_empty() {
            // Profile detected a monorepo kind but no explicit package list.
            // Scan common workspace directories (packages/, apps/, libs/, crates/).
            packages = scan_workspace_dirs(project_root, kind_hint);
        } else {
            // Profile returned explicit package paths — these may be globs or
            // directory names. Resolve each to a PackageInfo.
            for rel_path in &mono.packages {
                // Handle glob patterns like "crates/*" from Cargo workspace members.
                if rel_path.contains('*') {
                    let base = rel_path.trim_end_matches("/*").trim_end_matches("\\*");
                    let base_dir = project_root.join(base);
                    if base_dir.is_dir() {
                        if let Ok(entries) = std::fs::read_dir(&base_dir) {
                            for entry in entries.flatten() {
                                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                    continue;
                                }
                                let sub_name = entry.file_name().to_string_lossy().into_owned();
                                if sub_name.starts_with('.') { continue; }
                                let full_rel = format!("{}/{}", base, sub_name);
                                let abs = project_root.join(&full_rel);
                                let name = package_name_from_manifest(&abs, kind_hint)
                                    .unwrap_or_else(|| sub_name.clone());
                                packages.push(PackageInfo {
                                    id: None,
                                    name,
                                    path: full_rel.replace('\\', "/"),
                                    kind: Some(kind_hint.to_string()),
                                    manifest: find_manifest_path_abs(&abs, kind_hint),
                                });
                            }
                        }
                    }
                } else {
                    let abs = project_root.join(rel_path);
                    if !abs.is_dir() { continue; }
                    let name = package_name_from_manifest(&abs, kind_hint)
                        .unwrap_or_else(|| dir_name(rel_path));
                    packages.push(PackageInfo {
                        id: None,
                        name,
                        path: rel_path.replace('\\', "/"),
                        kind: Some(kind_hint.to_string()),
                        manifest: find_manifest_path_abs(&abs, kind_hint),
                    });
                }
            }
        }

        if !packages.is_empty() {
            info!("Monorepo detected ({}) — {} packages", workspace_kind, packages.len());
            return (packages, Some(workspace_kind));
        }
    }

    // 2. Fallback: scan workspace-style directories.
    let packages = scan_workspace_dirs(project_root, "unknown");
    if packages.len() >= 2 {
        info!("Fallback package scan — {} packages", packages.len());
        (packages, None)
    } else {
        (Vec::new(), None)
    }
}

/// Scan common workspace directory patterns (packages/, apps/, libs/, crates/, etc.)
/// for sub-packages containing manifest files.
fn scan_workspace_dirs(project_root: &Path, kind_hint: &str) -> Vec<crate::types::PackageInfo> {
    use crate::types::PackageInfo;

    let workspace_dirs = ["packages", "apps", "libs", "crates", "modules",
                          "services", "plugins", "integrations", "examples", "src"];
    let manifest_names: &[&str] = &[
        "package.json", "Cargo.toml", "go.mod", "pyproject.toml",
        "pubspec.yaml", "mix.exs", "Package.swift", "composer.json",
    ];
    let mut packages = Vec::new();

    for ws_dir in &workspace_dirs {
        let base = project_root.join(ws_dir);
        if !base.is_dir() { continue; }
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let sub_name = entry.file_name().to_string_lossy().into_owned();
                if sub_name.starts_with('.') || sub_name == "node_modules" {
                    continue;
                }
                let sub = entry.path();
                // Check standard manifest files.
                let mut found = false;
                for mf in manifest_names {
                    if sub.join(mf).exists() {
                        let rel = format!("{}/{}", ws_dir, sub_name);
                        let kind = if kind_hint != "unknown" { kind_hint } else { manifest_to_kind(mf) };
                        let name = package_name_from_manifest(&sub, kind)
                            .unwrap_or_else(|| sub_name.clone());
                        packages.push(PackageInfo {
                            id: None,
                            name,
                            path: rel,
                            kind: Some(kind.to_string()),
                            manifest: Some(format!("{}/{}/{}", ws_dir, sub_name, mf)),
                        });
                        found = true;
                        break;
                    }
                }
                // Check for .csproj (one per directory is a package).
                if !found {
                    if let Some(csproj) = find_csproj(&sub) {
                        let rel = format!("{}/{}", ws_dir, sub_name);
                        packages.push(PackageInfo {
                            id: None,
                            name: sub_name.clone(),
                            path: rel,
                            kind: Some("dotnet".to_string()),
                            manifest: Some(format!("{}/{}/{}", ws_dir, sub_name, csproj)),
                        });
                    }
                }
            }
        }
    }

    // Also check root-level subdirectories (for .NET solutions, Go multi-module, etc.)
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name_str = entry.file_name().to_string_lossy().into_owned();
            if dir_name_str.starts_with('.')
                || dir_name_str == "node_modules"
                || dir_name_str == "target"
                || workspace_dirs.contains(&dir_name_str.as_str())
            {
                continue;
            }
            let sub = entry.path();
            for mf in manifest_names {
                if sub.join(mf).exists() {
                    let kind = if kind_hint != "unknown" { kind_hint } else { manifest_to_kind(mf) };
                    let name = package_name_from_manifest(&sub, kind)
                        .unwrap_or_else(|| dir_name_str.clone());
                    // Avoid duplicates from workspace_dirs scan.
                    if !packages.iter().any(|p| p.path == dir_name_str) {
                        packages.push(PackageInfo {
                            id: None,
                            name,
                            path: dir_name_str.clone(),
                            kind: Some(kind.to_string()),
                            manifest: Some(format!("{}/{}", dir_name_str, mf)),
                        });
                    }
                    break;
                }
            }
        }
    }

    packages
}

/// Try to extract the native package name from a manifest file.
fn package_name_from_manifest(dir: &Path, kind: &str) -> Option<String> {
    match kind {
        "npm" => {
            let content = std::fs::read_to_string(dir.join("package.json")).ok()?;
            let v: serde_json::Value = serde_json::from_str(&content).ok()?;
            v.get("name")?.as_str().map(|s| s.to_string())
        }
        "cargo" => {
            let content = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
            // Simple TOML parse: find `name = "..."` under [package].
            let in_package = content.find("[package]")?;
            content[in_package..]
                .lines()
                .find(|l| l.trim().starts_with("name"))
                .and_then(|l| {
                    let val = l.split('=').nth(1)?.trim().trim_matches('"');
                    Some(val.to_string())
                })
        }
        "go" => {
            let content = std::fs::read_to_string(dir.join("go.mod")).ok()?;
            content.lines().next().and_then(|l| {
                l.strip_prefix("module ").map(|m| m.trim().to_string())
            })
        }
        _ => None,
    }
}

fn dir_name(rel_path: &str) -> String {
    rel_path.rsplit('/').next()
        .or_else(|| rel_path.rsplit('\\').next())
        .unwrap_or(rel_path)
        .to_string()
}

fn find_manifest_path_abs(abs_dir: &Path, kind: &str) -> Option<String> {
    let candidates: &[&str] = match kind {
        "cargo" => &["Cargo.toml"],
        "npm" => &["package.json"],
        "go" => &["go.mod"],
        "python" => &["pyproject.toml"],
        "dart" => &["pubspec.yaml"],
        "elixir" => &["mix.exs"],
        _ => &["package.json", "Cargo.toml", "go.mod", "pyproject.toml"],
    };
    for c in candidates {
        if abs_dir.join(c).exists() {
            // Return path relative to workspace root — caller uses the rel path.
            return Some(c.to_string());
        }
    }
    None
}

fn find_csproj(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".csproj") { Some(name) } else { None }
    })
}

fn manifest_to_kind(filename: &str) -> &str {
    match filename {
        "package.json" => "npm",
        "Cargo.toml" => "cargo",
        "go.mod" => "go",
        "pyproject.toml" => "python",
        "pubspec.yaml" => "dart",
        "mix.exs" => "elixir",
        "Package.swift" => "swift",
        "composer.json" => "php",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Service package marking
// ---------------------------------------------------------------------------

/// Set `is_service = 1` on packages whose path matches a detected Dockerfile.
///
/// `pairs` is `(package_relative_path, dockerfile_relative_path)` as returned
/// by `crate::languages::dockerfile::connectors::detect_dockerfiles`.
fn mark_service_packages(conn: &rusqlite::Connection, pairs: &[(String, String)]) {
    for (pkg_path, dockerfile_path) in pairs {
        match conn.execute(
            "UPDATE packages SET is_service = 1 WHERE path = ?1",
            rusqlite::params![pkg_path],
        ) {
            Ok(n) if n > 0 => {
                debug!("Marked package '{}' as service ({})", pkg_path, dockerfile_path);
            }
            Ok(_) => {
                // Package path not found — may have been cleaned up; not an error.
                debug!("No package row for path '{}' — skipping is_service mark", pkg_path);
            }
            Err(e) => {
                warn!("Failed to mark package '{}' as service: {e}", pkg_path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

pub(crate) fn read_stats(
    conn: &rusqlite::Connection,
    files_with_errors: u32,
    duration_ms: u64,
) -> Result<IndexStats> {
    let (
        file_count, symbol_count, edge_count,
        unresolved_ref_count, external_ref_count,
        route_count, db_mapping_count, flow_edge_count, package_count,
    ): (u32, u32, u32, u32, u32, u32, u32, u32, u32) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM files WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM symbols WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM edges),
           (SELECT COUNT(*) FROM unresolved_refs),
           (SELECT COUNT(*) FROM external_refs),
           (SELECT COUNT(*) FROM routes),
           (SELECT COUNT(*) FROM db_mappings),
           (SELECT COUNT(*) FROM flow_edges),
           (SELECT COUNT(*) FROM packages)",
        [],
        |r| Ok((
            r.get(0)?, r.get(1)?, r.get(2)?,
            r.get(3)?, r.get(4)?,
            r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?,
        )),
    )?;

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        external_ref_count,
        route_count,
        db_mapping_count,
        flow_edge_count,
        package_count,
        files_with_errors,
        duration_ms,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "full_tests.rs"]
mod tests;
