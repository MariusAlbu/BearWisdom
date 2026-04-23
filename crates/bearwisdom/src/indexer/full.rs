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
        let mut v: Vec<(String, i64)> = written_packages
            .iter()
            .filter_map(|p| p.id.map(|id| (p.path.replace('\\', "/"), id)))
            .collect();
        v.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        v
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
        scope.spawn(move || {
            pool_for_workers.install(|| {
                files_for_workers.par_iter().for_each_with(
                    parse_tx_for_workers,
                    |tx, w| {
                        match parse_file(w, registry_for_workers) {
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
                write::write_one_parsed_file(&tx, &pf, origin, now, &mut symbol_id_map)
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
            // `refs`, `flow`, `connection_points`, and the origin vectors
            // stay — resolution, connector detection, and flow matching
            // read them downstream.
            pf.content = None;
            pf.routes = Vec::new();
            pf.db_sets = Vec::new();

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
    let project_ctx = super::project_context::ProjectContext::initialize(
        project_root,
        &written_packages,
        distinct_langs,
        crate::ecosystem::default_registry(),
    );
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
        parsed: external_parsed,
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
    }

    // Vendored C/C++ files are already persisted with origin="external"
    // by the streaming pipeline above; nothing to do here.

    // Combined slice the resolver sees. External files are skipped by the
    // ref-iteration loop in resolve_and_write but their symbols are still
    // indexed as lookup targets.
    let total_cap = parsed.len() + external_parsed.len() + vendored_c_parsed.len();
    let mut combined_parsed: Vec<ParsedFile> = Vec::with_capacity(total_cap);
    combined_parsed.extend(parsed);
    combined_parsed.extend(external_parsed);
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
        let seeded = seed_demand_from_user_refs(
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
    let mut rstats =
        resolve::resolve_iteration(db, &parsed, &symbol_id_map, Some(&project_ctx))
            .context("Failed to resolve references")?;
    info!(
        "Wrote {} edges, {} external, {} unresolved references",
        rstats.resolved, rstats.external, rstats.unresolved
    );
    mem_probe::probe("10_resolve_iter_0");

    let mut iteration = 1;
    while iteration < MAX_EXPANSION_ITERATIONS && !rstats.converged() {
        let estats = expand::expand_chain_reachability_with_index(
            db,
            &mut parsed,
            &mut symbol_id_map,
            &rstats.chain_misses,
            registry,
            if symbol_index.is_empty() { None } else { Some(&symbol_index) },
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
        let rstats2 =
            resolve::resolve_iteration(db, &parsed, &symbol_id_map, Some(&project_ctx))
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
    // `refs`, `flow`, and the parallel origin / snippet vectors. The
    // connector pass below only reads `path`, `language`, `package_id`,
    // and `connection_points`; freeing the heavy vectors now strips each
    // `ParsedFile` down to <1 KB of residual state so memory pressure
    // doesn't stack with the connector registry's own allocations.
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

    // Note: `resolved_route` is now written inline at extract time (see
    // `write::write_parsed_files`). The post-parse UPDATE that used to run
    // here is no longer needed — connectors that later rewrite the full
    // controller-prefix-joined path still do so via per-connector UPDATEs.

    // Collect connection points emitted during extraction by
    // `LanguagePlugin::extract_connection_points`. Each plugin's
    // in-memory `crate::types::ConnectionPoint`s get joined to their
    // DB `file_id` / `symbol_id` here and handed to the matcher alongside
    // the points legacy `Connector::extract` impls pull from the DB.
    // Plugins that haven't yet migrated their connector detection emit
    // nothing here — the old path still fires for them.
    let plugin_points = crate::connectors::from_plugins::collect_plugin_connection_points(
        &parsed,
        &file_id_map,
        &symbol_id_map,
    );
    if !plugin_points.is_empty() {
        info!(
            "Collected {} plugin-emitted connection points",
            plugin_points.len()
        );
    }

    // Plugin-owned post-parse connector hook: lets each language plugin emit
    // connection points that need the fully-written DB (class-inheritance
    // lookups, method-by-parent joins, DI-container resolution).
    let mut resolved_plugin_points: Vec<crate::connectors::types::ConnectionPoint> = Vec::new();
    for plugin in registry.all() {
        let points = plugin.resolve_connection_points(db, project_root, &project_ctx);
        if !points.is_empty() {
            info!(
                "Plugin {}::resolve_connection_points: {} points",
                plugin.id(),
                points.len()
            );
            resolved_plugin_points.extend(points);
        }
    }

    let connector_registry = crate::connectors::registry::build_default_registry();
    match connector_registry.run_with_plugin_points(
        db.conn(),
        project_root,
        &project_ctx,
        &plugin_points,
        &resolved_plugin_points,
    ) {
        Ok(flow_count) => info!(
            "Connectors: {flow_count} flow edges in {:.2}s",
            connector_start.elapsed().as_secs_f64()
        ),
        Err(e) => warn!("Connector registry failed: {e}"),
    }
    mem_probe::probe("14_connectors_done");

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

// ---------------------------------------------------------------------------
// Parse a single file
// ---------------------------------------------------------------------------


// The external-source discovery, demand seed, and external virtual-path
// plumbing live in `stage_link.rs`. Re-export the types so `full_index`
// and the external-parsing integration tests can refer to them unchanged.
pub(crate) use super::stage_link::{
    make_walked_file, parse_external_sources, seed_demand_from_user_refs,
    ExternalParsingResult,
};


pub(crate) fn parse_file(walked: &WalkedFile, registry: &LanguageRegistry) -> Result<ParsedFile> {
    parse_file_with_demand(walked, registry, None)
}

/// R6 entry point for demand-driven parsing. When `demand` is `Some`, the
/// language plugin's `extract_with_demand` is called instead of `extract`,
/// and top-level declarations whose name is not in the set may be dropped.
/// Used when parsing external sources (node_modules `.d.ts`, etc.).
pub(crate) fn parse_file_with_demand(
    walked: &WalkedFile,
    registry: &LanguageRegistry,
    demand: Option<&std::collections::HashSet<String>>,
) -> Result<ParsedFile> {
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

    // Short-circuit: some projects vendor auto-generated platform headers
    // (Microsoft WebView2/WinRT MIDL output, etc.) that balloon the
    // unresolved_refs table with thousands of macro/typedef identifiers the
    // parser can't distinguish from real refs (STDMETHODCALLTYPE, IUnknown,
    // LPCWSTR, BEGIN_INTERFACE, …). These files are valid C but semantically
    // uninteresting and have no cross-project consumers. Record the file row
    // for hash tracking but emit zero symbols/refs.
    let is_generated = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        is_generated_platform_header(walked.language, &content)
    })) {
        Ok(flag) => flag,
        Err(e) => {
            let msg = panic_message(&e);
            warn!(
                "is_generated_platform_header panicked on {}: {msg} — treating as non-generated",
                walked.relative_path,
            );
            false
        }
    };
    if is_generated {
        return Ok(ParsedFile {
            path: walked.relative_path.clone(),
            language: walked.language.to_string(),
            content_hash: hash,
            size,
            line_count,
            mtime,
            package_id: None,
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            symbol_origin_languages: Vec::new(),
            ref_origin_languages: Vec::new(),
            symbol_from_snippet: Vec::new(),
            content: Some(content),
            has_errors: false,
            flow: crate::types::FlowMeta::default(),
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
        });
    }

    // Dispatch to the language plugin (dedicated or generic fallback).
    // When demand is Some, the plugin's demand-aware path runs; with None it
    // degrades to the regular `extract` via the trait's default impl.
    let plugin = registry.get(walked.language);
    let mut r = plugin.extract_with_demand(
        &content,
        &walked.relative_path,
        walked.language,
        demand,
    );

    // Run locals.scm query to filter out locally-resolved references.
    // This removes local variables, parameters, and other intra-scope names
    // that don't need cross-file resolution.
    filter_local_refs(&content, walked.language, plugin, &r.symbols, &mut r.refs);

    // Symbols produced by the host extractor all share the file's language,
    // so the origin vector starts empty and grows only when we splice in
    // sub-extracted regions below.
    let mut symbol_origin_languages: Vec<Option<String>> = Vec::new();
    // E3: parallel snippet flag — true for symbols spliced in from a
    // MarkdownFence region (fenced code in Markdown, Rust doctests, Python
    // docstring `>>>` lines). Used downstream to exclude these symbols'
    // unresolved references from aggregate resolution stats.
    let mut symbol_from_snippet: Vec<bool> = Vec::new();
    let mut ref_origin_languages: Vec<Option<String>> = Vec::new();

    // Dispatch embedded regions (Vue/Svelte/Astro/Razor/HTML/PHP/MDX) —
    // each region is sub-parsed by the declared language's plugin and the
    // results are spliced back with line/column offsets.
    let regions = plugin.embedded_regions(&content, &walked.relative_path, walked.language);
    if !regions.is_empty() {
        // Pad origin vecs so host symbols/refs are all None before embedded Some(..).
        symbol_origin_languages.resize(r.symbols.len(), None);
        symbol_from_snippet.resize(r.symbols.len(), false);
        ref_origin_languages.resize(r.refs.len(), None);
        dispatch_embedded_regions(
            &walked.relative_path,
            registry,
            regions,
            &mut r,
            &mut symbol_origin_languages,
            &mut symbol_from_snippet,
            &mut ref_origin_languages,
        );
    }

    // R5 Sprint 2: run flow-typing queries if the plugin provides a FlowConfig.
    // Populates FlowMeta (forward-inference binding map, conditional narrowings,
    // call-site type_args on chain segments). Plugins without flow_config pay
    // zero cost here.
    let flow_meta = if let (Some(flow_cfg), Some(grammar)) =
        (plugin.flow_config(), plugin.grammar(walked.language))
    {
        crate::indexer::flow::run_flow_queries(
            &content,
            &grammar,
            flow_cfg,
            &r.symbols,
            &mut r.refs,
        )
    } else {
        crate::types::FlowMeta::default()
    };

    // Cross-service wiring datapoints emitted by the plugin. Default impl
    // returns empty; plugins fill this in one framework at a time. Stage 3
    // pairs Start/Stop points across files into `flow_edges` rows without
    // touching the DB.
    let connection_points =
        plugin.extract_connection_points(&content, &walked.relative_path, walked.language);

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
        symbol_origin_languages,
        ref_origin_languages,
        symbol_from_snippet,
        content: Some(content),
        has_errors: r.has_errors,
        flow: flow_meta,
        connection_points,
        demand_contributions: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Auto-generated vendored-header detection
// ---------------------------------------------------------------------------

/// Returns `true` for C/C++ header files that were produced by a platform
/// code generator (MIDL, IDL, …) rather than hand-written application code.
///
/// These files are commonly vendored into `docs/` or `third_party/` sub-
/// directories as API references but carry thousands of platform-specific
/// macro and typedef identifiers that the C extractor cannot distinguish
/// from real references. Indexing them produces huge numbers of spurious
/// `unresolved_refs` rows (STDMETHODCALLTYPE, IUnknown, BEGIN_INTERFACE,
/// LPCWSTR, UINT32, …) with zero resolvable cross-project value.
///
/// Detection is content-based (not path-based) so we don't have to guess
/// which directories a given test project chooses to vendor under. We scan
/// the first 2048 bytes only — every known generator emits its marker in
/// the top banner comment.
fn is_generated_platform_header(language: &str, content: &str) -> bool {
    if !matches!(language, "c" | "cpp" | "c++") {
        return false;
    }
    let mut end = content.len().min(2048);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let head = &content[..end];
    const MARKERS: &[&str] = &[
        "File created by MIDL compiler",        // Microsoft MIDL (WebView2, WinRT, COM)
        "ALWAYS GENERATED file contains",        // MIDL banner variant
        "Created by: flatc compiler",            // FlatBuffers generator
        "Generated by the protocol buffer compiler", // protoc C++ output
    ];
    MARKERS.iter().any(|m| head.contains(m))
}

// ---------------------------------------------------------------------------
// Vendored C/C++ library detection
// ---------------------------------------------------------------------------

const VENDORED_PATH_SEGMENTS: &[&str] = &[
    "third_party",
    "vendor",
    "deps",
    "external",
    "extern",
];

/// Content markers that appear in the opening banner of well-known single-header
/// vendored libraries — specific enough that they don't appear in project code
/// that merely includes/uses the library.
const VENDORED_CONTENT_MARKERS: &[&str] = &[
    "JSON for Modern C++",              // nlohmann/json (banner in the ASCII logo)
    "raylib v",                         // raylib — "raylib v5.5 - A simple..."
    "raymath v",                        // raymath companion header
    "Sean Barrett",                     // STB single-file libs (all list this author)
    "Catch v",                          // Catch2 test framework (v1/v2 banner)
    "Catch2 v",                         // Catch2 (v3 banner)
    "dear imgui,",                      // Dear ImGui — "dear imgui, v1.X..."
    "GLFW 3",                           // GLFW — "GLFW 3" in main header comment
    "miniaudio - Audio playback",       // miniaudio — banner line
    "termbox2.h --",                    // termbox2 self-documentation comment
    // Sokol: the distinctive self-doc format used in ALL sokol headers.
    // The banner is "sokol_<name>.h -- description" in the first comment block.
    // This does NOT appear in files that merely include sokol headers.
    ".h -- Drop-in",
    ".h -- drop-in",
    ".h -- Minimal",
    ".h -- minimal",
    ".h -- Simple",
    ".h -- simple",
];

/// Returns `true` if a C/C++ file should be classified as a vendored
/// third-party library rather than first-party project code.
///
/// Two-tier: path segment check (conventional vendor dirs) then content
/// banner check (common single-header libraries dropped anywhere in the tree).
pub(crate) fn is_c_vendored_file(language: &str, path: &str, content: &str) -> bool {
    if !matches!(language, "c" | "cpp" | "c++") {
        return false;
    }
    let norm = path.replace('\\', "/");
    for seg in VENDORED_PATH_SEGMENTS {
        if norm.contains(&format!("/{seg}/"))
            || norm.ends_with(&format!("/{seg}"))
            || norm.starts_with(&format!("{seg}/"))
        {
            return true;
        }
    }
    // Slice the first ~4 KiB of content to scan for vendor banners.  Naive
    // byte-slicing panics when the cut falls inside a multi-byte UTF-8 char
    // (e.g. the box-drawing glyphs some Redis deps use in comments). Walk
    // back to the nearest char boundary.
    let mut end = content.len().min(4096);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let head = &content[..end];
    VENDORED_CONTENT_MARKERS.iter().any(|m| head.contains(m))
}

/// Extract a short human-readable description from a `catch_unwind` payload.
/// Used by the pipeline's panic guards so a scanner that misbehaves still
/// produces a legible warning instead of `<opaque Any>`.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}

// ---------------------------------------------------------------------------
// Embedded-region dispatch — splices sub-extracted symbols/refs back into the
// host file's vectors with line/column offsets applied.
// ---------------------------------------------------------------------------

/// Dispatch each `EmbeddedRegion` returned by a host extractor (Vue/Svelte/
/// Astro/Razor/HTML/PHP/MDX). For each region we:
///
/// 1. Punch out interpolation holes (only populated for Tier-3 string DSLs).
/// 2. Call the sub-language plugin's `extract()` against the region text.
/// 3. Run locals filtering with the sub-language's grammar+locals.scm.
/// 4. Rebase `source_symbol_index`, `parent_index`, `handler_symbol_index`,
///    and `property_symbol_index` by the current `r.symbols.len()`.
/// 5. Shift line/column positions by the region's offsets (column offset
///    applies only to positions on line 0 of the sub-extraction).
/// 6. Push spliced symbols into `r.symbols` and record `Some(language_id)`
///    in the parallel `origin_langs` vector so `write.rs` can populate the
///    `symbols.origin_language` column.
///
/// S11 supports exactly one level of embedding. Nested embedding (e.g. a
/// SQL string DSL inside a Razor `@{}` C# block) is deferred to S13+.
fn dispatch_embedded_regions(
    file_path: &str,
    registry: &LanguageRegistry,
    regions: Vec<crate::types::EmbeddedRegion>,
    r: &mut crate::types::ExtractionResult,
    origin_langs: &mut Vec<Option<String>>,
    from_snippet: &mut Vec<bool>,
    ref_origin_langs: &mut Vec<Option<String>>,
) {
    use crate::types::EmbeddedOrigin;
    for region in regions {
        let sub_plugin = registry.get(&region.language_id);
        let sub_text = if region.holes.is_empty() {
            region.text.clone()
        } else {
            punch_holes(&region.text, &region.holes)
        };

        let mut sub = sub_plugin.extract(&sub_text, file_path, &region.language_id);
        filter_local_refs(&sub_text, &region.language_id, sub_plugin, &sub.symbols, &mut sub.refs);

        let symbol_offset = r.symbols.len();
        let line_offset = region.line_offset;
        let col_offset = region.col_offset;
        // E3: Markdown fenced code, Rust doctests, and Python doctests are
        // snippet contexts — usually missing imports, so unresolved refs
        // from their symbols should be excluded from aggregate resolution
        // stats. Frontmatter (YAML/TOML/JSON) doesn't qualify.
        let region_is_snippet = matches!(region.origin, EmbeddedOrigin::MarkdownFence);

        for mut sym in sub.symbols {
            let start_on_first = sym.start_line == 0;
            let end_on_first = sym.end_line == 0;
            sym.start_line = sym.start_line.saturating_add(line_offset);
            sym.end_line = sym.end_line.saturating_add(line_offset);
            if start_on_first {
                sym.start_col = sym.start_col.saturating_add(col_offset);
            }
            if end_on_first {
                sym.end_col = sym.end_col.saturating_add(col_offset);
            }
            if let Some(parent) = sym.parent_index.as_mut() {
                *parent += symbol_offset;
            }
            // E1: host-injected wrapper prefix (e.g. Razor's synthetic
            // `__RazorBody` class) is stripped from qualified_name and
            // scope_path so user-facing names don't carry the wrapper.
            if let Some(prefix) = region.strip_scope_prefix.as_deref() {
                strip_scope_prefix_in_place(&mut sym.qualified_name, prefix);
                if let Some(sp) = sym.scope_path.as_mut() {
                    strip_scope_prefix_in_place(sp, prefix);
                }
                if let Some(sp) = sym.scope_path.as_ref() {
                    if sp.is_empty() {
                        sym.scope_path = None;
                    }
                }
            }
            r.symbols.push(sym);
            origin_langs.push(Some(region.language_id.clone()));
            from_snippet.push(region_is_snippet);
        }
        for mut rf in sub.refs {
            rf.source_symbol_index += symbol_offset;
            rf.line = rf.line.saturating_add(line_offset);
            r.refs.push(rf);
            // Tag this ref with the embedded language so the resolver
            // routes it to the correct externals/primitives table instead
            // of the host-file language.
            ref_origin_langs.push(Some(region.language_id.clone()));
        }
        for mut rt in sub.routes {
            rt.handler_symbol_index += symbol_offset;
            r.routes.push(rt);
        }
        for mut ds in sub.db_sets {
            ds.property_symbol_index += symbol_offset;
            r.db_sets.push(ds);
        }
        r.has_errors = r.has_errors || sub.has_errors;
    }
}

/// Strip a synthetic scope prefix (`"__RazorBody"`) from a dotted qualified
/// name in place. Handles both exact match and leading-with-dot forms:
///
///   * `"__RazorBody"`          → `""`   (empty — caller treats as no scope)
///   * `"__RazorBody.Foo"`      → `"Foo"`
///   * `"__RazorBody.Foo.Bar"`  → `"Foo.Bar"`
///   * `"Other.__RazorBody.X"`  → unchanged (prefix only strips at start)
fn strip_scope_prefix_in_place(name: &mut String, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    if name == prefix {
        name.clear();
        return;
    }
    let dotted = format!("{prefix}.");
    if name.starts_with(&dotted) {
        *name = name[dotted.len()..].to_string();
    }
}

/// Blank interpolation spans inside an embedded-region text, preserving byte
/// length and newlines so sub-extractor line/column numbers stay accurate.
/// Only called for Tier-3 string-DSL consumers; S11's SFC host extractors
/// leave `holes` empty and never invoke this path.
fn punch_holes(text: &str, holes: &[crate::types::Span]) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut sorted: Vec<crate::types::Span> = holes.to_vec();
    sorted.sort_by_key(|s| s.start);
    let mut cursor = 0usize;
    for hole in sorted {
        let start = hole.start.min(bytes.len());
        let end = hole.end.min(bytes.len());
        if start < cursor {
            continue; // overlapping hole; skip
        }
        out.extend_from_slice(&bytes[cursor..start]);
        for b in &bytes[start..end] {
            out.push(if *b == b'\n' { b'\n' } else { b' ' });
        }
        cursor = end;
    }
    out.extend_from_slice(&bytes[cursor..]);
    // Host extractors emitting holes are contracted to align spans on UTF-8
    // codepoint boundaries. If that invariant is violated, fall back to the
    // unpunched text — the sub-parse may fail on the interpolation but at
    // least we stay valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
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
