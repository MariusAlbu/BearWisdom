// =============================================================================
// indexer/expand.rs — R3 chain reachability second pass
//
// When the chain walker resolves `current_type` but can't step past it
// because the next member isn't indexed, that's the signal that the type's
// definition file was excluded by reachability narrowing. This module
// drives a second pass:
//
//   1. Re-discover ecosystem dep roots (cheap — just walks manifests).
//   2. For each ChainMiss `(current_type, target_name)`:
//        - Map `current_type` to an ExternalDepRoot via leading-segment
//          match against `module_path`.
//        - Queue a `Ecosystem::resolve_symbol(dep, current_type)` request.
//   3. Dispatch the queued requests, dedupe walked files, parse them in
//      parallel, write to DB as `external` origin.
//   4. Caller (full.rs) clears unresolved_refs/external_refs and re-runs
//      `resolve_and_write` against the augmented index.
//
// Limitation v1: only ecosystems whose `module_path` matches the FQN's
// leading segment are reachable (npm, pypi, cargo, pub, hex, rubygems,
// nimble, opam, zigpkg, cran). Maven (`group.id:artifact-id`) and
// composer (`vendor/name`) miss the leading-segment heuristic and need
// either a per-dep namespace probe or PSR-4 autoload parsing — deferred
// until measurement justifies the work.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;
use tracing::{debug, info, warn};

use crate::db::Database;
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::{default_locator, default_registry, Ecosystem, EcosystemKind};
use crate::indexer::full::parse_file;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::ChainMiss;
use crate::indexer::write;
use crate::languages::LanguageRegistry;
use crate::types::{PackageInfo, ParsedFile};

#[derive(Debug, Default, Clone)]
pub struct ExpansionStats {
    /// Number of unique chain misses processed.
    pub misses: usize,
    /// Number of misses that mapped to a known dep root.
    pub mapped: usize,
    /// Number of files newly walked + parsed by `resolve_symbol`.
    pub new_files: usize,
    /// Number of new symbols added to the index.
    pub new_symbols: usize,
}

