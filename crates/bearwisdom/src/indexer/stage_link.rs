// =============================================================================
// indexer/stage_link.rs — Stage 2: link user + external symbols into edges
//
// This is the "link" stage of the three-stage pipeline:
//
//   Stage 1 (discover) — file walk, parse user files, detect packages,
//                        build ProjectContext. Lives inline in `full.rs`.
//   Stage 2 (link)     — discover external dep roots, build the demand-driven
//                        symbol index, seed the external-file pull from user
//                        refs, resolve + iterate with chain-walker expansion.
//                        Everything here.
//   Stage 3 (connect)  — connector matching, FTS / chunks, ANALYZE. Lives
//                        inline in `full.rs` (small enough not to extract).
//
// Stage 2 used to be a 300-line-plus block inside `full_index`. Extracting
// it leaves `full.rs` focused on orchestration and keeps the demand-driven
// parse / seed / resolve machinery in one place where its data flow is
// obvious.
// =============================================================================

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rayon::prelude::*;
use tracing::{debug, info};

use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::{
    default_locator, default_registry, Ecosystem, EcosystemKind, SymbolLocationIndex,
};
use crate::languages::LanguageRegistry;
use crate::types::{EdgeKind, PackageInfo, ParsedFile};
use crate::walker::WalkedFile;

use super::demand::DemandSet;
use super::demand_symbol_index::build_demand_symbol_index;
use super::ext_virtual_path::virtual_path_for_pulled;
use super::project_context::ProjectContext;
use super::{external_root_dedup, external_root_demand};

// ---------------------------------------------------------------------------
// External-source discovery + parse
// ---------------------------------------------------------------------------

/// Result of the external-source discovery step. Carries both the eagerly
/// parsed files (legacy ecosystems) and the demand-driven symbol index
/// built from ecosystems whose `uses_demand_driven_parse` returned `true`.
/// The Stage 2 loop later queries the index to pull files on demand.
pub(crate) struct ExternalParsingResult {
    pub parsed: Vec<ParsedFile>,
    pub symbol_index: SymbolLocationIndex,
    /// Dep roots owned by demand-driven ecosystems — tracked so the Stage 2
    /// loop can rescan / extend the symbol index on new demand.
    pub demand_driven_roots: Vec<ExternalDepRoot>,
    pub demand_driven_ecosystems: HashMap<&'static str, Arc<dyn Ecosystem>>,
}

