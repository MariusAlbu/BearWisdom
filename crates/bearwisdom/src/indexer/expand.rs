// =============================================================================
// indexer/expand.rs — chain-reachability second pass
//
// When the chain walker resolves `current_type` but can't step past it
// because the next member isn't indexed, that's the signal that the type's
// definition file is in an external dep we haven't parsed yet. This module
// drives a second pass that asks the demand-driven symbol index built during
// `parse_external_sources` to locate the defining file, pulls it, parses it,
// and writes it with `origin='external'`. The caller (`full.rs`) then clears
// `unresolved_refs` / `external_refs` and re-runs `resolve_iteration`.
//
// Every package ecosystem is demand-driven (see
// `Ecosystem::uses_demand_driven_parse`), so the legacy `resolve_symbol`
// fallback path that used to live here is gone — the symbol index is
// authoritative, and misses that it can't answer are genuine resolution
// gaps (project-relative types, chains that leave the language entirely,
// etc.) rather than reachability gaps.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;
use tracing::{debug, info};

use crate::db::Database;
use crate::ecosystem::SymbolLocationIndex;
use crate::indexer::full::parse_file_with_demand;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::ChainMiss;
use crate::indexer::write;
use crate::languages::LanguageRegistry;
use crate::types::{PackageInfo, ParsedFile};
use crate::walker::WalkedFile;

#[derive(Debug, Default, Clone)]
pub struct ExpansionStats {
    /// Number of unique chain misses processed.
    pub misses: usize,
    /// Number of misses the symbol index had a location for. Equivalent to
    /// "misses answered" — every located miss turns into at least one file
    /// pull (modulo already-parsed dedupe).
    pub mapped: usize,
    /// Number of files newly walked + parsed.
    pub new_files: usize,
    /// Number of new symbols added to the index.
    pub new_symbols: usize,
}

/// Run a second-pass reachability expansion driven by chain walker bail-outs.
///
/// `parsed` is mutated in place: newly parsed files are appended.
/// `symbol_id_map` is extended with the new symbols' (path, qname) → id rows.
///
/// Thin wrapper around `expand_chain_reachability_with_index` that passes an
/// empty index — no op for callers that don't have one. Kept for legacy
/// call sites; new code should invoke the indexed variant directly.
pub fn expand_chain_reachability(
    db: &mut Database,
    parsed: &mut Vec<ParsedFile>,
    symbol_id_map: &mut HashMap<(String, String), i64>,
    chain_misses: &[ChainMiss],
    _project_root: &Path,
    _project_ctx: &ProjectContext,
    _packages: &[PackageInfo],
    registry: &LanguageRegistry,
) -> Result<ExpansionStats> {
    expand_chain_reachability_with_index(
        db, parsed, symbol_id_map, chain_misses, registry, None,
    )
}

