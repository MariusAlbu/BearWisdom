// =============================================================================
// engine/pipeline.rs — single-pass resolution entry for the new engine
//
// Builds a `Compilation` from parse output, materializes the externals the
// project reaches INTO that same tree, resolves every internal ref exactly once
// through `SemanticModel`, and bulk-writes edges + unresolved_refs to the DB. One
// pass, no fixpoint, no old-engine resolution code — externals are resolved like
// internals, the only difference being they are pulled from disk on first sight.
// =============================================================================

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

/// A resolved edge row: (source_id, target_id, kind, source_line, confidence, strategy).
type Edge = (i64, i64, &'static str, u32, f64, &'static str);
/// An unresolved-ref row: (source_id, target_name, kind, source_line, module, package_id, from_snippet).
type Unresolved = (i64, String, &'static str, u32, Option<String>, Option<i64>, bool);

use crate::db::Database;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry, RefContext, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::{semantic_model::SemanticModel, compilation::Compilation};
use crate::indexer::resolve::ResolutionStats;
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::type_checker::profile::language_profile::{ImportModulePath, LanguageProfile};
use crate::types::{AliasTarget, EdgeKind, ParsedFile};
use crate::walker::WalkedFile;

use crate::indexer::resolve::engine::contract::build_scope_chain;

// ---------------------------------------------------------------------------
// FileLookup — per-file SymbolLookup overlay with a forward-inference cache
// ---------------------------------------------------------------------------

/// A `SymbolLookup` that delegates every structural query to an underlying
/// `Compilation` and overlays a per-file forward-inference cache for local
/// variable types. One instance is constructed per internal file; the cache
/// is populated as the ref loop progresses so a later ref can read the type
/// inferred from an earlier binding (`const x = makeRepo(); x.find()`).
///
/// The cache is intentionally flat (no CFG, no narrowing) — this is forward
/// inference only: LHS-name → yield-type string. Narrowing remains in the
/// old engine path via `install_local_cache` / `set_cursor`.
struct FileLookup<'a> {
    tree: &'a Compilation,
    locals: RefCell<FxHashMap<String, String>>,
}

impl<'a> FileLookup<'a> {
    fn new(tree: &'a Compilation) -> Self {
        Self {
            tree,
            locals: RefCell::new(FxHashMap::default()),
        }
    }
}

