// =============================================================================
// engine/externals_demand.rs — demand-driven external materialization
//
// Pulls the externals the project reaches into the compilation tree: seeds from
// internal refs, follows the transitive type-dependency closure (refs, return-
// type heads, relative supertype imports, plugin-declared reachables), parses
// each pulled file, persists the symbols with origin='external', and ingests
// them so refs bind to real DB ids. Bounded by reachability — only files
// defining a referenced name are pulled, never whole dependency trees.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::Database;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::chain_walker::{
    parse_return_type_from_signature_for_lang, parse_type_head_and_args,
};
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::indexer::resolve::engine::relative_imports;
use crate::type_checker::core::types::TypeArena;
use crate::types::{EdgeKind, ParsedFile};
use crate::walker::WalkedFile;

pub(super) fn materialize_externals(
    db: &mut Database,
    tree: &mut Compilation,
    parsed: &[ParsedFile],
    loc: &SymbolLocationIndex,
    arena: &Arc<TypeArena>,
) -> Result<()> {
    if loc.is_empty() {
        return Ok(());
    }

    // Seed: the external files defining a name an INTERNAL ref reaches.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut frontier: Vec<PathBuf> = Vec::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        collect_external_files(&pf.refs, tree, loc, &mut seen, &mut frontier);
    }
    if frontier.is_empty() {
        return Ok(());
    }

    // Transitively pull the external type-dependency closure. A materialized
    // external file's own re-exports / imports / extends name the packages that
    // DECLARE the members its API exposes — a test runner re-exports its matcher
    // package whose assertion interface extends another package's, a query helper
    // returns a type aliased into a sibling package — so the member-declaring
    // interfaces stay absent until those are pulled too. Iterated to a fixpoint,
    // bounded: the Roslyn model where resolving a referenced library loads the
    // libraries it references in turn. The same `collect_external_files` seed
    // logic runs over each newly-parsed external file's refs.
    const MAX_CLOSURE_DEPTH: usize = 8;
    let mut ext_parsed: Vec<ParsedFile> = Vec::new();
    // TS module augmentations `(augmented_module, interface, augmenting_qname)`,
    // scanned from the on-disk source in the closure — the parse cache strips
    // `content`, so the disk path is the only reliable source here.
    let mut augmentations: Vec<(String, String, String)> = Vec::new();
    let mut to_parse = frontier;
    let mut depth = 0;
    while !to_parse.is_empty() && depth < MAX_CLOSURE_DEPTH {
        // Pair each pulled file with its on-disk path so the closure can also
        // follow the file's RELATIVE imports (resolved against this directory) —
        // a member-declaring sibling module the package's export map never named.
        let batch: Vec<(PathBuf, ParsedFile)> = to_parse
            .iter()
            .filter_map(|f| parse_external_file(f, arena).map(|pf| (f.clone(), pf)))
            .collect();
        let mut next: Vec<PathBuf> = Vec::new();
        for (abs, pf) in &batch {
            collect_external_files(&pf.refs, tree, loc, &mut seen, &mut next);
            collect_return_type_files(&pf.symbols, &pf.language, tree, loc, &mut seen, &mut next);
            relative_imports::collect_relative_supertype_imports(
                abs,
                &pf.refs,
                &mut seen,
                &mut next,
            );
            // Per-language extra reachability (e.g. Angular NgModule → component
            // .d.ts) — dispatched to the file's plugin so framework specifics stay
            // out of the generic resolve pipeline.
            if let Ok(content) = std::fs::read_to_string(abs) {
                let plugin = crate::languages::default_registry().get(&pf.language);
                if let Some(dir) = abs.parent() {
                    for spec in plugin.external_declaration_reachables(&pf.path, &content) {
                        if let Some(file) =
                            relative_imports::resolve_relative_ts_module(dir, &spec)
                        {
                            if seen.insert(file.clone()) {
                                next.push(file);
                            }
                        }
                    }
                }
            }
            super::module_augmentation::collect_module_augmentations(
                abs,
                &pf.path,
                &mut augmentations,
            );
        }
        ext_parsed.extend(batch.into_iter().map(|(_, pf)| pf));
        to_parse = next;
        depth += 1;
    }
    if ext_parsed.is_empty() {
        return Ok(());
    }

    // Sorted by virtual path before write/ingest: the closure above discovers
    // files in whatever order the frontier/BFS happened to enqueue them, which
    // is a function of which internal file's ref reached them first — not
    // necessarily stable when the same package is reachable through more than
    // one route (e.g. a package's root entry and one of its own subpaths both
    // independently declare a same-named symbol). `write_parsed_files_with_origin`
    // assigns each file's symbol ids in `ext_parsed`'s order, and `tree.ingest`'s
    // first-writer-wins `by_qname` insert keeps whichever file it sees first —
    // sorting here makes both a deterministic function of the file set, the
    // same fix already applied to the internal `parsed` batch in full.rs.
    ext_parsed.sort_by(|a, b| a.path.cmp(&b.path));

    // Persist the external symbols (origin='external') to get real DB ids, then
    // ingest them into the tree so refs bind to those ids.
    let (_files, ext_id_map) = crate::indexer::write::write_parsed_files_with_origin(
        db,
        &ext_parsed,
        "external",
        Some(arena),
    )
    .context("Failed to write external symbols")?;
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(&ext_parsed);
    tree.ingest(&ext_parsed, &ext_id_map, &ambient_qnames);

    // TS module augmentation: graft `declare module 'M' { interface I extends S }`
    // supertypes onto the interface module `M` exports as `I`, so a member
    // declared on `S` resolves on the receiver `M`-typed values carry.
    if !augmentations.is_empty() {
        tree.apply_module_augmentations(&augmentations);
    }

    // Cross-package re-export aliases: `{importing_module}.{name}` resolves to
    // the sibling package's declaration now that both sides are ingested. The
    // alias qname is assembled here; the target qname derives from the
    // declaring file's virtual-path package prefix — the same prefix its
    // materialized symbols carry.
    let aliases: Vec<(String, String, String)> = loc
        .reexport_aliases()
        .filter_map(|(module, name, target_file, target_name)| {
            let lang = language_from_file_ext(target_file)?;
            let vpath = virtual_path_for_indexed_file(target_file, lang);
            let pkg = crate::ecosystem::externals::ts_package_from_virtual_path(&vpath)?;
            Some((
                format!("{module}.{name}"),
                format!("{pkg}.{target_name}"),
                vpath,
            ))
        })
        .collect();
    if !aliases.is_empty() {
        tree.apply_external_reexport_aliases(&aliases);
    }
    Ok(())
}

