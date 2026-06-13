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
use crate::indexer::full::{parse_file_with_arena_and_demand, parse_file_with_demand};
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
    expand_chain_reachability_with_index(db, parsed, symbol_id_map, chain_misses, registry, None)
}

/// Same as `expand_chain_reachability_with_index` but threads a workspace
/// `TypeArena` to newly-parsed external files so any TypeId-populating
/// extractor produces canonical IDs in the same arena used by the rest of
/// the index.
pub fn expand_chain_reachability_with_index_and_arena(
    db: &mut Database,
    parsed: &mut Vec<ParsedFile>,
    symbol_id_map: &mut HashMap<(String, String), i64>,
    chain_misses: &[ChainMiss],
    registry: &LanguageRegistry,
    symbol_index: Option<&SymbolLocationIndex>,
    type_arena: &crate::type_checker::core::types::TypeArena,
) -> Result<ExpansionStats> {
    expand_chain_reachability_inner(
        db,
        parsed,
        symbol_id_map,
        chain_misses,
        registry,
        symbol_index,
        Some(type_arena),
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
    expand_chain_reachability_inner(
        db,
        parsed,
        symbol_id_map,
        chain_misses,
        registry,
        symbol_index,
        None,
    )
}

fn expand_chain_reachability_inner(
    db: &mut Database,
    parsed: &mut Vec<ParsedFile>,
    symbol_id_map: &mut HashMap<(String, String), i64>,
    chain_misses: &[ChainMiss],
    registry: &LanguageRegistry,
    symbol_index: Option<&SymbolLocationIndex>,
    type_arena: Option<&crate::type_checker::core::types::TypeArena>,
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
    // same file don't parse it twice. Each located file is pulled whole: the
    // full extractor runs over it so every symbol it defines becomes a lookup
    // target, regardless of which miss located it. Internal code may reference
    // any of those symbols by bare name, so slimming the pull to the miss's
    // demand drops symbols and leaves those refs permanently unresolved.
    let mut new_walked: Vec<WalkedFile> = Vec::new();
    let mut seen_paths: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    let mut already_walked: std::collections::HashSet<String> = std::collections::HashSet::new();
    for pf in parsed.iter() {
        if pf.path.starts_with("ext:") {
            already_walked.insert(pf.path.clone());
        }
    }

    for miss in chain_misses {
        let hits = locate_via_symbol_index(index, miss);
        if hits.is_empty() {
            continue;
        }
        stats.mapped += 1;
        for path in hits {
            if !seen_paths.insert(path.clone()) {
                continue;
            }
            let Some(language) = language_from_file_ext(&path) else {
                // Extension the indexer can't parse — skip.
                continue;
            };
            let virtual_path = virtual_path_for_indexed_file(&path, language);
            if already_walked.contains(&virtual_path) {
                continue;
            }
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
    // Apply the TS external post-parse hook so pulled externals get their
    // symbols prefixed with `<pkg>.` before the index sees them — keeping
    // qnames consistent for the same file across expand iterations.
    //
    // Parsing runs on the shared large-stack parse pool rather than the
    // default global pool: pulled externals are generated `.d.ts` files whose
    // CSTs nest deeper than app code, and the extractors walk them
    // recursively. The main pass already parses on this pool; external parsing
    // must match its stack budget or a deep union overflows the default stack.
    let parse_pool = crate::indexer::parse_file::build_parse_pool()?;
    let new_parsed: Vec<ParsedFile> = parse_pool.install(|| {
        new_walked
            .par_iter()
            .filter_map(|w| {
                // Whole-file pull: `None` demand extracts every symbol.
                let result = match type_arena {
                    Some(a) => parse_file_with_arena_and_demand(w, registry, None, a),
                    None => parse_file_with_demand(w, registry, None),
                };
                match result {
                    Ok(mut pf) => {
                        crate::ecosystem::npm::ts_post_process_external(&mut pf);
                        Some(pf)
                    }
                    Err(e) => {
                        debug!("expand: parse failed for {}: {e}", w.relative_path);
                        None
                    }
                }
            })
            .collect()
    });

    if new_parsed.is_empty() {
        return Ok(stats);
    }

    // Write with origin='external'. The write path upserts on path, so any
    // accidental duplicate of a pass-1 file is harmless.
    let mut new_parsed = new_parsed;
    let (_file_map, new_id_map) =
        write::write_parsed_files_with_origin(db, &new_parsed, "external", type_arena)
            .context("expand: failed to write expanded external symbols")?;
    stats.new_files = new_parsed.len();
    stats.new_symbols = new_id_map.len();
    symbol_id_map.extend(new_id_map);

    // Transitive `#include` closure for C/C++ headers. A pulled header
    // (`windows.h`) carries its own `#include` directives as `Imports` refs,
    // but external files are filtered from the resolve loop — so those refs
    // never re-drive the chain-miss demand. Follow them here, demand-time,
    // before the refs are about to be slimmed away: locate each included
    // header through the same path-keyed index, pull + parse it, and repeat
    // until no new header is reached, the per-pass cap is hit, or the
    // fan-out cap is hit. Bounded so a pathological include graph can't run
    // away. Non-C/C++ pulls contribute no header includes and skip this.
    let transitive = follow_header_includes(
        db,
        &new_parsed,
        index,
        registry,
        type_arena,
        &mut seen_paths,
        &mut already_walked,
    )?;
    stats.new_files += transitive.new_files;
    stats.new_symbols += transitive.new_symbols;
    symbol_id_map.extend(transitive.id_map);

    for pf in new_parsed.iter_mut() {
        pf.slim_for_resolve();
    }
    parsed.extend(new_parsed);
    for mut pf in transitive.parsed {
        pf.slim_for_resolve();
        parsed.push(pf);
    }

    info!(
        "Chain reachability expansion: {} misses → {} mapped → {} new files, {} new symbols",
        stats.misses, stats.mapped, stats.new_files, stats.new_symbols,
    );
    Ok(stats)
}

/// Maximum bounded passes over the transitive `#include` graph. Each pass
/// follows one hop of `#include` edges from the headers pulled by the
/// previous pass. Mirrors the npm transitive re-export cap.
const MAX_TRANSITIVE_PASSES: u32 = 5;

/// Hard ceiling on the total number of header files admitted across the whole
/// transitive closure for a single expand call. Mirrors the secondary-scan
/// `MAX_PULLED_FILES` precedent so a deeply-included SDK header (`windows.h`
/// fans out to hundreds of `um/` headers) can't admit an unbounded slice.
const MAX_PULLED_FILES: usize = 5000;

/// Outcome of the transitive `#include` walk: the parsed header files plus
/// the index growth they contributed.
#[derive(Default)]
struct TransitiveResult {
    parsed: Vec<ParsedFile>,
    id_map: HashMap<(String, String), i64>,
    new_files: usize,
    new_symbols: usize,
}

/// Follow C/C++ headers' own `#include` directives transitively, pulling each
/// reachable header through the path-keyed `SymbolLocationIndex`. Bounded by
/// [`MAX_TRANSITIVE_PASSES`] hops and [`MAX_PULLED_FILES`] total admissions.
///
/// `seen_paths` / `already_walked` are threaded in from the caller so a header
/// already pulled this expand call (by the symbol-miss wave or an earlier
/// transitive pass) is never pulled twice. Each pass parses the headers
/// reached by the previous pass and harvests their `#include` refs into the
/// next pass's frontier.
fn follow_header_includes(
    db: &mut Database,
    seed_parsed: &[ParsedFile],
    index: &SymbolLocationIndex,
    registry: &LanguageRegistry,
    type_arena: Option<&crate::type_checker::core::types::TypeArena>,
    seen_paths: &mut std::collections::HashSet<std::path::PathBuf>,
    already_walked: &mut std::collections::HashSet<String>,
) -> Result<TransitiveResult> {
    let mut result = TransitiveResult::default();

    // Frontier: header include-paths reached by the most recently parsed
    // wave but not yet pulled. Seeded from the symbol-miss wave's headers.
    let mut frontier = harvest_header_includes(seed_parsed);
    if frontier.is_empty() {
        return Ok(result);
    }

    let parse_pool = crate::indexer::parse_file::build_parse_pool()?;
    for _pass in 0..MAX_TRANSITIVE_PASSES {
        if frontier.is_empty() || result.new_files >= MAX_PULLED_FILES {
            break;
        }

        // Locate each frontier include-path through the path-keyed index and
        // build the next wave of WalkedFiles, deduped against everything
        // pulled so far this expand call.
        let mut wave_walked: Vec<WalkedFile> = Vec::new();
        for include_path in frontier.drain() {
            let Some(target) = index.locate(&include_path, &include_path) else {
                continue;
            };
            let path = target.to_path_buf();
            if !seen_paths.insert(path.clone()) {
                continue;
            }
            let Some(language) = language_from_file_ext(&path) else {
                continue;
            };
            let virtual_path = virtual_path_for_indexed_file(&path, language);
            if !already_walked.insert(virtual_path.clone()) {
                continue;
            }
            wave_walked.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
            if result.new_files + wave_walked.len() >= MAX_PULLED_FILES {
                break;
            }
        }
        if wave_walked.is_empty() {
            break;
        }

        let wave_parsed: Vec<ParsedFile> = parse_pool.install(|| {
            wave_walked
                .par_iter()
                .filter_map(|w| {
                    let parsed = match type_arena {
                        Some(a) => parse_file_with_arena_and_demand(w, registry, None, a),
                        None => parse_file_with_demand(w, registry, None),
                    };
                    match parsed {
                        Ok(pf) => Some(pf),
                        Err(e) => {
                            debug!("expand: transitive parse failed for {}: {e}", w.relative_path);
                            None
                        }
                    }
                })
                .collect()
        });
        if wave_parsed.is_empty() {
            break;
        }

        let (_file_map, wave_id_map) =
            write::write_parsed_files_with_origin(db, &wave_parsed, "external", type_arena)
                .context("expand: failed to write transitive header symbols")?;
        result.new_files += wave_parsed.len();
        result.new_symbols += wave_id_map.len();
        result.id_map.extend(wave_id_map);

        // The headers just parsed seed the next pass's frontier.
        frontier = harvest_header_includes(&wave_parsed);
        result.parsed.extend(wave_parsed);
    }

    if result.new_files > 0 {
        debug!(
            "expand: transitive #include closure pulled {} headers, {} symbols",
            result.new_files, result.new_symbols,
        );
    }
    Ok(result)
}

/// Collect the set of header include-paths referenced by C/C++ files' own
/// `#include` directives. The C extractor emits each `#include <x/y.h>` as an
/// `Imports` ref with `module = "x/y.h"`. Only header-shaped includes are
/// returned — project-relative includes the path-keyed index can't answer are
/// filtered out so the walk stays scoped to real SDK / vcpkg / POSIX headers.
fn harvest_header_includes(parsed: &[ParsedFile]) -> std::collections::HashSet<String> {
    use crate::types::EdgeKind;
    let mut out = std::collections::HashSet::new();
    for pf in parsed {
        if pf.language != "c" && pf.language != "cpp" {
            continue;
        }
        for r in &pf.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let Some(module) = r.module.as_deref().filter(|m| !m.is_empty()) else {
                continue;
            };
            if header_include_shape(module) {
                out.insert(module.to_string());
            }
        }
    }
    out
}

/// Whether an `#include` target names a header the path-keyed index could
/// hold: a header-extension path or an extensionless, separator-free name
/// (the C++ stdlib convention `<vector>` / `<memory>`).
fn header_include_shape(include_path: &str) -> bool {
    let basename = include_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(include_path);
    let is_header_ext = basename.ends_with(".h")
        || basename.ends_with(".hpp")
        || basename.ends_with(".hxx")
        || basename.ends_with(".hh");
    let is_extensionless_stdlib = !basename.contains('.') && !include_path.contains('/');
    is_header_ext || is_extensionless_stdlib
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
    // EXT-1 — module-scoped demand. An import-qualified external carries the
    // module its name was imported from, so locate the defining file *inside*
    // that module. No `find_by_name` fallback: a name absent under its own
    // module is a genuine gap (the demand index didn't reach it), not licence
    // to pull a coincidental same-name symbol from another package.
    if let Some(module) = &miss.module {
        return index
            .locate(module, &miss.target_name)
            .map(|p| vec![p.to_path_buf()])
            .unwrap_or_default();
    }

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
/// Must match `stage_link::virtual_path_for_pulled` — if a file pulled on
/// one expand iteration and re-pulled on the next gets a different
/// `ParsedFile::path`, it defeats the `already_walked` dedupe check and gets
/// parsed + written twice with mismatched qnames (post-processing hooks like
/// `prefix_ts_external_symbols` key off the `ext:ts:<pkg>/...` shape).
///
/// Falls back to `ext:idx:<absolute>` when the ecosystem-specific shape
/// isn't applicable (same as `make_walked_file` in `stage_link`).
fn virtual_path_for_indexed_file(path: &std::path::Path, language: &str) -> String {
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