impl<'a> SymbolLookup for FileLookup<'a> {
    // -- Structural delegation: 22 methods forwarded directly to the tree. ----

    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        self.tree.by_name(name)
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        self.tree.by_qualified_name(qname)
    }

    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        self.tree.all_by_qualified_name(qname)
    }

    fn members_of(&self, parent_qname: &str) -> SymbolSet<'_> {
        self.tree.members_of(parent_qname)
    }

    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        self.tree.types_by_name(name)
    }

    fn in_namespace(&self, namespace: &str) -> Vec<&Symbol> {
        self.tree.in_namespace(namespace)
    }

    fn has_in_namespace(&self, namespace: &str) -> bool {
        self.tree.has_in_namespace(namespace)
    }

    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        self.tree.in_file(file_path)
    }

    fn ambient_symbols(&self, name: &str) -> SymbolSet<'_> {
        self.tree.ambient_symbols(name)
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.tree.field_type_name(property_qname)
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.tree.return_type_name(method_qname)
    }

    fn field_type_args(&self, property_qname: &str) -> Option<&[String]> {
        self.tree.field_type_args(property_qname)
    }

    fn return_type_args(&self, method_qname: &str) -> Option<&[String]> {
        self.tree.return_type_args(method_qname)
    }

    fn generic_params(&self, type_name: &str) -> Option<&[String]> {
        self.tree.generic_params(type_name)
    }

    fn field_type_id(&self, property_qname: &str) -> Option<TypeId> {
        self.tree.field_type_id(property_qname)
    }

    fn return_type_id(&self, method_qname: &str) -> Option<TypeId> {
        self.tree.return_type_id(method_qname)
    }

    fn field_type_arg_ids(&self, property_qname: &str) -> Option<&[TypeId]> {
        self.tree.field_type_arg_ids(property_qname)
    }

    fn type_arena(&self) -> Option<&TypeArena> {
        self.tree.type_arena()
    }

    fn alias_target(&self, name: &str) -> Option<&AliasTarget> {
        self.tree.alias_target(name)
    }

    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.tree.reexports_from(file_path)
    }

    fn is_external_name(&self, name: &str, language: &str) -> bool {
        self.tree.is_external_name(name, language)
    }

    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.tree.parent_class_qname(class_qname)
    }

    fn enclosing_type_qname(&self, source_qname: &str) -> Option<&str> {
        self.tree.enclosing_type_qname(source_qname)
    }

    fn enclosing_namespace_qname(&self, source_qname: &str) -> Option<&str> {
        self.tree.enclosing_namespace_qname(source_qname)
    }

    // -- Flow cache: 5 methods implemented over `locals`. --------------------

    /// Return the inferred type of `name` from the per-file forward-inference
    /// cache. Returns `None` when the name has not been bound by an earlier ref.
    fn local_type(&self, name: &str) -> Option<String> {
        self.locals.borrow().get(name).cloned()
    }

    /// Single-branch wrapper over `local_type` for the union-aware chain walker.
    fn local_type_union(&self, name: &str) -> Option<Vec<String>> {
        self.local_type(name).map(|t| vec![t])
    }

    /// Bind `name` to `type_name` in the forward-inference cache.
    fn record_local_type(&self, name: String, type_name: String) {
        self.locals.borrow_mut().insert(name, type_name);
    }

    /// No-op: cursor-based narrowing is deferred; flat forward inference only.
    fn set_cursor(&self, _byte: u32) {}

    /// No-op: narrowing cache installation is deferred; flat forward inference only.
    fn install_local_cache(
        &self,
        _narrowings: Vec<crate::types::Narrowing>,
        _discriminants: Vec<crate::types::DiscriminantNarrowing>,
        _cfg: crate::indexer::flow_cfg::FileCfg,
    ) {
    }

    /// Evict all cached bindings so they cannot bleed into the next file's pass.
    fn clear_local_cache(&self) {
        self.locals.borrow_mut().clear();
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Single-pass resolution for the rule-based engine.
///
/// Builds a `Compilation` from `parsed`, resolves every ref in every internal
/// file once through `SemanticModel`, then bulk-writes the resulting edges and
/// unresolved_refs to the DB (replacing whatever was there before).
///
/// `project_ctx` supplies the generic project data the engine resolves against
/// (workspace-package names); the `Compilation` snapshots only those generic
/// fields, never language- or ecosystem-specific state.
pub fn resolve_single_pass(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    arena: Arc<TypeArena>,
    loc: Arc<SymbolLocationIndex>,
) -> Result<ResolutionStats> {
    // Classify ambient globals in the eager batch (internal files + the
    // eagerly-walked externals, including the synthetic TS lib module) so the
    // ambient-scope rung covers lib.*.d.ts / `@types` globals. Classification is
    // ecosystem knowledge; the engine only consults the resulting qname set.
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(parsed);
    let mut tree = Compilation::build_with_context(
        parsed,
        symbol_id_map,
        Arc::clone(&arena),
        project_ctx,
        &ambient_qnames,
    );

    // Grow the tree with the externals the project reaches. After this the tree
    // holds internal + external symbols and the resolve loop treats them alike.
    materialize_externals(db, &mut tree, parsed, &loc, &arena)
        .context("Failed to materialize external symbols")?;

    let profiles = build_profiles();
    let solver = SemanticModel::production();

    // Resolve every internal file in parallel. Files are independent units of
    // work; refs WITHIN a file stay ordered so forward flow inference (a local's
    // type recorded by an earlier ref is visible to a later ref) is
    // deterministic. The tree is read-only here, shared across workers by ref.
    let per_file: Vec<(Vec<Edge>, Vec<Unresolved>)> = parsed
        .par_iter()
        .filter(|pf| !pf.path.starts_with("ext:"))
        .map(|pf| resolve_one_file(pf, &tree, &profiles, &solver, symbol_id_map))
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    let mut unresolved: Vec<Unresolved> = Vec::new();
    for (e, u) in per_file {
        edges.extend(e);
        unresolved.extend(u);
    }

    let mut stats = ResolutionStats::default();
    stats.resolved = edges.len() as u64;
    stats.unresolved = unresolved.len() as u64;

    // Write: replace all three resolution tables atomically.
    flush_to_db(db, &edges, &unresolved)?;

    Ok(stats)
}

/// Resolve one internal file's refs, returning its edges and unresolved rows.
/// Refs are visited in source order so a later ref sees the flow-inferred type
/// of a local bound by an earlier ref. No shared mutable state — files run
/// concurrently over the read-only `tree`.
fn resolve_one_file(
    pf: &ParsedFile,
    tree: &Compilation,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    solver: &SemanticModel,
    symbol_id_map: &HashMap<(String, String), i64>,
) -> (Vec<Edge>, Vec<Unresolved>) {
    let mut edges: Vec<Edge> = Vec::new();
    let mut unresolved: Vec<Unresolved> = Vec::new();

    let Some(&profile) = profiles.get(pf.language.as_str()) else {
        return (edges, unresolved);
    };

    let file_ctx = build_file_context(&pf.language, pf, profile);

    // Fresh per-file flow cache: local bindings from earlier refs in this file
    // are visible to later refs in the same file only.
    let file_lookup = FileLookup::new(tree);

    for (ref_idx, r) in pf.refs.iter().enumerate() {
        let Some(source_sym) = pf.symbols.get(r.source_symbol_index) else {
            continue;
        };
        let Some(&source_id) =
            symbol_id_map.get(&(pf.path.clone(), source_sym.qualified_name.clone()))
        else {
            continue;
        };

        // Synthetic primitive-type marker emitted by the extractor — not a
        // resolvable symbol, so it is neither an edge nor an unresolved ref.
        if r.target_name == "_primitive" {
            continue;
        }

        let ref_ctx = RefContext {
            extracted_ref: r,
            source_symbol: source_sym,
            scope_chain: build_scope_chain(source_sym.scope_path.as_deref()),
            file_package_id: pf.package_id,
        };

        let kind_str = edge_kind_str(r.kind);

        match solver.get_symbol_info(&ref_ctx, &file_ctx, &file_lookup, profile) {
            Some(res) => {
                // Forward inference: when this ref is the RHS of a local binding,
                // record the yield type so a later ref rooted on the same variable
                // name can walk the chain.
                if let Some(&lhs_idx) = pf.flow.flow_binding_lhs.get(&ref_idx) {
                    let yield_ty = res
                        .resolved_yield_type
                        .and_then(|id| tree.type_arena().map(|a| a.format_type(id)))
                        .or_else(|| {
                            let target_id = res.target_symbol_id;
                            tree.by_name(&r.target_name)
                                .iter()
                                .find(|s| s.id == target_id)
                                .and_then(|s| {
                                    if r.kind == EdgeKind::Calls {
                                        tree.return_type_str(&s.qualified_name)
                                    } else {
                                        tree.field_type_str(&s.qualified_name)
                                    }
                                })
                        });
                    if let (Some(ty), Some(lhs_sym)) = (yield_ty, pf.symbols.get(lhs_idx)) {
                        file_lookup.record_local_type(lhs_sym.name.clone(), ty);
                    }
                }

                edges.push((
                    source_id,
                    res.target_symbol_id,
                    kind_str,
                    r.line,
                    res.confidence,
                    res.strategy,
                ));
            }
            None => {
                unresolved.push((
                    source_id,
                    r.target_name.clone(),
                    kind_str,
                    r.line,
                    r.module.clone(),
                    pf.package_id,
                    false,
                ));
            }
        }
    }

    (edges, unresolved)
}

// ---------------------------------------------------------------------------
// DB flush — inline because write_buf::{FileWriteBuf, flush_resolve_buf} are
// pub(super) relative to resolve/, which does not include resolve::engine.
// The logic is identical to flush_resolve_buf with persist_speculative=true.
// ---------------------------------------------------------------------------

fn flush_to_db(
    db: &mut Database,
    edges: &[(i64, i64, &'static str, u32, f64, &'static str)],
    unresolved: &[(i64, String, &'static str, u32, Option<String>, Option<i64>, bool)],
) -> Result<()> {
    use rusqlite::types::Value;

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin single-pass resolution transaction")?;

    tx.execute("DELETE FROM edges", [])
        .context("Failed to clear edges")?;
    tx.execute("DELETE FROM unresolved_refs", [])
        .context("Failed to clear unresolved_refs")?;
    tx.execute("DELETE FROM external_refs", [])
        .context("Failed to clear external_refs")?;

    const EDGE_CHUNK: usize = 256;
    const UNRESOLVED_CHUNK: usize = 256;

    fn placeholders(rows: usize, cols: usize) -> String {
        let mut s = String::with_capacity(rows * (cols * 2 + 4));
        for i in 0..rows {
            if i > 0 {
                s.push(',');
            }
            s.push('(');
            for j in 0..cols {
                if j > 0 {
                    s.push(',');
                }
                s.push('?');
            }
            s.push(')');
        }
        s
    }

    // Edges: (source_id, target_id, kind, source_line, confidence, strategy)
    if !edges.is_empty() {
        let mut start = 0;
        while start < edges.len() {
            let end = (start + EDGE_CHUNK).min(edges.len());
            let rows = end - start;
            let sql = format!(
                "INSERT OR IGNORE INTO edges \
                 (source_id, target_id, kind, source_line, confidence, strategy) \
                 VALUES {}",
                placeholders(rows, 6),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 6);
            for (sid, tid, kind, line, conf, strat) in &edges[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Integer(*tid));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(Value::Real(*conf));
                params.push(Value::Text((*strat).to_string()));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare edges insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute edges insert")?;
            start = end;
        }
    }

    // Unresolved refs: (source_id, target_name, kind, source_line, module, package_id, from_snippet)
    if !unresolved.is_empty() {
        let mut start = 0;
        while start < unresolved.len() {
            let end = (start + UNRESOLVED_CHUNK).min(unresolved.len());
            let rows = end - start;
            let sql = format!(
                "INSERT INTO unresolved_refs \
                 (source_id, target_name, kind, source_line, module, package_id, from_snippet) \
                 VALUES {}",
                placeholders(rows, 7),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 7);
            for (sid, name, kind, line, module, pkg, from_snippet) in &unresolved[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Text(name.clone()));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(match module {
                    Some(s) => Value::Text(s.clone()),
                    None => Value::Null,
                });
                params.push(match pkg {
                    Some(v) => Value::Integer(*v),
                    None => Value::Null,
                });
                params.push(Value::Integer(if *from_snippet { 1 } else { 0 }));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare unresolved_refs insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute unresolved_refs insert")?;
            start = end;
        }
    }

    tx.commit()
        .context("Failed to commit single-pass resolution transaction")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// EdgeKind → &'static str
// ---------------------------------------------------------------------------

/// Convert an `EdgeKind` to the snake_case `&'static str` used in the DB.
///
/// `EdgeKind` derives `strum::IntoStaticStr` with `serialize_all = "snake_case"`,
/// so the conversion is a zero-cost static dispatch into the strum vtable.
#[inline]
fn edge_kind_str(kind: EdgeKind) -> &'static str {
    kind.into()
}

// ---------------------------------------------------------------------------
// Profile map
// ---------------------------------------------------------------------------

/// Build a language-id → LanguageProfile map from the default plugin registry.
///
/// Registers each profile under every language id the plugin claims, matching
/// the same multi-id pattern `Engine::build_from_registry` uses.
fn build_profiles() -> FxHashMap<&'static str, &'static LanguageProfile> {
    let mut profiles: FxHashMap<&'static str, &'static LanguageProfile> = FxHashMap::default();
    for plugin in crate::languages::default_registry().all() {
        if let Some(profile) = plugin.profile() {
            for &lang in plugin.language_ids() {
                profiles.insert(lang, profile);
            }
        }
    }
    profiles
}

// ---------------------------------------------------------------------------
// FileContext builder
// ---------------------------------------------------------------------------

/// Build a `FileContext` from profile data alone, without invoking the Engine.
///
/// Replicates the logic from `engine.rs::generic_file_context`:
/// - `FromModuleField` — any ref with a `module` field becomes an import entry.
/// - Other modes — only `EdgeKind::Imports` refs; `module_path` is either empty
///   (`None` mode) or echoes the target name (`EchoTarget` mode).
fn build_file_context(language: &str, file: &ParsedFile, profile: &LanguageProfile) -> FileContext {
    let imports: Vec<ImportEntry> = match profile.import_module_path {
        ImportModulePath::FromModuleField => file
            .refs
            .iter()
            .filter_map(|r| {
                let module = r.module.clone()?;
                Some(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(module),
                    alias: None,
                    is_wildcard: r.target_name == "*",
                })
            })
            .collect(),
        mode => file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: match mode {
                    ImportModulePath::None => None,
                    ImportModulePath::EchoTarget => Some(r.target_name.clone()),
                    ImportModulePath::FromModuleField => unreachable!(),
                },
                alias: None,
                is_wildcard: r.target_name == "*",
            })
            .collect(),
    };
    FileContext {
        file_path: file.path.clone(),
        language: language.to_string(),
        imports,
        file_namespace: None,
    }
}