/// Run a second-pass reachability expansion driven by chain walker bail-outs.
///
/// `parsed` is mutated in place: newly parsed files are appended.
/// `symbol_id_map` is extended with the new symbols' (path, qname) → id rows.
///
/// Returns stats describing how much new material was pulled in.
pub fn expand_chain_reachability(
    db: &mut Database,
    parsed: &mut Vec<ParsedFile>,
    symbol_id_map: &mut HashMap<(String, String), i64>,
    chain_misses: &[ChainMiss],
    project_root: &Path,
    project_ctx: &ProjectContext,
    packages: &[PackageInfo],
    registry: &LanguageRegistry,
) -> Result<ExpansionStats> {
    let mut stats = ExpansionStats {
        misses: chain_misses.len(),
        ..Default::default()
    };
    if chain_misses.is_empty() {
        return Ok(stats);
    }

    // 1. Re-discover dep roots. This re-runs the cheap discovery pass that
    //    parse_external_sources already did once; it doesn't re-walk or
    //    re-parse. We need the dep roots in hand so we can address
    //    resolve_symbol calls at the right ExternalDepRoot.
    let (dep_roots, ecosystem_by_tag) =
        rediscover_dep_roots_and_ecosystems(project_root, project_ctx, packages);
    if dep_roots.is_empty() {
        debug!("expand: no dep roots discovered, nothing to expand");
        return Ok(stats);
    }

    // Build a name → dep_idx map from already-indexed external symbols.
    // For each external file (path starting with `ext:`), parse the dep name
    // from the virtual path tag (`ext:<lang>:<dep>/...`) and map every
    // symbol's simple name + qualified name to its owning dep.
    //
    // This catches misses whose `current_type` is a short name like
    // "Assertion" (not reachable via leading-segment match) but whose type
    // IS in the index from a partial walk: lookup tells us "Assertion lives
    // in chai's source tree" and the reload pulls more chai files.
    let name_to_dep_idx = build_name_to_dep_idx(parsed, &dep_roots);

    // 2. Map each miss to a (dep_idx, fqn) request. Misses that don't
    //    map to any dep are dropped — typically these are project-relative
    //    types (resolution gap, not a reachability gap).
    let mut requests: HashMap<usize, HashSet<String>> = HashMap::new();
    for miss in chain_misses {
        let dep_idx = infer_dep_idx_for_fqn(&miss.current_type, &dep_roots)
            .or_else(|| name_to_dep_idx.get(&miss.current_type).copied())
            .or_else(|| {
                // Try first segment of dotted current_type.
                miss.current_type
                    .split('.')
                    .next()
                    .and_then(|head| name_to_dep_idx.get(head).copied())
            });
        if let Some(idx) = dep_idx {
            stats.mapped += 1;
            requests
                .entry(idx)
                .or_default()
                .insert(miss.current_type.clone());
        }
    }

    if requests.is_empty() {
        debug!("expand: {} misses, none mapped to a dep", chain_misses.len());
        let dep_names: Vec<&str> = dep_roots
            .iter()
            .take(20)
            .map(|d| d.module_path.as_str())
            .collect();
        debug!("expand: known deps (sample): {dep_names:?}");
        let sample_misses: Vec<&str> = chain_misses
            .iter()
            .take(15)
            .map(|m| m.current_type.as_str())
            .collect();
        debug!("expand: miss current_types (sample): {sample_misses:?}");
        return Ok(stats);
    }
    debug!(
        "expand: {} misses → {} mapped → {} unique deps",
        chain_misses.len(),
        stats.mapped,
        requests.len(),
    );

    // 3. Dispatch resolve_symbol per (dep, fqn). Dedupe walked files by
    //    absolute_path so the same file pulled by multiple misses is parsed
    //    once.
    let mut new_walked = Vec::new();
    let mut seen_paths: HashSet<std::path::PathBuf> = HashSet::new();
    // Pre-seed seen with already-walked external paths so we don't re-emit
    // a file that pass-1 already parsed.
    for pf in parsed.iter() {
        if pf.path.starts_with("ext:") {
            // Reconstructing the absolute path from a virtual path is
            // ecosystem-specific; instead, gather the unique absolute paths
            // by the file's content origin if available. Conservative
            // fallback: skip — duplicate parsing is harmless because
            // write_parsed_files_with_origin upserts on path.
        }
    }
    for (idx, fqns) in &requests {
        let dep = &dep_roots[*idx];
        let Some(eco) = ecosystem_by_tag.get(dep.ecosystem) else { continue };
        // Stdlib ecosystems are walked eagerly upfront — skip the per-symbol
        // pull (it would duplicate work and dedupe is path-keyed anyway).
        if eco.kind() == EcosystemKind::Stdlib {
            continue;
        }
        for fqn in fqns {
            let walked = eco.resolve_symbol(dep, fqn);
            for wf in walked {
                if seen_paths.insert(wf.absolute_path.clone()) {
                    new_walked.push(wf);
                }
            }
        }
    }

    if new_walked.is_empty() {
        debug!("expand: resolve_symbol returned no new files");
        return Ok(stats);
    }
    debug!("expand: {} new files to parse", new_walked.len());

    // 4. Parse new files. Errors are logged but not fatal.
    let new_parsed: Vec<ParsedFile> = new_walked
        .par_iter()
        .filter_map(|w| match parse_file(w, registry) {
            Ok(pf) => Some(pf),
            Err(e) => {
                debug!("expand: parse failed for {}: {e}", w.relative_path);
                None
            }
        })
        .collect();

    if new_parsed.is_empty() {
        return Ok(stats);
    }

    // 5. Write to DB with origin='external'. The write path upserts on path,
    //    so any accidental duplicate of a pass-1 file is harmless.
    let (_file_map, new_id_map) =
        write::write_parsed_files_with_origin(db, &new_parsed, "external")
            .context("expand: failed to write expanded external symbols")?;
    stats.new_files = new_parsed.len();
    stats.new_symbols = new_id_map.len();
    symbol_id_map.extend(new_id_map);
    parsed.extend(new_parsed);

    info!(
        "Chain reachability expansion: {} misses → {} mapped → {} new files, {} new symbols",
        stats.misses, stats.mapped, stats.new_files, stats.new_symbols,
    );
    Ok(stats)
}