/// Discover every dep root across active ecosystems, build a demand-driven
/// symbol index for the ones that opted in, and eagerly walk the rest
/// (stdlibs + un-migrated Package ecosystems). Returns parsed files for the
/// eager slice plus the symbol index for the demand slice.
///
/// Called once per full index, per-package roots are deduped globally so a
/// dep shared across workspace packages (e.g. both apps/web and apps/server
/// declaring react 18.3.1) is walked exactly once.
pub(crate) fn parse_external_sources(
    project_root: &Path,
    registry: &LanguageRegistry,
    ctx: &ProjectContext,
    packages: &[PackageInfo],
    demand: &DemandSet,
    type_arena: &crate::type_checker::core::types::TypeArena,
) -> ExternalParsingResult {
    // Resolve every active ecosystem to its legacy locator adapter. The
    // legacy trait still carries the per-package attribution overrides
    // (`locate_roots_for_package`) and the post-parse hook.
    //
    // The workspace-wide `locators` set is still needed downstream by the
    // metadata-only pass, the locator-by-tag dedup map, and the symbol
    // index build. Per-package gating only changes which locators run
    // during root discovery (Step 1) below.
    let mut locators: Vec<(
        crate::ecosystem::EcosystemId,
        Arc<dyn ExternalSourceLocator>,
    )> = Vec::new();
    for &id in &ctx.active_ecosystems {
        if let Some(loc) = default_locator(id) {
            locators.push((id, loc));
        }
    }

    // Phase 2: per-package locator subsets, derived from
    // `ctx.active_ecosystems_by_package` (populated by Phase 1's per-package
    // activation evaluator). When non-empty, root discovery iterates only
    // each package's own active set, preventing an activation marker in one
    // package from enabling an unrelated provider in another package.
    let per_package_locators: HashMap<
        i64,
        Vec<(
            crate::ecosystem::EcosystemId,
            Arc<dyn ExternalSourceLocator>,
        )>,
    > = ctx
        .active_ecosystems_by_package
        .iter()
        .map(|(&pkg_id, ids)| {
            let locs: Vec<_> = ids
                .iter()
                .filter_map(|&id| default_locator(id).map(|l| (id, l)))
                .collect();
            (pkg_id, locs)
        })
        .collect();
    let use_per_package = !per_package_locators.is_empty();

    // Step 1 — discover roots. Either single-project (one locate_roots call
    // at project_root) or per-package (one locate_roots_for_package per
    // (locator, package) pair, with locator selection narrowed to that
    // package's per-package active set when available).
    let mut all_roots: Vec<ExternalDepRoot> = Vec::new();
    let _t_discover = Some(crate::indexer::phase_timer::scope("externals.locate_roots"));
    if packages.is_empty() {
        for (id, locator) in &locators {
            let roots = locator.locate_roots(project_root);
            if !roots.is_empty() {
                info!(
                    "Discovered {} external {} dependency roots",
                    roots.len(),
                    id
                );
            }
            all_roots.extend(roots);
        }
    } else {
        // Sorted by path before iterating: symlink-hoisted dependencies can
        // be reached again from every consuming package, producing several
        // `ExternalDepRoot`s for the same
        // physical directory under different symlink paths — the dedup key
        // below doesn't canonicalize symlinks, so those survive as distinct
        // entries. Their relative order among `all_roots` then decides the
        // first-writer-wins winner in the downstream symbol index for any
        // name the package re-exports through more than one file. `packages`
        // arrives in filesystem-scan order, which isn't a stable contract
        // across runs, so a content-derived order is required here.
        let mut packages: Vec<&PackageInfo> = packages.iter().collect();
        packages.sort_by(|a, b| a.path.cmp(&b.path));
        for pkg in packages {
            let Some(pkg_id) = pkg.id else { continue };
            let pkg_abs_path = project_root.join(&pkg.path);
            let pkg_locators: &[(
                crate::ecosystem::EcosystemId,
                Arc<dyn ExternalSourceLocator>,
            )] = if use_per_package {
                per_package_locators
                    .get(&pkg_id)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
            } else {
                locators.as_slice()
            };
            for (id, locator) in pkg_locators {
                let roots = locator.locate_roots_for_package(project_root, &pkg_abs_path, pkg_id);
                if !roots.is_empty() {
                    debug!(
                        "Package {} (id={}): {} external {} roots",
                        pkg.name,
                        pkg_id,
                        roots.len(),
                        id
                    );
                }
                all_roots.extend(roots);
            }
        }
    }

    drop(_t_discover);

    // Step 2 — widen demand to the module, then collapse the roots. A dep
    // root is a physical directory shared across the workspace while the
    // demand that reached it was collected per discovering package, so the
    // widening must happen before the collapse discards all but the first
    // discoverer's.
    external_root_demand::union_module_demand(&mut all_roots);
    let deduped = external_root_dedup::dedup_roots(all_roots);

    if !packages.is_empty() && !deduped.is_empty() {
        let total_declarations: usize = deduped.iter().map(|(_, pkgs)| pkgs.len()).sum();
        info!(
            "External discovery: {} unique roots across {} package declarations",
            deduped.len(),
            total_declarations
        );
    }

    // Build ecosystem-tag → locator index for the walk phase.
    let mut locator_by_ecosystem: HashMap<&'static str, Arc<dyn ExternalSourceLocator>> =
        HashMap::new();
    for (_id, locator) in &locators {
        locator_by_ecosystem.insert(locator.ecosystem(), locator.clone());
    }

    // Step 3 — walk source-based roots and collect metadata-only outputs.
    let mut walked: Vec<WalkedFile> = Vec::new();
    let mut walked_owners: Vec<Arc<dyn ExternalSourceLocator>> = Vec::new();
    let mut metadata_parsed: Vec<ParsedFile> = Vec::new();

    // Metadata-only path runs once per locator regardless of package layout.
    // .NET reads `{project}/obj/*.deps.json` which is already per-csproj
    // aware internally.
    {
        let _t = crate::indexer::phase_timer::scope("externals.parse_metadata_only");
        for (id, locator) in &locators {
            if let Some(pre_parsed) = locator.parse_metadata_only(project_root) {
                info!(
                    "Parsed {} external {} entries via metadata",
                    pre_parsed.len(),
                    id
                );
                metadata_parsed.extend(pre_parsed);
            }
        }
    }

    // Resolve each active ecosystem's id → Ecosystem trait impl so we can
    // branch on kind() between the eager walk (Stdlib) and reachability-based
    // resolve_import (Package). Store by the same legacy string tag used on
    // ExternalDepRoot.ecosystem so the per-root lookup is cheap.
    let mut ecosystem_by_tag: HashMap<&'static str, Arc<dyn Ecosystem>> = HashMap::new();
    for (id, locator) in &locators {
        if let Some(eco) = default_registry().get(*id) {
            ecosystem_by_tag.insert(locator.ecosystem(), eco.clone());
        }
    }

    // Partition dep roots by migration status. Ecosystems that opted into
    // demand-driven parsing skip the eager walk entirely — their symbols
    // get located on demand through the symbol index, and their files are
    // parsed only when the Stage 2 loop asks for them.
    let mut demand_driven_roots: Vec<ExternalDepRoot> = Vec::new();
    let mut demand_driven_by_eco: HashMap<&'static str, Vec<ExternalDepRoot>> = HashMap::new();
    let mut demand_driven_ecosystems: HashMap<&'static str, Arc<dyn Ecosystem>> = HashMap::new();

    let _t_walk = Some(crate::indexer::phase_timer::scope(
        "externals.eager_walk_roots",
    ));
    for (root, _declaring_pkgs) in &deduped {
        let Some(locator) = locator_by_ecosystem.get(root.ecosystem) else {
            continue;
        };
        let eco = ecosystem_by_tag.get(root.ecosystem);
        // Demand-driven: skip eager walk, collect root for later index build.
        if let Some(e) = eco {
            if e.uses_demand_driven_parse() {
                demand_driven_roots.push(root.clone());
                demand_driven_by_eco
                    .entry(root.ecosystem)
                    .or_default()
                    .push(root.clone());
                demand_driven_ecosystems
                    .entry(root.ecosystem)
                    .or_insert_with(|| e.clone());
                continue;
            }
        }
        // Remaining holdouts on the eager walk: ecosystems with no
        // source-symbol surface to build an index from — POSIX / MSVC C
        // headers and VBA TypeLib metadata blobs. The pre-parse walk stays
        // in place because there's nothing to drive demand for them.
        let files = locator.walk_root(root);
        walked_owners.extend(std::iter::repeat(locator.clone()).take(files.len()));
        walked.extend(files);
    }

    drop(_t_walk);

    let symbol_index = build_demand_symbol_index(&demand_driven_by_eco, &demand_driven_ecosystems);

    if walked.is_empty() && symbol_index.is_empty() && metadata_parsed.is_empty() {
        return ExternalParsingResult {
            parsed: Vec::new(),
            symbol_index,
            demand_driven_roots,
            demand_driven_ecosystems,
        };
    }
    if !walked.is_empty() {
        debug!("Walking {} external source files total", walked.len());
    }

    // Let the owning ecosystem identify packages that contribute ambient
    // symbols. The resulting package set is the normalized input to the
    // per-file demand policy below.
    let ambient_globals_packages =
        crate::ecosystem::external_policy::ambient_global_packages(&demand_driven_roots);
    if !ambient_globals_packages.is_empty() {
        debug!(
            "Ambient-globals packages (declare-global probe): {}",
            ambient_globals_packages.len()
        );
    }

    // R6: the ecosystem supplies the per-file demand policy. Files outside
    // that policy retain the permissive extract path.
    let results: Vec<Result<ParsedFile>> = {
        let _t = crate::indexer::phase_timer::scope("externals.parse_walked_files");
        // Deep external declarations can nest the CST far past the default
        // stack, overflowing the extractor's recursive type walk. Parse on the
        // deep PARSE_STACK_SIZE pool, same as the project parse. (A bare
        // `par_iter` runs on the global default-stack pool and can execute items
        // on the calling thread via work-stealing — which is why the overflow
        // surfaced on `main`.)
        let parse = || {
            walked
                .par_iter()
                .map(|w| {
                    let per_file_demand = match crate::ecosystem::external_policy::external_demand(
                        &w.relative_path,
                        demand,
                        &ambient_globals_packages,
                    ) {
                        crate::ecosystem::external_policy::ExternalDemandDecision::Filter(
                            symbols,
                        ) => Some(symbols),
                        crate::ecosystem::external_policy::ExternalDemandDecision::Full
                        | crate::ecosystem::external_policy::ExternalDemandDecision::Decline => {
                            None
                        }
                    };
                    super::full::parse_file_with_arena_and_demand(
                        w,
                        registry,
                        per_file_demand,
                        type_arena,
                    )
                })
                .collect()
        };
        match crate::indexer::parse_file::build_parse_pool() {
            Ok(pool) => pool.install(parse),
            Err(_) => parse(),
        }
    };

    let mut parsed = Vec::with_capacity(results.len() + metadata_parsed.len());
    let mut errors = 0usize;
    for ((walked_file, owner), res) in walked.iter().zip(walked_owners.iter()).zip(results) {
        match res {
            Ok(mut pf) => {
                // Per-locator post-processing may normalize extracted symbols.
                owner.post_process_parsed(&mut pf, type_arena);
                parsed.push(pf);
            }
            Err(e) => {
                errors += 1;
                debug!(
                    "External parse failed for {}: {e}",
                    walked_file.relative_path
                );
            }
        }
    }
    if errors > 0 {
        debug!("{errors} external files failed to parse (non-fatal)");
    }
    parsed.extend(metadata_parsed);
    ExternalParsingResult {
        parsed,
        symbol_index,
        demand_driven_roots,
        demand_driven_ecosystems,
    }
}