// ---------------------------------------------------------------------------
// External materialization — pull the externals the project reaches into the
// tree, parse them, write their symbols to the DB, and ingest them. Demand is
// driven by the project's own refs: a ref whose import-resolution tagged a
// `module` names an external symbol; the location index says which file defines
// it. Bounded by reachability — only files defining a referenced name are
// pulled, never whole `node_modules`.
// ---------------------------------------------------------------------------

fn materialize_externals(
    db: &mut Database,
    tree: &mut Compilation,
    parsed: &[ParsedFile],
    loc: &SymbolLocationIndex,
    arena: &Arc<TypeArena>,
) -> Result<()> {
    if loc.is_empty() {
        return Ok(());
    }

    // Collect the distinct external files defining a referenced external name.
    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        for r in &pf.refs {
            match r.module.as_deref() {
                // Import-resolution tagged this ref with a module — locate the
                // file that exports the name in that module.
                Some(module) => {
                    if let Some(file) = loc.locate(module, &r.target_name) {
                        let file = file.to_path_buf();
                        if seen.insert(file.clone()) {
                            files.push(file);
                        }
                    }
                }
                // No module tag. Only pull when the name has no internal
                // definition — an internal symbol always wins over an external.
                None if tree.by_name(&r.target_name).is_empty() => {
                    // Import-free global: a package registered this name under the
                    // ambient-scope namespace (`declare global`, test-runner
                    // global). Demand-driven — pulled only because the project
                    // references the name, and bounded to global-declaring
                    // packages, so a bare `expect()` materializes its `.d.ts`
                    // while an ordinary bare call pulls nothing.
                    if let Some(file) =
                        crate::ecosystem::ambient::locate_ambient_global(loc, &r.target_name)
                    {
                        let file = file.to_path_buf();
                        if seen.insert(file.clone()) {
                            files.push(file);
                        }
                    }
                    // A type-position ref may still name an external type the
                    // `locate` seed missed (e.g. re-exported through a barrel).
                    // Bounded to type-position kinds so a bare method call doesn't
                    // pull a same-named external function.
                    if matches!(
                        r.kind,
                        EdgeKind::Instantiates
                            | EdgeKind::TypeRef
                            | EdgeKind::Inherits
                            | EdgeKind::Implements
                    ) {
                        for (_module, file) in loc.find_by_name(&r.target_name) {
                            let file = file.to_path_buf();
                            if seen.insert(file.clone()) {
                                files.push(file);
                            }
                        }
                    }
                }
                None => {}
            }
        }
    }
    if files.is_empty() {
        return Ok(());
    }

    // Parse each external file (cached) into a ParsedFile.
    let ext_parsed: Vec<ParsedFile> = files
        .iter()
        .filter_map(|f| parse_external_file(f, arena))
        .collect();
    if ext_parsed.is_empty() {
        return Ok(());
    }

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
    Ok(())
}

/// Parse one external source file into a `ParsedFile`, consulting the persistent
/// parse cache. Binary-format virtual paths (JAR / DLL) are skipped for now.
/// Mirrors the source path of the old materialize-on-miss driver, but the
/// resulting file is ingested into the new tree rather than the old store.
fn parse_external_file(file: &Path, arena: &Arc<TypeArena>) -> Option<ParsedFile> {
    let path_str = file.to_string_lossy();
    if path_str.starts_with("ext:jar:") || path_str.starts_with("ext:dotnet-type:") {
        return None;
    }

    let language = language_from_file_ext(file)?;
    let virtual_path = virtual_path_for_indexed_file(file, language);
    let bytes = std::fs::read(file).ok()?;
    let hash = crate::indexer::external_parse_cache::content_hash(&bytes);
    let size = bytes.len() as u64;

    if let Some(cached) = crate::indexer::external_parse_cache::get(file, &hash, &virtual_path, size)
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
    // External `.d.ts` symbols carry a `<pkg>.` prefix the resolver keys on.
    crate::ecosystem::npm::ts_post_process_external(&mut pf);
    crate::indexer::external_parse_cache::put(file, &hash, &pf);
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

// ---------------------------------------------------------------------------
// Smoke test
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