/// Re-run dep discovery + build the legacy_tag → Ecosystem map. Mirrors
/// the discovery half of `parse_external_sources` but stops short of
/// walking/parsing — we just need addressable ExternalDepRoot handles for
/// `resolve_symbol` calls.
fn rediscover_dep_roots_and_ecosystems(
    project_root: &Path,
    ctx: &ProjectContext,
    packages: &[PackageInfo],
) -> (Vec<ExternalDepRoot>, HashMap<&'static str, Arc<dyn Ecosystem>>) {
    let mut locators: Vec<(
        crate::ecosystem::EcosystemId,
        Arc<dyn ExternalSourceLocator>,
    )> = Vec::new();
    for &id in &ctx.active_ecosystems {
        if let Some(loc) = default_locator(id) {
            locators.push((id, loc));
        }
    }

    let mut all_roots: Vec<ExternalDepRoot> = Vec::new();
    if packages.is_empty() {
        for (_id, locator) in &locators {
            all_roots.extend(locator.locate_roots(project_root));
        }
    } else {
        for pkg in packages {
            let Some(pkg_id) = pkg.id else { continue };
            let pkg_abs_path = project_root.join(&pkg.path);
            for (_id, locator) in &locators {
                all_roots.extend(locator.locate_roots_for_package(
                    project_root,
                    &pkg_abs_path,
                    pkg_id,
                ));
            }
        }
    }

    // Dedupe by (ecosystem, module_path, version, root_path) — same key
    // shape as parse_external_sources so behavior matches pass 1.
    let mut deduped: Vec<ExternalDepRoot> = Vec::new();
    let mut root_index: HashMap<(&'static str, String, String, std::path::PathBuf), usize> =
        HashMap::new();
    for root in all_roots {
        let key = (
            root.ecosystem,
            root.module_path.clone(),
            root.version.clone(),
            root.root.clone(),
        );
        if !root_index.contains_key(&key) {
            root_index.insert(key, deduped.len());
            deduped.push(root);
        }
    }

    let mut eco_by_tag: HashMap<&'static str, Arc<dyn Ecosystem>> = HashMap::new();
    for &id in &ctx.active_ecosystems {
        if let Some(eco) = default_registry().get(id).cloned() {
            if let Some(loc) = default_locator(id) {
                eco_by_tag.insert(loc.ecosystem(), eco);
            }
        }
    }

    (deduped, eco_by_tag)
}

/// Build a `simple_name → dep_idx` map from external parsed files.
///
/// Walks every ParsedFile whose path starts with `ext:`, parses the dep name
/// out of the virtual path (format `ext:<lang>:<dep>/<rel>`), and indexes
/// each symbol's simple name + qualified name against the dep that owns it.
///
/// Used to map chain misses whose `current_type` is a short name (e.g.
/// "Assertion" from chai) onto its dep — the leading-segment heuristic
/// would otherwise miss these.
fn build_name_to_dep_idx(
    parsed: &[ParsedFile],
    dep_roots: &[ExternalDepRoot],
) -> HashMap<String, usize> {
    // Pre-build: dep_module_path → dep_idx.
    let mut module_to_idx: HashMap<&str, usize> = HashMap::new();
    for (idx, dep) in dep_roots.iter().enumerate() {
        // First wins — duplicate module_paths (per-package re-emit) collapse
        // onto the earliest dep_root, which is fine since they're addressing
        // the same on-disk root.
        module_to_idx.entry(dep.module_path.as_str()).or_insert(idx);
    }

    let mut out: HashMap<String, usize> = HashMap::new();
    for pf in parsed {
        if !pf.path.starts_with("ext:") { continue }
        let Some(dep_name) = parse_dep_name_from_virtual_path(&pf.path) else { continue };
        let Some(&dep_idx) = module_to_idx.get(dep_name) else { continue };
        for sym in &pf.symbols {
            out.entry(sym.name.clone()).or_insert(dep_idx);
            out.entry(sym.qualified_name.clone()).or_insert(dep_idx);
        }
    }
    out
}

/// Parse `ext:typescript:chai/lib/Assertion.ts` → `"chai"`.
/// Handles scoped npm packages (`ext:typescript:@scope/pkg/...`).
fn parse_dep_name_from_virtual_path(path: &str) -> Option<&str> {
    let after_ext = path.strip_prefix("ext:")?;
    // Skip the language tag.
    let (_lang, rest) = after_ext.split_once(':')?;
    if rest.starts_with('@') {
        // Scoped: take first two slash-separated segments.
        let mut iter = rest.splitn(3, '/');
        let scope = iter.next()?;
        let name = iter.next()?;
        // Reconstruct using the original slice so we can return &str.
        let scope_len = scope.len();
        let total = scope_len + 1 + name.len();
        Some(&rest[..total])
    } else {
        rest.split('/').next()
    }
}

/// Given an FQN like `chai.Assertion`, find the dep_root whose `module_path`
/// matches the FQN's leading dotted segment.
///
/// Strategy: longest-prefix match. For `axios.AxiosResponse`, both `axios`
/// and (hypothetically) `axios.AxiosResponse` would match if both were dep
/// names; we prefer the longer match.
///
/// Returns None when no dep matches — typical for project-relative types
/// or for ecosystems whose `module_path` doesn't share its FQN root
/// (Maven `group.id:artifact-id`, Composer `vendor/name`).
fn infer_dep_idx_for_fqn(fqn: &str, dep_roots: &[ExternalDepRoot]) -> Option<usize> {
    let fqn_with_dot = format!("{fqn}.");
    let mut best: Option<(usize, usize)> = None; // (idx, module_path.len())
    for (idx, dep) in dep_roots.iter().enumerate() {
        let mp = dep.module_path.as_str();
        if mp.is_empty() {
            continue;
        }
        let matches = fqn == mp || fqn_with_dot.starts_with(&format!("{mp}."));
        if !matches {
            continue;
        }
        match best {
            Some((_, blen)) if mp.len() <= blen => {}
            _ => best = Some((idx, mp.len())),
        }
    }
    best.map(|(idx, _)| idx)
}

#[cfg(test)]
#[path = "expand_tests.rs"]
mod tests;

// Visibility shims so the sibling test file can reach private helpers
// without exposing them to the rest of the crate.
#[cfg(test)]
pub(super) fn _test_infer_dep_idx_for_fqn(
    fqn: &str,
    deps: &[ExternalDepRoot],
) -> Option<usize> {
    infer_dep_idx_for_fqn(fqn, deps)
}

#[cfg(test)]
pub(super) fn _test_parse_dep_name_from_virtual_path(path: &str) -> Option<&str> {
    parse_dep_name_from_virtual_path(path)
}
