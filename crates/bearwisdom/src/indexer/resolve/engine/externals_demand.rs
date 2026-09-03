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
use crate::indexer::resolve::engine::demand_veto::{DemandVeto, FileLanguages};
use crate::indexer::resolve::engine::relative_imports;
use crate::indexer::resolve::engine::type_mention_demand;
use crate::indexer::write::SymbolIds;
use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ParsedFile};
use crate::walker::WalkedFile;
use rustc_hash::FxHashMap;

/// Grow `tree` with the externals the project's refs demand and return the
/// parsed external batch plus its DB id map, so a caller whose plugins carry
/// cross-file state derived from `parsed` (Elixir's `use`-injection map) can
/// refresh that state against files this pull surfaced — they never appear
/// in the eager `parsed` slice the caller built `tree` from.
pub(super) fn materialize_externals(
    db: &mut Database,
    tree: &mut Compilation,
    parsed: &[ParsedFile],
    loc: &SymbolLocationIndex,
    arena: &Arc<TypeArena>,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
) -> Result<(Vec<ParsedFile>, SymbolIds)> {
    if loc.is_empty() {
        return Ok((Vec::new(), SymbolIds::default()));
    }

    // Seed: the external files defining a name an INTERNAL ref reaches.
    let file_langs: FileLanguages<'_> = parsed
        .iter()
        .map(|pf| (pf.path.as_str(), pf.language.as_str()))
        .collect();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut frontier: Vec<PathBuf> = Vec::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        let veto = DemandVeto::new(&pf.language, profiles, &file_langs);
        collect_external_files(&pf.refs, &veto, tree, loc, &mut seen, &mut frontier);
        type_mention_demand::collect_chain_root_type_files(
            &pf.refs,
            tree,
            loc,
            &mut seen,
            &mut frontier,
        );
    }
    if frontier.is_empty() {
        return Ok((Vec::new(), SymbolIds::default()));
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
    // One spelling per file across the whole closure: candidates arrive both
    // root-relative (scan-produced) and `..`-relative (relative-import
    // resolution), and `seen` keys on the raw spelling — normalize at the
    // wave boundary and gate on registered root coverage so an escape from a
    // scoped root neither materializes nor mints a second virtual identity.
    let mut pulled: HashSet<PathBuf> = HashSet::new();
    let mut admit_wave = |wave: Vec<PathBuf>, pulled: &mut HashSet<PathBuf>| -> Vec<PathBuf> {
        let mut out = Vec::new();
        for f in wave {
            let norm = crate::ecosystem::symbol_index::normalize_lexically(&f);
            // Scheme-prefixed virtual entries (`ext:jar:`, `ext:dotnet-type:`)
            // are not filesystem paths; root coverage only confines real files.
            if (!norm.has_root() || loc.covers(&norm)) && pulled.insert(norm.clone()) {
                out.push(norm);
            }
        }
        out
    };
    let mut ext_parsed: Vec<ParsedFile> = Vec::new();
    // TS module augmentations `(augmented_module, interface, augmenting_qname)`,
    // scanned from the on-disk source in the closure — the parse cache strips
    // `content`, so the disk path is the only reliable source here.
    let mut augmentations: Vec<(String, String, String)> = Vec::new();
    let mut to_parse = admit_wave(frontier, &mut pulled);
    let mut depth = 0;
    while !to_parse.is_empty() && depth < MAX_CLOSURE_DEPTH {
        tracing::info!("demand closure wave {depth}: {} candidate files", to_parse.len());
        // Pair each pulled file with its on-disk path so the closure can also
        // follow the file's RELATIVE imports (resolved against this directory) —
        // a member-declaring sibling module the package's export map never named.
        let batch: Vec<(PathBuf, ParsedFile)> = to_parse
            .iter()
            .filter_map(|f| {
                let _t = crate::indexer::phase_timer::scope("demand.parse_external_file");
                parse_external_file(f, arena, loc, profiles).map(|pf| (f.clone(), pf))
            })
            .collect();
        let mut next: Vec<PathBuf> = Vec::new();
        for (abs, pf) in &batch {
            super::demand_reachability::collect(
                abs,
                pf,
                profiles,
                &file_langs,
                tree,
                loc,
                &mut seen,
                &mut next,
                &mut augmentations,
            );
        }
        ext_parsed.extend(batch.into_iter().map(|(_, pf)| pf));
        to_parse = admit_wave(next, &mut pulled);
        depth += 1;
    }
    if ext_parsed.is_empty() {
        return Ok((Vec::new(), SymbolIds::default()));
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
    let (_files, ext_id_map) = {
        let _t = crate::indexer::phase_timer::scope("demand.write_externals");
        crate::indexer::write::write_parsed_files_with_origin(db, &ext_parsed, "external", Some(arena))
            .context("Failed to write external symbols")?
    };
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(&ext_parsed);
    {
        let _t = crate::indexer::phase_timer::scope("demand.ext_ingest");
        tree.ingest(&ext_parsed, &ext_id_map, &ambient_qnames);
    }

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
    Ok((ext_parsed, ext_id_map))
}

/// Collect the external files that define a name reached by `refs`, into `out`
/// (deduped via `seen`). A module-tagged ref locates the file exporting the name
/// in that module; an untagged ref pulls unless `veto` finds an internal
/// definition the ref could bind — an ambient global, or (in type position) any
/// external definition the `locate` seed missed. Shared by the internal seed
/// pass and the transitive closure passes over already-materialized external
/// files, so a re-export chain into a sibling package is followed the same way
/// an internal import is; `veto` carries the collecting file's language either
/// way.
pub(super) fn collect_external_files(
    refs: &[crate::types::ExtractedRef],
    veto: &DemandVeto,
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
            // No module tag. Only pull when no INTERNAL definition of the name
            // is a binding candidate for this ref — same language, compatible
            // kind (see `DemandVeto`). External symbols already in the tree
            // never veto: one package's `Assert` method must not suppress
            // pulling another package's `Assert` class.
            None if !veto.vetoes(tree, r) => {
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
                // name — no ref ever names the declaring class); the veto gate
                // above and `seen` keep the pull set bounded.
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

/// Parse one external source file into a `ParsedFile`, consulting the persistent
/// parse cache, then reduce it to its declaration contract. External supply is
/// consumed declaration-first — body symbols and body refs are never readable
/// through the resolver's external surface, and dropping them here keeps the
/// demand frontier, the symbol writes, and the tree ingest proportional to the
/// API surface instead of the implementation.
fn parse_external_file(
    file: &Path,
    arena: &Arc<TypeArena>,
    loc: &SymbolLocationIndex,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
) -> Option<ParsedFile> {
    let mut pf = parse_external_file_full(file, arena, loc)?;
    // A language whose profile opts out keeps full fidelity — its bodies
    // define contract (macro-expansion codegen).
    let reduce = profiles
        .get(pf.language.as_str())
        .is_none_or(|p| p.external_contract_reduction);
    if !reduce {
        return Some(pf);
    }
    // Contract reduction trusts the symbol parent chain, and error-recovered
    // parses distort it — an include fragment's top-level functions come out
    // nested under earlier constructs and would be dropped as body detail.
    // Clean parses reduce fully; error-recovered ones keep every symbol and
    // reduce only their refs, whose kinds stay trustworthy.
    if pf.has_errors {
        crate::indexer::contract_filter::reduce_refs_to_contract(&mut pf);
    } else {
        crate::indexer::contract_filter::reduce_to_contract(&mut pf);
    }
    Some(pf)
}

/// The unreduced parse: cache consult, virtual materialization, or a fresh
/// tree-sitter parse. Binary-format virtual paths (JAR / DLL) are skipped.
fn parse_external_file_full(
    file: &Path,
    arena: &Arc<TypeArena>,
    loc: &SymbolLocationIndex,
) -> Option<ParsedFile> {
    let path_str = file.to_string_lossy();
    // A virtual demand-index entry (no file on disk) materializes through the
    // ecosystem that minted its scheme.
    if let Some(pf) = crate::ecosystem::externals::materialize_virtual_external(&path_str) {
        return Some(pf);
    }
    if path_str.starts_with("ext:jar:") || path_str.starts_with("ext:dotnet-type:") {
        return None;
    }

    // A single-language ecosystem's `SymbolLocationIndex::tag_language` hint
    // takes priority over the extension table — a file whose extension is
    // claimed by more than one language plugin (FPC `.pp` units vs Puppet
    // `.pp` manifests) still parses with the extractor its owning dep root
    // actually holds. Untagged files (no ecosystem claimed a hint, or the
    // owning ecosystem spans several languages) fall through to extension
    // dispatch as before.
    let language = loc.language_hint(file).or_else(|| language_from_file_ext(file))?;
    let virtual_path = virtual_path_for_indexed_file(file, language);
    let bytes = std::fs::read(file).ok()?;
    let hash = crate::indexer::external_parse_cache::content_hash(&bytes);
    let size = bytes.len() as u64;

    if let Some(mut cached) =
        crate::indexer::external_parse_cache::get(file, &hash, &virtual_path, size, arena)
    {
        // The cached payload is symbols/refs only (`content` is not part of
        // the content-addressed cache row) — a plugin's cross-file state pass
        // that CST-walks a file's source (Elixir's `use`/`__using__` harvest)
        // would see nothing for a warm-cache hit otherwise. `bytes` is already
        // on hand from the hash read above, so attach it at no extra I/O cost.
        cached.content = Some(String::from_utf8_lossy(&bytes).into_owned());
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

/// Language id for a pulled file via the registry's extension table. The
/// fallback dispatch when no ecosystem `tag_language` hint claimed the file.
fn language_from_file_ext(path: &Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    crate::languages::default_registry().language_by_extension(name)
}

/// Virtual path under which a pulled external file is indexed.
fn virtual_path_for_indexed_file(path: &Path, language: &str) -> String {
    crate::indexer::ext_virtual_path::virtual_path_for_pulled(path, language)
        .unwrap_or_else(|| format!("ext:idx:{}", path.to_string_lossy().replace('\\', "/")))
}

#[cfg(test)]
#[path = "externals_demand_tests.rs"]
mod tests;