/// Symbol-index-driven chain-miss expansion. For each miss the index can
/// answer, pulls the exact file that defines the missing symbol, parses it
/// with the full extractor, and writes it to the DB with `origin='external'`.
/// Misses the index doesn't answer are dropped — they're either
/// project-relative (resolution gap, not reachability) or the target lives
/// in an un-indexed stdlib.
pub fn expand_chain_reachability_with_index(
    db: &mut Database,
    parsed: &mut Vec<ParsedFile>,
    symbol_id_map: &mut HashMap<(String, String), i64>,
    chain_misses: &[ChainMiss],
    registry: &LanguageRegistry,
    symbol_index: Option<&SymbolLocationIndex>,
) -> Result<ExpansionStats> {
    let mut stats = ExpansionStats {
        misses: chain_misses.len(),
        ..Default::default()
    };
    if chain_misses.is_empty() {
        return Ok(stats);
    }
    let Some(index) = symbol_index else {
        debug!("expand: no symbol index available, nothing to expand");
        return Ok(stats);
    };

    // Build the dedupe sets — ext: paths already parsed this run and a set of
    // absolute paths already queued for this pass so multiple misses for the
    // same file don't parse it twice. `per_file_demand` accumulates, per
    // walked file, the set of symbol names the chain walker actually wants
    // — every miss's `target_name` plus the last segment of `current_type`
    // (the name the walker already resolved to and whose members we need).
    // Passed to `extract_with_demand` so a located .d.ts is extracted only
    // for the handful of names we asked about.
    let mut new_walked: Vec<WalkedFile> = Vec::new();
    let mut seen_paths: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    let mut already_walked: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut per_file_demand: HashMap<std::path::PathBuf, std::collections::HashSet<String>> =
        HashMap::new();
    for pf in parsed.iter() {
        if pf.path.starts_with("ext:") {
            already_walked.insert(pf.path.clone());
        }
    }

    for miss in chain_misses {
        let hits = locate_via_symbol_index(index, miss);
        if hits.is_empty() { continue }
        stats.mapped += 1;
        let current_leaf = miss
            .current_type
            .rsplit(['.', '\\', '/', ':'])
            .next()
            .unwrap_or("")
            .to_string();
        for path in hits {
            let demand_entry = per_file_demand.entry(path.clone()).or_default();
            demand_entry.insert(miss.target_name.clone());
            if !current_leaf.is_empty() {
                demand_entry.insert(current_leaf.clone());
            }
            if !seen_paths.insert(path.clone()) { continue }
            let Some(language) = language_from_file_ext(&path) else {
                // Extension the indexer can't parse — skip.
                continue;
            };
            let virtual_path = virtual_path_for_indexed_file(&path, language);
            if already_walked.contains(&virtual_path) { continue }
            new_walked.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }

    if new_walked.is_empty() {
        debug!(
            "expand: {} misses, {} had no index location, 0 new files",
            chain_misses.len(),
            chain_misses.len() - stats.mapped,
        );
        return Ok(stats);
    }
    debug!("expand: {} new files to parse", new_walked.len());

    // Parse new files in parallel. Errors are logged but not fatal.
    // Apply the same post-parse hook `seed_demand_from_user_refs` uses so
    // pulled TS externals get their symbols prefixed with `<pkg>.` before
    // the index sees them — otherwise expand-path qnames would diverge
    // from seed-path qnames for the same file across iterations.
    let new_parsed: Vec<ParsedFile> = new_walked
        .par_iter()
        .filter_map(|w| {
            let demand = per_file_demand.get(&w.absolute_path);
            match parse_file_with_demand(w, registry, demand) {
                Ok(mut pf) => {
                    if let Some(pkg) = crate::ecosystem::externals::ts_package_from_virtual_path(
                        &pf.path,
                    )
                    .map(str::to_string)
                    {
                        crate::ecosystem::npm::prefix_ts_external_symbols(&mut pf, &pkg);
                    }
                    Some(pf)
                }
                Err(e) => {
                    debug!("expand: parse failed for {}: {e}", w.relative_path);
                    None
                }
            }
        })
        .collect();

    if new_parsed.is_empty() {
        return Ok(stats);
    }

    // Write with origin='external'. The write path upserts on path, so any
    // accidental duplicate of a pass-1 file is harmless.
    let mut new_parsed = new_parsed;
    let (_file_map, new_id_map) =
        write::write_parsed_files_with_origin(db, &new_parsed, "external")
            .context("expand: failed to write expanded external symbols")?;
    stats.new_files = new_parsed.len();
    stats.new_symbols = new_id_map.len();
    symbol_id_map.extend(new_id_map);
    for pf in new_parsed.iter_mut() {
        pf.slim_for_resolve();
    }
    parsed.extend(new_parsed);

    info!(
        "Chain reachability expansion: {} misses → {} mapped → {} new files, {} new symbols",
        stats.misses, stats.mapped, stats.new_files, stats.new_symbols,
    );
    Ok(stats)
}

/// Query the symbol index for every file plausibly defining the miss's
/// target. Tries the most type-scoped forms first and only falls back to
/// bare-name lookup when the scoped probes all miss — otherwise a chain
/// miss on a common name like `Request` / `Client` / `Builder` would
/// pull every file in the index that happens to define SOMETHING by that
/// name, across dozens of unrelated packages.
fn locate_via_symbol_index(
    index: &SymbolLocationIndex,
    miss: &ChainMiss,
) -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let push_all = |name: &str, out: &mut Vec<std::path::PathBuf>| {
        for (_, path) in index.find_by_name(name) {
            out.push(path.to_path_buf());
        }
    };

    // Phase A — type-scoped probes. Every probe has the type prefix
    // baked in, so hits here are always for THIS type's members and
    // never for an unrelated type across the index.
    //   1. `{current_type}.{target_name}` — full method key.
    //   2. `{last_seg}.{target_name}`     — unwrap dotted type.
    //   3. `current_type`                 — receiver-type file (TS/JS:
    //      the type's body holds properties the method-key probe misses).
    let full = format!("{}.{}", miss.current_type, miss.target_name);
    push_all(&full, &mut out);

    if let Some(last_seg) = miss.current_type.rsplit('.').next() {
        if last_seg != miss.current_type {
            let short = format!("{}.{}", last_seg, miss.target_name);
            push_all(&short, &mut out);
        }
    }

    // Type-only probe — still carries the type name so same-type scoping.
    push_all(&miss.current_type, &mut out);

    // Phase B — bare-name fallback. Only fires when no type-scoped probe
    // found a candidate. For unknown-receiver chains (anonymous object,
    // externally-defined type we haven't indexed yet) this is the only
    // way to surface a plausible target. The blast radius is contained
    // to misses where we have literally nothing else to go on.
    if out.is_empty() {
        push_all(&miss.target_name, &mut out);
        if let Some(last_seg) = miss.current_type.rsplit('.').next() {
            if last_seg != miss.current_type {
                push_all(last_seg, &mut out);
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

/// Build a virtual path for a file located through the symbol index.
///
/// Must match the shape used by `stage_link::seed_demand_from_user_refs` —
/// if the two paths disagree, a file pulled by the seed on iteration 1 and
/// re-pulled by chain expansion on iteration 2 gets a different
/// `ParsedFile::path`, defeats the `already_walked` dedupe check, and gets
/// parsed + written twice with mismatched qnames (because post-processing
/// hooks like `prefix_ts_external_symbols` key off the `ext:ts:<pkg>/...`
/// shape that the seed produces).
///
/// Falls back to `ext:idx:<absolute>` when the ecosystem-specific shape
/// isn't applicable (same as `make_walked_file` in `stage_link`).
fn virtual_path_for_indexed_file(
    path: &std::path::Path,
    language: &str,
) -> String {
    super::stage_link::virtual_path_for_pulled(path, language)
        .unwrap_or_else(|| format!("ext:idx:{}", path.to_string_lossy().replace('\\', "/")))
}

/// Infer the language id for a file pulled through the demand-driven symbol
/// index. Delegates to the shared language registry so every plugin's
/// `extensions()` declaration is the single source of truth — callers never
/// maintain parallel extension tables. Returns `None` for extensions the
/// indexer can't parse so the caller drops the hit instead of mis-routing it.
fn language_from_file_ext(path: &std::path::Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    crate::languages::default_registry().language_by_extension(name)
}

#[cfg(test)]
#[path = "expand_tests.rs"]
mod tests;