/// Wrap an absolute path in a `WalkedFile` so the shared parser can handle
/// it. Filters by extension (via the shared language registry) and by the
/// `already_virtual` set so we don't re-pull what the eager walker already
/// surfaced.
///
/// The virtual path mimics the eager walker's shape per ecosystem so
/// post-processing hooks recognize pulled files. Falls back to an
/// `ext:idx:<abs>` tag when the package layout can't be inferred.
pub(crate) fn make_walked_file(
    abs: &Path,
    already_virtual: &HashSet<String>,
) -> Option<WalkedFile> {
    let file_name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // Route via the shared registry so every language's `extensions()` +
    // `language_id_for_extension()` declaration is the single source of
    // truth. No caller needs to maintain a parallel extension table.
    //
    // Providers may additionally classify extensionless files they own.
    let language = crate::languages::default_registry()
        .language_by_extension(file_name)
        .or_else(|| crate::ecosystem::external_policy::extensionless_source_language(file_name))?;

    let virtual_path = virtual_path_for_pulled(abs, language)
        .unwrap_or_else(|| format!("ext:idx:{}", abs.to_string_lossy().replace('\\', "/")));
    if already_virtual.contains(&virtual_path) {
        return None;
    }
    Some(WalkedFile {
        relative_path: virtual_path,
        absolute_path: abs.to_path_buf(),
        language,
    })
}
