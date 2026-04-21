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
    let mut locators: Vec<(
        crate::ecosystem::EcosystemId,
        Arc<dyn ExternalSourceLocator>,
    )> = Vec::new();
    for &id in &ctx.active_ecosystems {
        if let Some(loc) = default_locator(id) {
            locators.push((id, loc));
        }
    }

    // Step 1 — discover roots. Either single-project (one locate_roots call
    // at project_root) or per-package (one locate_roots_for_package per
    // (locator, package) pair).
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
            for (id, locator) in &locators {
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

    // R6: per-file demand lookup. For TS externals the module path lives in
    // the virtual file path (`ext:ts:react/index.d.ts` → `react`). Other
    // ecosystems haven't wired a demand mapping yet — they pass None and
    // keep the permissive extract path.
    let results: Vec<Result<ParsedFile>> = walked
        .par_iter()
        .map(|w| {
            let per_file_demand = lookup_demand_for_walked(&w.relative_path, demand);
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
    // Safety cap against pathological cases (mutual cycles not caught by
    // the `seen` set because of symlink-like aliasing, etc.).
    const MAX_PULLS_PER_SEED: usize = 20_000;

    // Single pass over `parsed`: populates `already_virtual`, `wanted_names`,
    // and seeds `queue`. One loop, one touch per ref.
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
            wanted_names.insert(r.target_name.clone());
            enqueue_named_target(
                symbol_index,
                &r.target_name,
                r.module.as_deref(),
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
        let mut pf = match super::full::parse_file(&walked, registry) {
            Ok(pf) => pf,
            Err(e) => {
                debug!("seed: parse failed for {}: {e}", walked.relative_path);
                continue;
            }
        };

        // Per-locator post-processing: npm rewrites symbols to
        // `<pkg>.<name>` so the resolver matches user's
        // `import { X } from 'pkg'` against the pulled file's symbols.
        if let Some(pkg) =
            crate::ecosystem::externals::ts_package_from_virtual_path(&pf.path).map(str::to_string)
        {
            crate::ecosystem::npm::prefix_ts_external_symbols(&mut pf, &pkg);
        }

        // Follow re-exports — but only for names in the wanted set. An
        // `export { Foo } from './types'` is interesting only when `Foo`
        // is something the user actually needs.
        let file_dir = path.parent().unwrap_or_else(|| Path::new("."));
        for r in &pf.refs {
            if r.kind != EdgeKind::Imports { continue }
            if !wanted_names.contains(&r.target_name) { continue }
            enqueue_named_target(
                symbol_index,
                &r.target_name,
                r.module.as_deref(),
                Some(file_dir),
                &mut seen_paths,
                &mut queue,
            );
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

    // Module-qualified hit via the index (direct locate).
    if let Some(m) = module {
        if let Some(path) = symbol_index.locate(m, target_name) {
            let owned = path.to_path_buf();
            if seen.insert(owned.clone()) {
                queue.push_back(owned);
            }
            return;
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

/// Resolve a TypeScript / JavaScript relative import specifier to an
/// absolute file path. Tries the extensions Node / bundlers try in order,
/// then the `index.*` variant if the specifier points at a directory.
fn resolve_ts_relative_import(base_dir: &Path, specifier: &str) -> Option<PathBuf> {
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
    let language = crate::languages::default_registry().language_by_extension(file_name)?;

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
fn virtual_path_for_pulled(abs: &Path, language: &str) -> Option<String> {
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
) -> Option<&'a HashSet<String>> {
    if is_ambient_global_external(relative_path) {
        return demand.globals();
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

/// R6: detect external files whose declarations are ambient globals (visible
/// project-wide without an import). Matches the file-path shapes emitted by
/// the `ts-lib-dom` ecosystem: `typescript/lib/lib.*.d.ts` and
/// `@types/node/**/*.d.ts`.
fn is_ambient_global_external(relative_path: &str) -> bool {
    let normalized = relative_path.replace('\\', "/");
    normalized.contains("/typescript/lib/lib.") || normalized.contains("/@types/node/")
}