/// Collect the external files that define a name reached by `refs`, into `out`
/// (deduped via `seen`). A module-tagged ref locates the file exporting the name
/// in that module; an untagged ref pulls only when no internal symbol claims the
/// name — an ambient global, or (in type position) any external definition the
/// `locate` seed missed. Shared by the internal seed pass and the transitive
/// closure passes over already-materialized external files, so a re-export chain
/// into a sibling package is followed the same way an internal import is.
fn collect_external_files(
    refs: &[crate::types::ExtractedRef],
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    for r in refs {
        match r.module.as_deref() {
            // Import-resolution tagged this ref with a module — locate the file
            // that exports the name in that module.
            Some(module) => {
                if let Some(file) = loc.locate(module, &r.target_name) {
                    let file = file.to_path_buf();
                    if seen.insert(file.clone()) {
                        out.push(file);
                    }
                }
                // Also pull the module's `.` entry. A barrel package (`vue`)
                // re-exports its names from other packages, so the name's def file
                // resolves under the DEFINING package; materializing the entry brings
                // in the `export *` chain — whose re-export refs carry the source
                // module, so a closure pass pulls each hop — and lets re-export
                // following bind the import against the entry.
                if let Some(entry) = loc.module_entry(module) {
                    let entry = entry.to_path_buf();
                    if seen.insert(entry.clone()) {
                        out.push(entry);
                    }
                }
            }
            // No module tag. Only pull when the name has no internal definition —
            // an internal symbol always wins over an external.
            None if tree.by_name(&r.target_name).is_empty() => {
                // Import-free global registered under the ambient-scope namespace
                // (`declare global`, test-runner global); bounded to global-
                // declaring packages, so a bare `expect()` materializes its
                // `.d.ts` while an ordinary bare call pulls nothing.
                if let Some(file) =
                    crate::ecosystem::ambient::locate_ambient_global(loc, &r.target_name)
                {
                    let file = file.to_path_buf();
                    if seen.insert(file.clone()) {
                        out.push(file);
                    }
                }
                // A type-position ref may still name an external type the `locate`
                // seed missed (e.g. re-exported through a barrel). Calls join
                // the pull set for names the demand index maps (a static
                // class's extension/utility methods are reached by METHOD
                // name — no ref ever names the declaring class); the
                // no-internal-definition gate above and `seen` keep the pull
                // set bounded.
                if matches!(
                    r.kind,
                    EdgeKind::Instantiates
                        | EdgeKind::TypeRef
                        | EdgeKind::Inherits
                        | EdgeKind::Implements
                        | EdgeKind::Calls
                ) {
                    for (_module, file) in loc.find_by_name(&r.target_name) {
                        let file = file.to_path_buf();
                        if seen.insert(file.clone()) {
                            out.push(file);
                        }
                    }
                }
            }
            None => {}
        }
    }
}

