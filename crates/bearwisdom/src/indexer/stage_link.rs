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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use rayon::prelude::*;
use tracing::{debug, info};

use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::{default_locator, default_registry, Ecosystem, EcosystemKind, SymbolLocationIndex};
use crate::languages::LanguageRegistry;
use crate::types::{EdgeKind, PackageInfo, ParsedFile};
use crate::walker::WalkedFile;

use super::demand::DemandSet;
use super::project_context::ProjectContext;

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
    // each package's own active set — closing the workspace-flat gap where a
    // frontend `tsconfig.json` declaring DOM previously activated `ts-lib-dom`
    // for unrelated backend packages in the same monorepo.
    let per_package_locators: HashMap<
        i64,
        Vec<(crate::ecosystem::EcosystemId, Arc<dyn ExternalSourceLocator>)>,
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
                let roots =
                    locator.locate_roots_for_package(project_root, &pkg_abs_path, pkg_id);
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

    // Step 2 — deduplicate by (ecosystem, module_path, version, root_path).
    // Root path is included so a package with BOTH a primary directory
    // (node_modules/chai/) AND a DefinitelyTyped sibling (node_modules/
    // @types/chai/) are treated as separate roots to walk.
    let mut deduped: Vec<(ExternalDepRoot, Vec<i64>)> = Vec::new();
    let mut root_index: HashMap<(&'static str, String, String, PathBuf), usize> = HashMap::new();
    for root in all_roots {
        let key = (
            root.ecosystem,
            root.module_path.clone(),
            root.version.clone(),
            root.root.clone(),
        );
        if let Some(&idx) = root_index.get(&key) {
            if let Some(pid) = root.package_id {
                if !deduped[idx].1.contains(&pid) {
                    deduped[idx].1.push(pid);
                }
            }
        } else {
            root_index.insert(key, deduped.len());
            let declaring = root.package_id.map(|p| vec![p]).unwrap_or_default();
            deduped.push((root, declaring));
        }
    }

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

    // Build the symbol index for every demand-driven ecosystem. One call
    // per ecosystem with the full set of that ecosystem's dep roots, merged
    // into a process-wide master index.
    let mut symbol_index = SymbolLocationIndex::new();
    for (tag, roots) in &demand_driven_by_eco {
        if let Some(eco) = demand_driven_ecosystems.get(tag) {
            let idx = eco.build_symbol_index(roots);
            if !idx.is_empty() {
                info!(
                    "Built demand-driven symbol index for {}: {} entries across {} roots",
                    tag,
                    idx.len(),
                    roots.len()
                );
            }
            symbol_index.extend(idx);
            // Ecosystem-declared pre-pull: entry files whose symbols are
            // broad enough to warrant eager parsing even in demand-driven
            // mode (npm type-entry files, future PyPI __init__.py, etc.).
            let pre_pull = eco.demand_pre_pull(roots);
            if !pre_pull.is_empty() {
                info!(
                    "Demand pre-pull for {}: {} entry files",
                    tag,
                    pre_pull.len()
                );
                if let Some(locator) = locator_by_ecosystem.get(tag) {
                    walked_owners
                        .extend(std::iter::repeat(locator.clone()).take(pre_pull.len()));
                }
                walked.extend(pre_pull);
            }
        }
    }

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

    // Compute the ambient-globals package set once: any discovered TS dep
    // whose entry .d.ts contributes globals via `declare global { ... }`
    // or top-level `declare namespace ...`. Cheap — bounded I/O over a
    // small number of entry files.
    let ambient_globals_packages: HashSet<String> = demand_driven_roots
        .iter()
        .filter(|r| crate::ecosystem::npm::package_declares_globals(&r.root))
        .map(|r| r.module_path.clone())
        .collect();
    if !ambient_globals_packages.is_empty() {
        debug!(
            "Ambient-globals packages (declare-global probe): {}",
            ambient_globals_packages.len()
        );
    }

    // R6: per-file demand lookup. For TS externals the module path lives in
    // the virtual file path (`ext:ts:react/index.d.ts` → `react`). Other
    // ecosystems haven't wired a demand mapping yet — they pass None and
    // keep the permissive extract path.
    let results: Vec<Result<ParsedFile>> = walked
        .par_iter()
        .map(|w| {
            let per_file_demand = lookup_demand_for_walked(
                &w.relative_path,
                demand,
                &ambient_globals_packages,
            );
            super::full::parse_file_with_demand(w, registry, per_file_demand)
        })
        .collect();

    let mut parsed = Vec::with_capacity(results.len() + metadata_parsed.len());
    let mut errors = 0usize;
    for ((walked_file, owner), res) in walked.iter().zip(walked_owners.iter()).zip(results) {
        match res {
            Ok(mut pf) => {
                // Per-locator post-processing hook: TS rewrites declaration
                // file symbols to package-qualified names here.
                owner.post_process_parsed(&mut pf);
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

// ---------------------------------------------------------------------------
// Demand seed — pull the smallest set of external files user refs demand
// ---------------------------------------------------------------------------

/// Pull the smallest set of external files needed to define every target
/// name the user actually references. The set of "wanted names" comes from
/// user refs — every `target_name` they touch (import targets, call
/// targets, type refs). We walk the symbol index + re-export tree for
/// only those names, never pulling files for re-exports of names the user
/// doesn't need.
///
/// For each user ref, resolve its target:
///   * Module-qualified (`import { Foo } from 'pkg'`) → `locate(pkg, Foo)`.
///   * Bare name → `find_by_name`.
///
/// After parsing each pulled file, follow re-exports *only for names in
/// the wanted set*. Chain-walker bail-outs during resolution pick up
/// deeper names that surface only after parse.
pub(crate) fn seed_demand_from_user_refs(
    parsed: &[ParsedFile],
    symbol_index: &SymbolLocationIndex,
    registry: &LanguageRegistry,
) -> Vec<ParsedFile> {
    // Run the BFS on a 32 MiB-stack worker. Tree-sitter extractors walking
    // deeply-nested external .d.ts files exhaust smaller budgets — 8 MiB
    // covered ts-immich's @types/node transitive deps but blew on
    // astro-awesome-privacy (web/node_modules has 9k+ .d.ts files including
    // some very deep type chains). The seed thread is short-lived and
    // single-purpose, so the budget is cheap. Scoped thread so we can
    // borrow `parsed`, `symbol_index`, and `registry` without cloning.
    std::thread::scope(|s| {
        let handle = std::thread::Builder::new()
            .name("bw-demand-seed".to_string())
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(s, move || {
                seed_demand_from_user_refs_inner(parsed, symbol_index, registry)
            })
            .expect("failed to spawn bw-demand-seed thread");
        handle.join().unwrap_or_else(|_| {
            tracing::warn!("bw-demand-seed thread panicked; returning empty seed set");
            Vec::new()
        })
    })
}

fn seed_demand_from_user_refs_inner(
    parsed: &[ParsedFile],
    symbol_index: &SymbolLocationIndex,
    registry: &LanguageRegistry,
) -> Vec<ParsedFile> {
    // Safety cap against pathological cases (mutual cycles not caught by
    // the `seen` set because of symlink-like aliasing, etc.).
    const MAX_PULLS_PER_SEED: usize = 20_000;

    // Single pass over `parsed`: populates `already_virtual`, `wanted_names`,
    // and seeds `queue`. Only refs with import context feed the seed — an
    // explicit `Imports` edge, or any ref whose module is populated (namespace-
    // qualified member chains). Bare call targets like `.map()` / `.push()`
    // are intentionally excluded: they match tens of thousands of symbols
    // across the index and previously blew seed_demand out to 18k+ pulled
    // files on ts-immich. The chain walker's demand-expansion pass
    // (expand_chain_reachability_with_index) picks up unresolved bare refs
    // during resolve iterations with proper type context.
    let mut wanted_names: HashSet<String> = HashSet::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    let mut already_virtual: HashSet<String> = HashSet::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            already_virtual.insert(pf.path.clone());
            continue;
        }
        for r in &pf.refs {
            if r.target_name.is_empty() { continue }
            // A ref with explicit `module` context routes unambiguously
            // via `locate(module, name)`. For module-less refs we can
            // still recover **ambient globals** — `document`, `Buffer`,
            // `fetch`, `process`, `HTMLElement` etc. that `scan_declare
            // _global_blocks` deposited under `NPM_GLOBALS_MODULE` — by
            // probing that synthetic module first. `Inherits`/`Implements`
            // refs without a module are also seeded via the bare-name
            // fallback: class hierarchy edges are narrow (one entry per
            // `extends`/`implements` clause) so `find_by_name` blasting
            // is safe — it only fires when the index has no module-keyed
            // entry. Plain call targets stay excluded; the chain walker's
            // expand pass picks them up later with type context.
            let is_hierarchy = matches!(r.kind, EdgeKind::Inherits | EdgeKind::Implements);
            let effective_module: Option<&str> = if r.module.is_some() {
                r.module.as_deref()
            } else if symbol_index
                .locate(
                    crate::ecosystem::npm::NPM_GLOBALS_MODULE,
                    &r.target_name,
                )
                .is_some()
            {
                Some(crate::ecosystem::npm::NPM_GLOBALS_MODULE)
            } else if is_hierarchy {
                None // bare-name fallback via find_by_name below
            } else {
                continue;
            };
            wanted_names.insert(r.target_name.clone());
            enqueue_named_target(
                symbol_index,
                &r.target_name,
                effective_module,
                None, // user files never have relative-import context here
                &mut seen_paths,
                &mut queue,
            );
        }
    }

    let mut all_parsed: Vec<ParsedFile> = Vec::new();
    let mut pulls = 0usize;

    while let Some(path) = queue.pop_front() {
        if pulls >= MAX_PULLS_PER_SEED {
            debug!("seed: hit pull cap at {pulls}, stopping");
            break;
        }
        pulls += 1;

        let Some(walked) = make_walked_file(&path, &already_virtual) else { continue };
        // Demand-filter the extraction to `wanted_names`. Without this, a
        // single lib.dom.d.ts gets extracted with ~12k symbols even though
        // the user only references ~200 types from it — hundreds of MiB of
        // extra ParsedFile state retained until phase 13.
        let mut pf = match super::full::parse_file_with_demand(&walked, registry, Some(&wanted_names)) {
            Ok(pf) => pf,
            Err(e) => {
                debug!("seed: parse failed for {}: {e}", walked.relative_path);
                continue;
            }
        };

        // Per-locator post-processing: npm rewrites symbols to
        // `<pkg>.<name>` and backfills any `declare global` /
        // `declare namespace` decls the extractor missed, so the
        // resolver matches user's `import { X } from 'pkg'` (and
        // ambient namespace refs like `Express.Multer.File`) against
        // the pulled file's symbols.
        crate::ecosystem::npm::ts_post_process_external(&mut pf);

        // Inheritance closure: follow `extends` across sibling files in the
        // same package. `BehaviorSubject.d.ts` has
        //     import { Subject } from './Subject';
        //     export declare class BehaviorSubject<T> extends Subject<T>
        // — the TS chain walker's Phase-3 inheritance walk can only reach
        // `Subject.asObservable` when Subject.d.ts is parsed too. The seed
        // doesn't otherwise follow external→external relative imports, so
        // this is the narrow door that pulls them in: only for Inherits
        // edges, matched against the file's own Imports to discover the
        // parent's module path. MAX_PULLS_PER_SEED caps any runaway.
        //
        // Generic re-exports are intentionally NOT walked — build_npm_symbol
        // _index already resolves those at index-build time and the symbol
        // index therefore points at definition files directly.
        follow_inheritance_closure(
            &pf,
            path.parent(),
            symbol_index,
            &mut wanted_names,
            &mut seen_paths,
            &mut queue,
        );

        // Transitive include walk for C/C++ external headers. `<openssl/ssl.h>`
        // includes `<openssl/x509_crt.h>` internally; without following the
        // chain, types defined in transitively-included headers stay
        // unresolved even though the user's project triggered the original
        // pull. Bounded by MAX_PULLS_PER_SEED so a runaway never escapes.
        if matches!(pf.language.as_str(), "c" | "cpp") {
            for r in &pf.refs {
                if r.kind != EdgeKind::Imports {
                    continue;
                }
                let Some(module) = r.module.as_deref() else { continue };
                if r.target_name.is_empty() {
                    continue;
                }
                wanted_names.insert(r.target_name.clone());
                enqueue_named_target(
                    symbol_index,
                    &r.target_name,
                    Some(module),
                    None,
                    &mut seen_paths,
                    &mut queue,
                );
            }
        }

        // Transitive import walk for Haskell external files. Haskell modules
        // re-export from sibling modules via `module Foo (module Bar) where`
        // — the project only imports `Foo` but the symbols are defined in `Bar`.
        // Follow `Imports` edges from the pulled external file to pull the
        // defining sub-modules. The `wanted_names` set is not restricted here
        // because Haskell re-export lists can be wildcard (`module Bar`), so
        // we pull the full sub-module and let the resolver sort out membership.
        if pf.language == "haskell" && pf.path.starts_with("ext:") {
            for r in &pf.refs {
                if r.kind != EdgeKind::Imports {
                    continue;
                }
                let Some(module) = r.module.as_deref() else { continue };
                enqueue_named_target(
                    symbol_index,
                    &r.target_name,
                    Some(module),
                    None,
                    &mut seen_paths,
                    &mut queue,
                );
            }
        }

        all_parsed.push(pf);
    }

    if !all_parsed.is_empty() {
        debug!(
            "seed: pulled {} files for {} wanted names",
            all_parsed.len(),
            wanted_names.len()
        );
    }
    all_parsed
}

/// Resolve a `(target_name, module?)` request to an absolute file path and
/// enqueue it. Module can be:
///   * `None` — user reference without import context; look up bare name.
///   * `Some("./x")` / `Some("../y")` — relative import inside an external
///     file; resolve against `relative_base` using TS module-resolution.
///   * `Some("pkg")` / `Some("@scope/pkg")` — package-absolute import;
///     route through the symbol index.
fn enqueue_named_target(
    symbol_index: &SymbolLocationIndex,
    target_name: &str,
    module: Option<&str>,
    relative_base: Option<&Path>,
    seen: &mut HashSet<PathBuf>,
    queue: &mut VecDeque<PathBuf>,
) {
    // Relative import — walk into a sibling file inside the same package.
    if let (Some(m), Some(base)) = (module, relative_base) {
        if m.starts_with('.') {
            if let Some(resolved) = resolve_ts_relative_import(base, m) {
                if seen.insert(resolved.clone()) {
                    queue.push_back(resolved);
                }
                return;
            }
        }
    }

    // Module-qualified hit via the index (direct locate). For JVM Inherits refs
    // the enriched module carries the FQN ("spock.lang.Specification") while
    // the symbol index keys by Java package ("spock.lang"). Try the direct key
    // first; if it misses, strip the last segment and retry as a package key.
    if let Some(m) = module {
        if let Some(path) = symbol_index.locate(m, target_name) {
            let owned = path.to_path_buf();
            if seen.insert(owned.clone()) {
                queue.push_back(owned);
            }
            return;
        }
        if let Some(dot) = m.rfind('.') {
            let pkg = &m[..dot];
            if let Some(path) = symbol_index.locate(pkg, target_name) {
                let owned = path.to_path_buf();
                if seen.insert(owned.clone()) {
                    queue.push_back(owned);
                }
                return;
            }
        }
    }

    // Bare-name / name-only fallback.
    if !target_name.is_empty() {
        for (_module, path) in symbol_index.find_by_name(target_name) {
            let owned = path.to_path_buf();
            if seen.insert(owned.clone()) {
                queue.push_back(owned);
            }
        }
    }
}

/// For every `Inherits` ref emitted by an external file, pair it with the
/// matching `Imports` ref (same `target_name`) in the same file to discover
/// the parent class's module. Relative modules resolve against the file's
/// own directory and enqueue the sibling file; bare modules route through
/// the symbol index. The parent name is added to `wanted_names` so the
/// parent file, when parsed, keeps the class past the demand filter.
fn follow_inheritance_closure(
    pf: &ParsedFile,
    parent_dir: Option<&Path>,
    symbol_index: &SymbolLocationIndex,
    wanted_names: &mut HashSet<String>,
    seen_paths: &mut HashSet<PathBuf>,
    queue: &mut VecDeque<PathBuf>,
) {
    // Build local alias → module map from this file's import bindings.
    // The TS extractor emits import bindings as `TypeRef` refs carrying a
    // module path (see `typescript/imports.rs::push_import`) — `Imports`
    // is reserved for CommonJS `import = require(...)`. Accept both so
    // the closure works uniformly across TS and JS externals.
    let mut import_module: HashMap<&str, &str> = HashMap::new();
    for r in &pf.refs {
        if !matches!(r.kind, EdgeKind::TypeRef | EdgeKind::Imports) {
            continue;
        }
        let Some(m) = r.module.as_deref() else { continue };
        if m.is_empty() {
            continue;
        }
        import_module.insert(r.target_name.as_str(), m);
    }

    if import_module.is_empty() {
        return;
    }

    for r in &pf.refs {
        if r.kind != EdgeKind::Inherits {
            continue;
        }
        let parent_name = r.target_name.as_str();
        let Some(&module) = import_module.get(parent_name) else {
            continue;
        };
        wanted_names.insert(parent_name.to_string());
        if module.starts_with('.') {
            let Some(base) = parent_dir else { continue };
            if let Some(resolved) = resolve_ts_relative_import(base, module) {
                if seen_paths.insert(resolved.clone()) {
                    queue.push_back(resolved);
                }
            }
        } else if let Some(path) = symbol_index.locate(module, parent_name) {
            // Direct module-keyed hit (e.g. npm package name).
            let owned = path.to_path_buf();
            if seen_paths.insert(owned.clone()) {
                queue.push_back(owned);
            }
        } else if let Some(dot) = module.rfind('.') {
            // JVM fully-qualified class name: module = "spock.mock.MockingApi".
            // The symbol index is keyed by the Java package path ("spock.mock"),
            // not the FQN. Strip the last segment and retry with the package key.
            let pkg = &module[..dot];
            if let Some(path) = symbol_index.locate(pkg, parent_name) {
                let owned = path.to_path_buf();
                if seen_paths.insert(owned.clone()) {
                    queue.push_back(owned);
                }
            }
        }
    }
}

/// Resolve a TypeScript / JavaScript relative import specifier to an
/// absolute file path. Tries the extensions Node / bundlers try in order,
/// then the `index.*` variant if the specifier points at a directory.
pub(crate) fn resolve_ts_relative_import(base_dir: &Path, specifier: &str) -> Option<PathBuf> {
    let target = base_dir.join(specifier);
    const EXTS: &[&str] = &["ts", "tsx", "d.ts", "mts", "cts", "js", "jsx", "mjs", "cjs"];
    for ext in EXTS {
        let candidate = target.with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if target.is_dir() {
        for ext in EXTS {
            let candidate = target.join(format!("index.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Wrap an absolute path in a `WalkedFile` so the shared parser can handle
/// it. Filters by extension (via the shared language registry) and by the
/// `already_virtual` set so we don't re-pull what the eager walker already
/// surfaced.
///
/// The virtual path mimics the eager walker's shape per ecosystem so
/// post-processing hooks (notably npm's package-prefix rewrite) recognize
/// pulled files. Falls back to an `ext:idx:<abs>` tag when the package
/// layout can't be inferred.
pub(crate) fn make_walked_file(
    abs: &Path,
    already_virtual: &HashSet<String>,
) -> Option<WalkedFile> {
    let file_name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // Route via the shared registry so every language's `extensions()` +
    // `language_id_for_extension()` declaration is the single source of
    // truth. No caller needs to maintain a parallel extension table.
    //
    // Fallback for C++ stdlib extensionless headers (`<vector>`,
    // `<memory>`, `<string>`, `<unordered_map>`): these have no
    // extension to drive detection, but they're real C++ headers
    // pulled through the demand loop after the posix_headers walker
    // recognized them. Treat as "cpp" so the parser actually runs.
    let language = crate::languages::default_registry()
        .language_by_extension(file_name)
        .or_else(|| {
            if crate::ecosystem::posix_headers::is_extensionless_cpp_stdlib_header(file_name) {
                Some("cpp")
            } else {
                None
            }
        })?;

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

/// Derive the ecosystem-shaped virtual path for a file pulled through the
/// demand-driven path, so per-locator `post_process_parsed` hooks (notably
/// npm's `ext:ts:<pkg>/...` → prefix symbols with `<pkg>.`) recognize the
/// pulled file the same as a walker-emitted one.
pub(crate) fn virtual_path_for_pulled(abs: &Path, language: &str) -> Option<String> {
    let s = abs.to_string_lossy().replace('\\', "/");
    match language {
        "typescript" | "tsx" | "javascript" => {
            // Standard layout: `.../node_modules/<pkg>/...`.
            let after = if let Some(nm) = s.rfind("/node_modules/") {
                s[nm + "/node_modules/".len()..].to_string()
            } else {
                // Test/override layout: `BEARWISDOM_TS_NODE_MODULES` points
                // at a directory whose direct children are packages. Treat
                // that directory as the `node_modules` root.
                let env = std::env::var_os("BEARWISDOM_TS_NODE_MODULES")?;
                let root = env.to_string_lossy().replace('\\', "/");
                let root = root.trim_end_matches('/');
                s.strip_prefix(&format!("{root}/"))?.to_string()
            };
            let parts: Vec<&str> = after.splitn(4, '/').collect();
            if parts.is_empty() { return None }
            let (pkg, rel) = if parts[0].starts_with('@') && parts.len() >= 3 {
                (format!("{}/{}", parts[0], parts[1]), parts[2..].join("/"))
            } else {
                (parts[0].to_string(), parts[1..].join("/"))
            };
            // Reject pnpm `.ignored_*` shadows and other dot-prefixed
            // directories that masquerade as packages — same gate npm.rs
            // applies at the dep-discovery side.
            if !crate::ecosystem::npm::is_valid_npm_module_path(&pkg) {
                return None;
            }
            Some(format!("ext:ts:{pkg}/{rel}"))
        }
        "go" => {
            let mod_idx = s.find("/pkg/mod/")?;
            let after = &s[mod_idx + "/pkg/mod/".len()..];
            Some(format!("ext:go/{after}"))
        }
        _ => None,
    }
}

/// R6: look up the demand set for a single external walked file based on its
/// virtual path. Returns `None` when no demand is tracked (fall through to
/// permissive extraction).
///
/// Routing:
///   * ts-lib / @types/node globals — return the `__globals__` bucket.
///   * Scoped DefinitelyTyped packages (`@types/react`) — try the demand
///     for the runtime counterpart (`react`) first; fall back to globals.
///   * Other npm packages — match the package name from the virtual path.
fn lookup_demand_for_walked<'a>(
    relative_path: &str,
    demand: &'a DemandSet,
    ambient_globals_packages: &HashSet<String>,
) -> Option<&'a HashSet<String>> {
    // Ambient-global libraries — lib.dom.d.ts, lib.es5.d.ts,
    // lib.webworker.d.ts, @types/node — declare the whole runtime type
    // surface. Filtering these by the project's user-ref demand set drops
    // interfaces the user doesn't name directly but whose instance methods
    // they still call (`Number.toFixed`, `String.trim`, `ExtendableEvent.waitUntil`).
    // A top-level interface filtered out loses all its child method symbols,
    // which leaves chain walkers nothing to land on. Parse these files
    // fully — they're the type-system floor, not an optimisable surface.
    if is_ambient_global_external(relative_path, ambient_globals_packages) {
        return None;
    }

    if let Some(pkg) = crate::ecosystem::externals::ts_package_from_virtual_path(relative_path) {
        if let Some(set) = demand.for_module(pkg) {
            return Some(set);
        }
        // DefinitelyTyped: `@types/react` demand usually lives under `react`.
        if let Some(runtime) = pkg.strip_prefix("@types/") {
            if let Some(set) = demand.for_module(runtime) {
                return Some(set);
            }
        }
    }
    None
}

/// R6: detect external files whose declarations are ambient globals
/// (visible project-wide without an import). The demand filter MUST NOT
/// run on these — top-level interfaces / classes / functions need to
/// keep all their members reachable, even when the user source never
/// names the parent type directly (the chain walker lands on members
/// via `$.each`, `Buffer.from`, etc.).
///
/// Matches:
///   * The `ts-lib-dom` synthetic `__ts_lib__` module wrapping
///     `typescript/lib/lib.*.d.ts`.
///   * Every package whose entry .d.ts contributes globals via
///     `declare global { ... }` or top-level `declare namespace ...`.
///     Set is computed by the caller via `npm::package_declares_globals`
///     across the discovered dep roots.
fn is_ambient_global_external(
    relative_path: &str,
    ambient_globals_packages: &HashSet<String>,
) -> bool {
    let normalized = relative_path.replace('\\', "/");
    if normalized.starts_with(&format!(
        "ext:ts:{}/",
        crate::ecosystem::ts_lib_dom::TS_LIB_SYNTHETIC_MODULE
    )) {
        return true;
    }
    let Some(pkg) = crate::ecosystem::externals::ts_package_from_virtual_path(&normalized)
    else {
        return false;
    };
    ambient_globals_packages.contains(pkg)
}
