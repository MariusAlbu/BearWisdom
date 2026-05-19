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
use crate::indexer::expand;
use crate::indexer::mem_probe;
use crate::indexer::ref_cache::RefCache;
use crate::indexer::resolve;
use crate::indexer::write;
use crate::languages::{self, LanguageRegistry};
use crate::types::{IndexStats, ParsedFile};
use crate::walker::WalkedFile;

/// Closes the C/C++ macro-catalog indexing session when the index pass
/// exits, including via early return / `?`. The session is opened at the
/// top of `full_index` so per-file extractors can compose project-root
/// + relative-path. Without the guard, an early failure would leave the
/// catalog pinned to a stale project for the next run.
struct MacroSessionGuard;
impl Drop for MacroSessionGuard {
    fn drop(&mut self) {
        crate::languages::c_lang::macro_catalog::end_index_session();
    }
}
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
    mem_probe::probe("00_start");

    // C/C++ macro discovery needs to resolve relative file paths to
    // on-disk header files. Open a session with the project root so
    // language plugins can compose `<root>/<relative>` without each one
    // re-discovering the root or the trait having to grow a parameter
    // every plugin would ignore.
    crate::languages::c_lang::macro_catalog::begin_index_session(project_root);
    let _macro_session_guard = MacroSessionGuard;

    // --- Step 1: Change detection (FullScan) ---
    emit("scanning", 0.0, None);
    let cs = changeset::full_scan(project_root, pre_walked)?;
    mem_probe::probe("01_scan_done");
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
            // Derived tables (routes, flow_edges, db_mappings, code_chunks,
            // lsp_edge_meta) must also be cleared — they reference file/symbol
            // IDs that become stale after DROP TABLE files/symbols.
            let _ = db.conn().execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP TABLE IF EXISTS lsp_edge_meta;
                 DROP TABLE IF EXISTS flow_edges;
                 DROP TABLE IF EXISTS routes;
                 DROP TABLE IF EXISTS db_mappings;
                 DROP TABLE IF EXISTS code_chunks;
                 DROP TABLE IF EXISTS edges;
                 DROP TABLE IF EXISTS imports;
                 DROP TABLE IF EXISTS unresolved_refs;
                 DROP TABLE IF EXISTS external_refs;
                 DROP TABLE IF EXISTS symbols;
                 DROP TABLE IF EXISTS files;
                 DROP TABLE IF EXISTS package_deps;
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
    mem_probe::probe("02_db_reset_done");

    // --- Steps 2-3: Read + parse (parallel via Rayon) ---
    let registry = languages::default_registry();
    let files = cs.added; // FullScan puts everything in `added`
    emit("parsing", 0.0, Some(&format!("0/{} files", files.len())));

    // Parsing runs on a dedicated rayon pool with a capped thread count.
    // The default global pool spawns one worker per logical core (24 on a
    // Ryzen 7900), and each active worker concurrently holds a tree-sitter
    // Tree + String content + in-flight ParsedFile — on a 7k-file project
    // that stacks into GB of transient RAM and can make the user's machine
    // unresponsive. Capping at `min(logical_cores, 8)` keeps ~95% of the
    // parse throughput (parsing is CPU-bound but only modestly
    // parallel-scalable past 8 threads given shared-grammar contention)
    // and cuts peak memory roughly 3x.
    //
    // Override via `BEARWISDOM_PARSE_THREADS` env var when a dedicated
    // CI runner wants to use every core.
    let parse_threads = std::env::var("BEARWISDOM_PARSE_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            cores.min(8)
        });
    let parse_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(parse_threads)
        .thread_name(|i| format!("bw-parse-{i}"))
        .stack_size(16 * 1024 * 1024)
        .build()
        .context("Failed to build parse thread pool")?;
    debug!("Parsing with {parse_threads} threads (cap for memory discipline)");

    // --- Step 3b: Detect workspace packages (filesystem-only, no parse needed) ---
    //
    // Moved BEFORE streaming parse so each parsed file can be written with
    // its package_id the first time it hits the DB, instead of a separate
    // assign_package_ids mutation pass afterward.
    let (packages, workspace_kind) = detect_packages(project_root);
    let written_packages = if !packages.is_empty() {
        let written = write::write_packages(db, &packages)
            .context("Failed to write packages")?;
        info!("Detected {} workspace packages", written.len());
        if let Some(ref kind) = workspace_kind {
            if let Err(e) = changeset::set_meta(db, "workspace_kind", kind) {
                warn!("Failed to store workspace_kind: {e}");
            }
        }
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

    // Pre-sorted (longest-prefix-wins) list of package paths → id.
    // Streaming writer uses this to stamp package_id on each file before
    // persisting to SQLite. Matches the semantics of
    // `write::assign_package_ids`: longest prefix wins, and the match must
    // end on a path separator (so `src-tauri` doesn't match a sibling
    // package rooted at `src`).
    let package_lookup: Vec<(String, i64)> = {
        let mut v: Vec<(String, Option<String>, i64)> = written_packages
            .iter()
            .filter_map(|p| {
                p.id.map(|id| (p.path.replace('\\', "/"), p.kind.clone(), id))
            })
            .collect();
        // Longest path first. When two packages share a path (e.g. a Tauri
        // root with both Cargo.toml and package.json producing `("", "cargo")`
        // and `("", "npm")`), break the tie by `kind` so the lookup order is
        // deterministic across runs. Without this the empty-path winner
        // depended on iteration order from `detect_packages`.
        v.sort_by(|a, b| {
            b.0.len()
                .cmp(&a.0.len())
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        v.into_iter().map(|(p, _, id)| (p, id)).collect()
    };
    let package_id_for_path = |rel_path: &str| -> Option<i64> {
        let normalized = rel_path.replace('\\', "/");
        for (pkg_path, id) in &package_lookup {
            if pkg_path.is_empty() {
                return Some(*id);
            }
            if normalized.starts_with(pkg_path.as_str())
                && (normalized.len() == pkg_path.len()
                    || normalized.as_bytes()[pkg_path.len()] == b'/')
            {
                return Some(*id);
            }
        }
        None
    };

    // --- Steps 3c + 4 + 4a: Streaming parse → write → FTS + chunks + slim ---
    //
    // Bounded-channel pipeline: parser workers on the capped rayon pool
    // send ParsedFiles to the main thread, which writes each one to SQLite
    // as it arrives (file row + symbols + routes + imports + FTS content
    // + code chunks), then drops the heavy content/routes/db_sets fields
    // before pushing a slim copy into the result vec.
    //
    // The old pattern was `par_iter().collect() → write_parsed_files(&)`,
    // which forced every ParsedFile to live in RAM simultaneously before
    // any write began. On a 7k-file codebase that peaked at multi-GB and
    // triggered the machine-unresponsive behaviour the user reported.
    // Streaming bounds peak memory at `channel_capacity × full ParsedFile
    // + N × slim ParsedFile` regardless of project size.
    //
    // Vendored-C detection happens inline (still needs content, so it's
    // done right after parse and before slim-down). FTS + chunks also use
    // the caller's transaction via `index_one_file_in_tx` /
    // `chunk_one_file_in_tx` — avoids thousands of per-file BEGIN/COMMIT.
    const PARSE_CHANNEL_CAP: usize = 32;
    let (parse_tx, parse_rx) = std::sync::mpsc::sync_channel::<ParsedFile>(PARSE_CHANNEL_CAP);

    // Workspace TypeArena: created once per index run, shared by every
    // extractor that opts into TypeId population AND by the SymbolIndex
    // build step. Cloning the Arc is cheap — the underlying RwLock-backed
    // arena lives at the indexer scope so TypeIds flow consistently from
    // parse through resolve.
    let workspace_arena = std::sync::Arc::new(
        crate::type_checker::core::types::TypeArena::new(),
    );

    let mut parsed: Vec<ParsedFile> = Vec::with_capacity(files.len());
    let mut vendored_c_parsed: Vec<ParsedFile> = Vec::new();
    let mut file_id_map: write::FileIdMap = std::collections::HashMap::new();
    let mut symbol_id_map: write::SymbolIdMap = std::collections::HashMap::new();
    let mut files_with_errors = 0u32;
    let mut fts_count = 0u32;
    let mut total_chunks = 0u32;

    std::thread::scope(|scope| -> Result<()> {
        // Parser worker thread: drives the rayon pool to parse files in
        // parallel and send each result into the bounded channel. When the
        // par_iter completes, the sender is dropped, which closes the
        // channel so the main-thread drain exits its loop.
        let parse_tx_for_workers = parse_tx.clone();
        let files_for_workers = &files;
        let registry_for_workers = registry;
        let pool_for_workers = &parse_pool;
        let arena_for_workers = std::sync::Arc::clone(&workspace_arena);
        scope.spawn(move || {
            pool_for_workers.install(|| {
                files_for_workers.par_iter().for_each_with(
                    parse_tx_for_workers,
                    |tx, w| {
                        match parse_file_with_arena_and_demand(
                            w,
                            registry_for_workers,
                            None,
                            arena_for_workers.as_ref(),
                        ) {
                            Ok(pf) => {
                                let _ = tx.send(pf);
                            }
                            Err(e) => {
                                warn!("Failed to parse {}: {e}", w.relative_path);
                            }
                        }
                    },
                );
            });
            // `parse_tx_for_workers` drops at scope end, closing the channel.
        });
        drop(parse_tx); // main thread's copy — workers hold the live senders.

        // Main thread: open the write transaction, drain channel, write
        // each ParsedFile, slim, push to result.
        let conn = db.conn();
        let tx = conn
            .unchecked_transaction()
            .context("Failed to begin streaming write transaction")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        while let Ok(mut pf) = parse_rx.recv() {
            if pf.has_errors {
                files_with_errors += 1;
                debug!("Syntax errors in {}", pf.path);
            }

            // Stamp package_id based on path prefix match.
            pf.package_id = package_id_for_path(&pf.path);

            // Vendored-C detection uses content — do it before slim-down.
            // Wrapped in catch_unwind because this is the drain loop's only
            // content-sensitive call site: if the scanner ever panics again
            // the pipeline must not hang waiting for workers that can no
            // longer deliver to a vanished receiver (see panic_hook.rs).
            let is_vendored_c = matches!(pf.language.as_str(), "c" | "cpp")
                && match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    is_c_vendored_file(
                        &pf.language,
                        &pf.path,
                        pf.content.as_deref().unwrap_or(""),
                    )
                })) {
                    Ok(flag) => flag,
                    Err(e) => {
                        let msg = panic_message(&e);
                        warn!(
                            "is_c_vendored_file panicked on {}: {msg} — treating as non-vendored",
                            pf.path,
                        );
                        false
                    }
                };
            if is_vendored_c {
                let original = pf.path.clone();
                pf.path = format!("ext:c:{original}");
                debug!("C/C++ vendored external: {original}");
            }

            let origin = if is_vendored_c { "external" } else { "internal" };
            let file_id =
                write::write_one_parsed_file(&tx, &pf, origin, now, &mut symbol_id_map, /*is_full*/ true)
                    .with_context(|| format!("streaming write failed for {}", pf.path))?;
            file_id_map.insert(pf.path.clone(), file_id);

            // FTS5 + code chunks: join the outer transaction so we don't
            // pay BEGIN/COMMIT per file.
            if let Some(content) = pf.content.as_deref() {
                if let Err(e) = crate::search::content_index::index_one_file_in_tx(
                    &tx, file_id, &pf.path, content,
                ) {
                    warn!("FTS index for {} failed: {e}", pf.path);
                } else {
                    fts_count += 1;
                }
                match crate::search::chunker::chunk_one_file_in_tx(&tx, file_id, content) {
                    Ok(n) => total_chunks += n,
                    Err(e) => warn!("chunking {} failed: {e}", pf.path),
                }
            }

            // Populate RefCache while symbols + refs are still live.
            if let Some(rc) = ref_cache.as_ref() {
                if let Ok(mut guard) = rc.lock() {
                    guard.store(&pf.path, &pf.content_hash, &pf);
                }
            }

            // Slim down: drop the big per-file fields whose only consumers
            // (write / FTS / chunks) have already read them. `symbols`,
            // `refs`, `flow`, and the origin vectors stay — resolution and
            // flow matching read them downstream. `routes` and `db_sets`
            // are kept because
            // the cross-language flow adapter (see
            // `extracted_routes_to_emissions` and `extracted_db_sets_to_emissions`)
            // reads them during the resolve pass to emit HttpCall Consumer
            // and DbEntity edges respectively.
            pf.content = None;

            if is_vendored_c {
                vendored_c_parsed.push(pf);
            } else {
                parsed.push(pf);
            }
        }

        tx.commit()
            .context("Failed to commit streaming write transaction")?;

        // Invalidate query caches — symbols changed.
        if let Some(ref cache) = db.query_cache {
            cache.invalidate_all();
        }
        Ok(())
    })?;

    info!(
        "Parsed + wrote {} files ({} with syntax errors) via streaming pipeline",
        parsed.len() + vendored_c_parsed.len(),
        files_with_errors
    );
    info!(
        "Wrote {} symbols across {} files",
        symbol_id_map.len(),
        file_id_map.len()
    );
    info!("Indexed {fts_count} files for FTS5 content search");
    info!("Created {total_chunks} code chunks");

    if !vendored_c_parsed.is_empty() {
        info!(
            "Classified {} C/C++ files as vendored externals",
            vendored_c_parsed.len()
        );
    }

    // L1: per-language audit log — runs on slim parsed (symbols still live
    // until resolve).
    log_language_breakdown(&parsed);
    emit("parsing", 1.0, Some(&format!("{} files parsed", parsed.len())));
    mem_probe::probe("03_streaming_parse_done");
    emit(
        "indexing_content",
        1.0,
        Some(&format!("{total_chunks} chunks created")),
    );

    // --- Step 4b: Build the per-package project context (M2 + Phase 4) ---
    // Built BEFORE external discovery so (a) M3 can write per-package
    // dependency rows from it and (b) the resolver reuses the same
    // instance below without re-reading manifests. Phase 4: also evaluates
    // every registered ecosystem's activation predicate and stores the
    // active set on the context so externals discovery and resolution
    // share the same authoritative list.
    let distinct_langs: Vec<String> = {
        let mut set: std::collections::HashSet<String> = std::collections::HashSet::new();
        for pf in &parsed {
            set.insert(pf.language.clone());
        }
        set.into_iter().collect()
    };
    // Phase 5: bucket parsed files by workspace package so the per-package
    // activation evaluator can narrow `LanguagePresent` clauses.
    // `ParsedFile.package_id` is populated upstream during the workspace
    // package detection pass; files outside any package leave it `None`
    // and contribute to the workspace-wide set only.
    let language_presence_by_package: std::collections::HashMap<i64, std::collections::HashSet<String>> = {
        let mut map: std::collections::HashMap<i64, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for pf in &parsed {
            if let Some(pkg_id) = pf.package_id {
                map.entry(pkg_id).or_default().insert(pf.language.clone());
            }
        }
        map
    };
    let mut project_ctx =
        super::project_context::ProjectContext::initialize_with_per_package_languages(
            project_root,
            &written_packages,
            distinct_langs,
            language_presence_by_package,
            crate::ecosystem::default_registry(),
        );

    // --- Step 4b: Plugin-owned cross-file state ---
    // Each active plugin scans the parsed file slice and stores its
    // cross-file state in the bag. Populate into a separate bag first to
    // satisfy the borrow checker (can't borrow `project_ctx` mutably and
    // immutably at the same time), then assign the completed bag.
    {
        let mut plugin_state = crate::indexer::plugin_state::PluginStateBag::new();
        for plugin in registry.all() {
            if !project_ctx.language_presence.contains(plugin.id()) {
                continue;
            }
            plugin.populate_project_state(
                &mut plugin_state,
                &parsed,
                project_root,
                &project_ctx,
            );
        }
        project_ctx.plugin_state = plugin_state;
    }
    mem_probe::probe("04_project_ctx_built");

    // --- Step 4c: Write per-package dependency graph (M3) ---
    // `package_deps` rows let queries like "which packages in this monorepo
    // declare axios?" answer without re-reading every manifest. One row per
    // (package_id, ecosystem, dep_name). The table is dropped + recreated
    // by the full-reindex path above, so we can blindly insert.
    if !written_packages.is_empty() {
        let dep_rows = collect_package_dep_rows(&project_ctx);
        if !dep_rows.is_empty() {
            match write::write_package_deps(db, &dep_rows) {
                Ok(n) => info!(
                    "Wrote {} package_deps rows across {} packages",
                    n,
                    project_ctx.by_package.len()
                ),
                Err(e) => warn!("Failed to write package_deps (non-fatal): {e}"),
            }
        }
    }
    mem_probe::probe("05_package_deps_written");

    // --- Step 4d: Discover + index external dependencies ---
    //
    // External dep sources (e.g. `$GOMODCACHE/github.com/foo/bar@v1.2.3/`,
    // `node_modules/react/`, `site-packages/fastapi/`) are parsed through
    // the exact same pipeline and written with origin='external' so
    // user-facing queries filter them out. The resolver picks them up via
    // the SymbolIndex so that Tier 1.5 can turn `ext:github.com/foo` refs
    // into real edges instead of opaque `external_refs` rows.
    //
    // M3: workspace packages drive per-package locator calls; roots are
    // deduplicated globally and walked exactly once.
    // R6: build the demand set from project refs before parsing externals.
    // `parse_external_sources` consumes this to extract only the symbols each
    // package actually supplies, skipping 99% of a 1.8MB `lib.dom.d.ts` when
    // the project uses 20 DOM types.
    let demand = super::demand::DemandSet::from_parsed_files(&parsed);
    if !demand.is_empty() {
        info!(
            "R6 demand set: {} modules, {} total names",
            demand.module_count(),
            demand.total_items(),
        );
    }
    mem_probe::probe("06_demand_built");
    let ExternalParsingResult {
        parsed: mut external_parsed,
        symbol_index,
        demand_driven_roots,
        demand_driven_ecosystems,
    } = parse_external_sources(
        project_root, registry, &project_ctx, &written_packages, &demand,
    );
    mem_probe::probe("07_external_parsed");
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
        for pf in external_parsed.iter_mut() {
            pf.slim_for_resolve();
        }
    }

    // --- Step 4d.1: Demand-driven script-tag dep parse ---
    //
    // Host-language extractors (HTML, Razor, cshtml, Vue, Svelte, Astro)
    // emit `Imports` refs for every `<script src="…">` tag. This stage
    // resolves those URLs against the discovered webroot (or the host
    // file's directory for relative paths) and parses the referenced
    // vendor files as `origin='external'`.
    //
    // This is the demand-driven counterpart to the per-ecosystem
    // external locator above: instead of eagerly crawling every
    // `wwwroot/lib/*` directory (which the walker intentionally excludes
    // to avoid `.min.js` noise), we only pull in files the project
    // actually references. Replaces per-library synthetics like
    // `ecosystem/jquery_synthetics.rs` with generic reference following.
    let mut script_tag_parsed = super::script_tag_deps::parse_script_tag_deps(
        project_root, &parsed, registry,
    );
    if !script_tag_parsed.is_empty() {
        info!(
            "Parsed {} script-tag-referenced vendor files",
            script_tag_parsed.len()
        );
        let (_st_file_map, st_symbol_map) =
            write::write_parsed_files_with_origin(db, &script_tag_parsed, "external")
                .context("Failed to write script-tag external index")?;
        info!(
            "Wrote {} script-tag vendor symbols",
            st_symbol_map.len()
        );
        symbol_id_map.extend(st_symbol_map);
        for pf in script_tag_parsed.iter_mut() {
            pf.slim_for_resolve();
        }
    }

    // Vendored C/C++ files are already persisted with origin="external"
    // by the streaming pipeline above; nothing to do here.

    // Combined slice the resolver sees. External files are skipped by the
    // ref-iteration loop in resolve_and_write but their symbols are still
    // indexed as lookup targets.
    let total_cap = parsed.len()
        + external_parsed.len()
        + script_tag_parsed.len()
        + vendored_c_parsed.len();
    let mut combined_parsed: Vec<ParsedFile> = Vec::with_capacity(total_cap);
    combined_parsed.extend(parsed);
    combined_parsed.extend(external_parsed);
    combined_parsed.extend(script_tag_parsed);
    combined_parsed.extend(vendored_c_parsed);
    let mut parsed = combined_parsed;
    let mut symbol_id_map = symbol_id_map;
    mem_probe::probe("08_externals_written");
    // `_` binds so compiler doesn't flag unused — these feed the Stage 2
    // loop below.
    let _ = &demand_driven_roots;
    let _ = &demand_driven_ecosystems;

    // --- Step 4e: Seed demand from user refs (demand-driven pipeline) ---
    //
    // When a demand-driven ecosystem's eager walk was skipped, the first
    // resolve_iteration would classify every ref into externals as
    // "external_refs" (no target symbol indexed yet) rather than real
    // `edges` rows. Pre-pull files the user's direct import-qualified refs
    // demand so those resolutions land as edges on pass 1. Chain walker
    // still drives the loop below for deeper hops.
    if !symbol_index.is_empty() {
        let mut seeded = seed_demand_from_user_refs(
            &parsed, &symbol_index, registry,
        );
        if !seeded.is_empty() {
            info!(
                "Seeded {} external files from user-ref demand",
                seeded.len(),
            );
            let (_sfm, seeded_sym_map) =
                write::write_parsed_files_with_origin(db, &seeded, "external")
                    .context("Failed to write seeded external index")?;
            symbol_id_map.extend(seeded_sym_map);
            for pf in seeded.iter_mut() {
                pf.slim_for_resolve();
            }
            parsed.extend(seeded);
        }
    }
    mem_probe::probe("09_demand_seeded");

    // --- Step 5: Cross-file resolution + edge writing (Stage 2 loop) ---
    //
    // Demand-driven iteration: resolve once, let the chain walker record any
    // `(current_type, target_name)` bail-outs, pull the files that define
    // those symbols (via the demand-driven symbol index first, falling back
    // to `Ecosystem::resolve_symbol` for un-migrated ecosystems), re-resolve.
    // Edges from earlier iterations survive (`INSERT OR IGNORE`); speculative
    // `unresolved_refs` / `external_refs` are wiped between iterations so the
    // final iteration's answer is authoritative.
    //
    // Loop exit:
    //   * `stats.converged()` — chain walker recorded no bail-outs.
    //   * `estats.new_files == 0` — misses exist but no file pull answered any.
    //   * `MAX_EXPANSION_ITERATIONS` hit — safety cap against degenerate
    //     mutual recursion in external types.
    emit("resolving", 0.0, None);
    const MAX_EXPANSION_ITERATIONS: usize = 8;
    // SymbolIndex is built once on iteration 0 and augmented in-place
    // for each expand-loop iteration that adds files. Avoids the ~5-10s
    // rebuild per iteration on a 280k-symbol index — saves 40-80s on
    // aspnetcore-sized projects across the 8-iteration cap.
    let mut cached_index: Option<resolve::engine::SymbolIndex> = None;
    let parsed_len_at_iter_start = parsed.len();
    let mut rstats = resolve::resolve_iteration_with_cached_index_and_arena(
        db,
        &parsed,
        &symbol_id_map,
        Some(&project_ctx),
        &mut cached_index,
        // Iteration 0: cached_index is None so the function builds full;
        // empty new_files_slice is moot (the build path doesn't read it).
        &[],
        std::sync::Arc::clone(&workspace_arena),
    )
    .context("Failed to resolve references")?;
    let _ = parsed_len_at_iter_start;
    info!(
        "Wrote {} edges, {} external, {} unresolved references",
        rstats.resolved, rstats.external, rstats.unresolved
    );
    mem_probe::probe("10_resolve_iter_0");

    let mut iteration = 1;
    while iteration < MAX_EXPANSION_ITERATIONS && !rstats.converged() {
        let parsed_len_before = parsed.len();
        let estats = expand::expand_chain_reachability_with_index_and_arena(
            db,
            &mut parsed,
            &mut symbol_id_map,
            &rstats.chain_misses,
            registry,
            if symbol_index.is_empty() { None } else { Some(&symbol_index) },
            workspace_arena.as_ref(),
        )
        .context("Failed to expand chain reachability")?;
        if estats.new_files == 0 {
            // No file pull answered any demand — fixpoint under the lens of
            // the current ecosystems. Remaining chain misses stay as
            // unresolved/external; they're genuine resolution gaps.
            break;
        }
        db.conn().execute("DELETE FROM unresolved_refs", [])
            .context("Failed to clear unresolved_refs before re-resolve")?;
        db.conn().execute("DELETE FROM external_refs", [])
            .context("Failed to clear external_refs before re-resolve")?;
        // Augment the cached SymbolIndex with just the new files added
        // by expand instead of rebuilding from scratch.
        let new_slice = &parsed[parsed_len_before..];
        let rstats2 = resolve::resolve_iteration_with_cached_index_and_arena(
            db,
            &parsed,
            &symbol_id_map,
            Some(&project_ctx),
            &mut cached_index,
            new_slice,
            std::sync::Arc::clone(&workspace_arena),
        )
        .context("Failed to re-resolve after chain reachability expansion")?;
        info!(
            "Chain expansion iteration {}: {} edges ({:+}), {} external, {} unresolved, {} new files",
            iteration,
            rstats2.resolved,
            rstats2.resolved as i64 - rstats.resolved as i64,
            rstats2.external,
            rstats2.unresolved,
            estats.new_files,
        );
        rstats = rstats2;
        iteration += 1;
        mem_probe::probe(&format!("10_resolve_iter_{iteration}"));
    }
    if iteration == MAX_EXPANSION_ITERATIONS && !rstats.converged() {
        warn!(
            "Chain expansion hit iteration cap ({} iterations); {} misses still pending",
            MAX_EXPANSION_ITERATIONS,
            rstats.chain_misses.len(),
        );
    }
    // Materialize incoming_edge_count once, after the loop settles.
    resolve::finalize_resolution(db)
        .context("Failed to finalize resolution")?;
    emit("resolving", 1.0, Some(&format!("{} edges resolved", rstats.resolved)));
    mem_probe::probe("11_resolve_finalized");

    // --- Step 5b: Populate the RefCache while symbols + refs are still live. ---
    //
    // RefCache caches per-file `symbols` and `refs` so incremental reindex can
    // skip re-parsing unchanged files on the next pass. It clones the data
    // internally, so it's safe to drop the originals afterwards.
    if let Some(rc) = ref_cache {
        let mut guard = rc.lock().unwrap();
        guard.store_all(&parsed);
        debug!("RefCache populated: {} files", parsed.len());
    }
    mem_probe::probe("12_refcache_stored");

    // --- Step 5c: Release resolve-only fields. ---
    //
    // Resolution + flow inference are the last consumers of `symbols`,
    // `refs`, `flow`, and the parallel origin / snippet vectors. Freeing
    // the heavy vectors now strips each `ParsedFile` down to <1 KB of
    // residual state so memory pressure doesn't stack with downstream
    // allocations.
    for pf in parsed.iter_mut() {
        pf.symbols = Vec::new();
        pf.refs = Vec::new();
        pf.flow = crate::types::FlowMeta::default();
        pf.symbol_origin_languages = Vec::new();
        pf.ref_origin_languages = Vec::new();
        pf.symbol_from_snippet = Vec::new();
        pf.demand_contributions = Vec::new();
    }
    mem_probe::probe("13_parsed_slim");

    // --- Step 7a: Route discoverers ---
    //
    // Each language populates the `routes` table directly. The routes-table →
    // FlowEmission bridge below (`append_db_route_consumer_emissions`) emits
    // Consumer flows from those rows; resolver-side HttpCall emissions provide
    // the matching Producers.
    emit("connectors", 0.0, Some("Running connectors"));
    let connector_start = Instant::now();
    {
        let conn = db.conn();
        let n_elixir = crate::languages::elixir::connectors::discover_phoenix_routes(conn, project_root);
        let n_go = crate::languages::go::connectors::discover_go_routes(conn, project_root, &project_ctx);
        let n_java = crate::languages::java::connectors::discover_spring_routes(conn, project_root);
        let n_php = crate::languages::php::connectors::discover_laravel_routes(conn, project_root, &project_ctx);
        let n_django = crate::languages::python::connectors::discover_django_routes(conn, project_root, &project_ctx);
        let n_fastapi = crate::languages::python::connectors::discover_fastapi_routes(conn, project_root, &project_ctx);
        let n_rails = crate::languages::ruby::connectors::discover_rails_routes(conn, project_root, &project_ctx);
        let n_nestjs = crate::languages::typescript::connectors::discover_nestjs_routes(conn, project_root, &project_ctx);
        let n_nextjs = crate::languages::typescript::connectors::discover_nextjs_routes(conn, project_root, &project_ctx);
        let n_groovy = crate::languages::groovy::connectors::discover_groovy_routes(conn, project_root, &project_ctx);
        let total = n_elixir + n_go + n_java + n_php + n_django + n_fastapi + n_rails + n_nestjs + n_nextjs + n_groovy;
        if total > 0 {
            info!(
                "Route discovery: phoenix={n_elixir} go={n_go} spring={n_java} laravel={n_php} django={n_django} fastapi={n_fastapi} rails={n_rails} nestjs={n_nestjs} nextjs={n_nextjs} groovy={n_groovy} in {:.2}s",
                connector_start.elapsed().as_secs_f64()
            );
        }
    }
    mem_probe::probe("14_connectors_done");

    // Cross-language route → flow-edge adapter. Languages whose route
    // detection is implemented as a project-wide `Connector` (Go chi/gin,
    // Java Spring, Elixir Phoenix) populate the `routes` table during the
    // connector phase above — too late for the per-file flow emitter in
    // resolve. Materialise those rows now as Consumer NamedChannel
    // HttpCall flow edges so they pair against TS/C# producer-side
    // calls.
    {
        let mut emissions: Vec<(String, u32, crate::indexer::resolve::flow_emit::FlowEmission)> =
            Vec::new();
        if let Err(e) = crate::indexer::resolve::append_db_route_consumer_emissions(
            db.conn(),
            &mut emissions,
        ) {
            warn!("DB-route flow adapter failed: {e}");
        } else if !emissions.is_empty() {
            match crate::indexer::resolve::flush_flow_emissions_public(db.conn(), &emissions) {
                Ok(n) if n > 0 => info!("Cross-language route flow edges: {n}"),
                Err(e) => warn!("DB-route flow flush failed: {e}"),
                _ => {}
            }
        }
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
    mem_probe::probe("15_post_index_done");

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

    // RefCache was populated earlier (Step 5b) while `symbols` and `refs`
    // were still live on each ParsedFile. Nothing to do here anymore.

    Ok(stats)
}