/// Pull the files that DEFINE a materialized external file's callables' RETURN
/// types. A method's return type is captured from its signature, not emitted as a
/// TypeRef edge, so `collect_external_files` (which follows refs) misses it — yet
/// `db.delete(t)` yields `PgDeleteBase`, whose member `.where(...)` the chain then
/// walks. This is the same next-hop reachability as following an import, sourced
/// from the signature: pull only an un-materialized, externally-defined head, so a
/// return type already indexed (internal or pulled) adds nothing. Signature shape
/// is per-language (TS/.NET `):`, Rust/Python `->`, Go's separator-less trailing
/// result) — dispatched through `parse_return_type_from_signature_for_lang`, the
/// same parser `populate_return_type_ids` uses for every language's own symbols.
fn collect_return_type_files(
    symbols: &[crate::types::ExtractedSymbol],
    lang: &str,
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    for s in symbols {
        let Some(sig) = s.signature.as_deref() else {
            continue;
        };
        let Some(ret) = parse_return_type_from_signature_for_lang(sig, lang) else {
            continue;
        };
        let (head, _args) = parse_type_head_and_args(&ret);
        // The lookup key is the bare declared name — TS/namespace paths join
        // segments with `.` (`ns.Type`), Rust/C++ paths with `::`
        // (`gadgetcrate::Gadget`) — while the location index keys locations
        // by leaf name.
        let leaf = head.rsplit("::").next().unwrap_or(head);
        let leaf = leaf.rsplit('.').next().unwrap_or(leaf);
        if leaf.is_empty() {
            continue;
        }
        // Guard on the identity the chain walker will look the yield up by.
        // For a QUALIFIED head the bare-leaf check is wrong in both
        // directions: a same-named type from an unrelated module satisfies
        // `by_name` and suppresses the pull of the one the signature names,
        // while the walk needs the qualified type and misses. An unqualified
        // head keeps the name check.
        let already_indexed = if leaf == head {
            !tree.by_name(leaf).is_empty()
        } else {
            tree.by_qualified_name(head).is_some()
        };
        if already_indexed {
            continue;
        }
        for (_module, file) in loc.find_by_name(leaf) {
            let file = file.to_path_buf();
            if seen.insert(file.clone()) {
                out.push(file);
            }
        }
    }
}

/// Parse one external source file into a `ParsedFile`, consulting the persistent
/// parse cache. Binary-format virtual paths (JAR / DLL) are skipped for now.
/// Mirrors the source path of the old materialize-on-miss driver, but the
/// resulting file is ingested into the new tree rather than the old store.
fn parse_external_file(file: &Path, arena: &Arc<TypeArena>) -> Option<ParsedFile> {
    let path_str = file.to_string_lossy();
    // A virtual demand-index entry (no file on disk) materializes through the
    // ecosystem that minted its scheme.
    if let Some(pf) = crate::ecosystem::externals::materialize_virtual_external(&path_str) {
        return Some(pf);
    }
    if path_str.starts_with("ext:jar:") || path_str.starts_with("ext:dotnet-type:") {
        return None;
    }

    let language = language_from_file_ext(file)?;
    let virtual_path = virtual_path_for_indexed_file(file, language);
    let bytes = std::fs::read(file).ok()?;
    let hash = crate::indexer::external_parse_cache::content_hash(&bytes);
    let size = bytes.len() as u64;

    if let Some(cached) =
        crate::indexer::external_parse_cache::get(file, &hash, &virtual_path, size, arena)
    {
        return Some(cached);
    }

    let walked = WalkedFile {
        relative_path: virtual_path,
        absolute_path: file.to_path_buf(),
        language,
    };
    let mut pf = crate::indexer::parse_file::parse_file_with_arena_and_demand(
        &walked,
        crate::languages::default_registry(),
        None,
        arena,
    )
    .ok()?;
    // External `.d.ts` symbols carry a `<pkg>.` prefix the resolver keys on; this
    // also prefixes the parse pass's `component_selectors` to match.
    crate::ecosystem::npm::ts_post_process_external(&mut pf, arena);
    crate::indexer::external_parse_cache::put(file, &hash, &pf, arena);
    Some(pf)
}

/// Language id for a pulled file via the registry's extension table.
fn language_from_file_ext(path: &Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    crate::languages::default_registry().language_by_extension(name)
}

/// Virtual path under which a pulled external file is indexed.
fn virtual_path_for_indexed_file(path: &Path, language: &str) -> String {
    crate::indexer::stage_link::virtual_path_for_pulled(path, language)
        .unwrap_or_else(|| format!("ext:idx:{}", path.to_string_lossy().replace('\\', "/")))
}

#[cfg(test)]
#[path = "externals_demand_tests.rs"]
mod tests;