// Stage 1 — project + package discovery. Full implementations live in
// `stage_discover.rs`; these re-exports keep the call-site names short
// inside `full_index`.
pub(crate) use super::stage_discover::{
    collect_package_dep_rows, detect_packages, log_language_breakdown,
    mark_service_packages,
};


// Single-file parsing helpers live in `parse_file.rs`. Re-export the
// public surface (`parse_file`, `parse_file_with_demand`,
// `is_c_vendored_file`) so other indexer submodules and tests keep the
// `crate::indexer::full::*` import path they had before the carve.
pub(crate) use super::parse_file::{
    is_c_vendored_file, parse_file, parse_file_with_arena_and_demand,
    parse_file_with_demand,
};
// `panic_message` is consumed by `full_index`'s catch_unwind guards;
// `is_generated_platform_header` is re-exported so `full_tests.rs` can
// keep referring to it via `super::*` after the carve.
use super::parse_file::panic_message;
#[cfg(test)]
pub(super) use super::parse_file::is_generated_platform_header;

// External-source discovery, demand seeding, and external virtual-path
// plumbing live in `stage_link.rs`.
pub(crate) use super::stage_link::{
    make_walked_file, parse_external_sources, seed_demand_from_user_refs,
    ExternalParsingResult,
};

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
        unresolved_ref_count, unresolved_ref_count_external, external_ref_count,
        route_count, db_mapping_count, flow_edge_count, package_count,
    ): (u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM files WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM symbols WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM edges),
           (SELECT COUNT(*)
            FROM unresolved_refs ur
            JOIN symbols s ON s.id = ur.source_id
            WHERE ur.from_snippet = 0 AND s.origin = 'internal'),
           (SELECT COUNT(*)
            FROM unresolved_refs ur
            JOIN symbols s ON s.id = ur.source_id
            WHERE ur.from_snippet = 0 AND s.origin = 'external'),
           (SELECT COUNT(*) FROM external_refs),
           (SELECT COUNT(*) FROM routes),
           (SELECT COUNT(*) FROM db_mappings),
           (SELECT COUNT(*) FROM flow_edges),
           (SELECT COUNT(*) FROM packages)",
        [],
        |r| Ok((
            r.get(0)?, r.get(1)?, r.get(2)?,
            r.get(3)?, r.get(4)?, r.get(5)?,
            r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?,
        )),
    )?;

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        unresolved_ref_count_external,
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
