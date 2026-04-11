// =============================================================================
// indexer/resolve.rs  —  cross-file reference resolution
//
// The resolver turns "unresolved references" (a name + kind from one symbol)
// into "edges" (source_id → target_id in the graph).
//
// 4-priority lookup
// -----------------
// Each reference gets resolved by trying four strategies in order.  The first
// match wins; ties within a priority are broken by picking the first result.
//
//  Priority 1 — Import match (confidence 0.95)
//    The reference appears in a file that has `import { TargetName } from "path"`.
//    The referenced name matches the imported name.  We then search the
//    symbol_id_map for that name in files whose relative path ends with
//    the module path.
//
//  Priority 2 — Qualified name (confidence 0.90)
//    The reference target_name contains dots (e.g. "Catalog.CatalogService.List")
//    and matches a symbol's qualified_name exactly.
//
//  Priority 3 — Namespace/module match (confidence 0.80)
//    The source file has `using X` (C#) or `import ... from "./Y"` (TS)
//    and there exists a symbol with name = target_name in a file under
//    namespace/module X.
//
//  Priority 4 — Name + kind (confidence 0.50)
//    Bare name match.  Disambiguation: if the reference kind is Calls, we
//    prefer symbols with kind Method, Function, or Constructor.  If it is
//    Inherits, we prefer Class or Struct.  If Implements, we prefer Interface.
//
// If no strategy resolves the reference, it is written to `unresolved_refs`.
//
// Implementation notes
// --------------------
// All lookups are done against the in-memory symbol_id_map (built during
// write_to_db) to avoid expensive SQL queries per reference.  We do a single
// SQL write per resolved edge.
// =============================================================================

use crate::db::Database;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};
use anyhow::{Context, Result};
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use tracing::debug;

/// Attempt to resolve all references across all parsed files, then write
/// the resolved edges and unresolved refs to the database.
///
/// Returns (resolved_count, unresolved_count).
pub fn resolve_and_write(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
) -> Result<(u64, u64)> {
    // Build auxiliary lookup structures.
    // name_to_ids: simple_name → [(file, qname, kind, id)]
    let name_to_ids = build_name_index(symbol_id_map, parsed);
    let qname_to_id = build_qname_index(symbol_id_map);
    let module_to_files = build_module_to_files(parsed);

    // Build a per-file import map: file_path → Vec<(imported_name, module_path)>
    let import_map = build_import_map(parsed);

    // Build a per-file namespace map for same-namespace resolution.
    let file_namespace_map = build_file_namespace_map(parsed);

    let conn = db.conn();
    let tx = conn.unchecked_transaction().context("Failed to begin resolution transaction")?;

    let mut resolved = 0u64;
    let mut unresolved = 0u64;

    for pf in parsed {
        // Get the DB symbol IDs for this file's symbols.
        // We need to map the in-file index (source_symbol_index in ExtractedRef)
        // to the DB symbol ID.
        let file_symbol_ids: Vec<Option<i64>> = pf.symbols
            .iter()
            .map(|sym| {
                symbol_id_map
                    .get(&(pf.path.clone(), sym.qualified_name.clone()))
                    .copied()
            })
            .collect();

        // Get the imports for this file.
        let empty_vec = vec![];
        let file_imports = import_map.get(&pf.path).unwrap_or(&empty_vec);

        for r in &pf.refs {
            // Find the source symbol DB ID.
            let source_id = match file_symbol_ids.get(r.source_symbol_index).and_then(|id| *id) {
                Some(id) => id,
                None => {
                    debug!(
                        "Skipping ref to '{}': source symbol index {} has no DB ID in {}",
                        r.target_name, r.source_symbol_index, pf.path
                    );
                    continue;
                }
            };

            // Try each resolution priority.
            let source_namespace = file_namespace_map.get(&pf.path).map(|s| s.as_str());
            let chain_prefix = r.chain.as_ref().and_then(|c| {
                if c.segments.len() >= 2 {
                    Some(c.segments[c.segments.len() - 2].name.as_str())
                } else {
                    None
                }
            });
            let ref_module = r.module.as_deref();
            let resolution = resolve_ref(
                r.target_name.as_str(),
                r.kind,
                &pf.path,
                file_imports,
                source_namespace,
                chain_prefix,
                ref_module,
                &name_to_ids,
                &qname_to_id,
                &module_to_files,
                symbol_id_map,
                parsed,
            );

            match resolution {
                Some((target_id, confidence)) => {
                    // Write the edge (ignore duplicate constraint violations).
                    let result = tx.execute(
                        "INSERT OR IGNORE INTO edges
                           (source_id, target_id, kind, source_line, confidence)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![
                            source_id,
                            target_id,
                            r.kind.as_str(),
                            r.line,
                            confidence,
                        ],
                    );
                    match result {
                        Ok(_) => resolved += 1,
                        Err(e) => debug!("Edge insert failed: {e}"),
                    }
                }
                None => {
                    // Store for diagnostics.
                    tx.execute(
                        "INSERT INTO unresolved_refs
                           (source_id, target_name, kind, source_line, module)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![
                            source_id,
                            r.target_name,
                            r.kind.as_str(),
                            r.line,
                            r.module,
                        ],
                    ).ok(); // best-effort — don't fail the whole pass
                    unresolved += 1;
                }
            }
        }
    }

    tx.commit().context("Failed to commit resolution transaction")?;
    Ok((resolved, unresolved))
}

// ---------------------------------------------------------------------------
// Resolution logic
// ---------------------------------------------------------------------------

/// Attempt to resolve a reference using the 4-priority strategy.
///
/// Returns `Some((target_id, confidence))` on success, `None` if unresolvable.
pub(super) fn resolve_ref(
    target_name: &str,
    kind: EdgeKind,
    source_file: &str,
    file_imports: &[(String, Option<String>)],
    source_namespace: Option<&str>,
    chain_prefix: Option<&str>,
    ref_module: Option<&str>,
    name_to_ids: &FxHashMap<String, Vec<(String, String, String, i64)>>, // name → [(file, qname, kind, id)]
    qname_to_id: &FxHashMap<String, i64>,
    module_to_files: &FxHashMap<String, Vec<String>>,
    symbol_id_map: &HashMap<(String, String), i64>,
    parsed: &[ParsedFile],
) -> Option<(i64, f64)> {
    // --- Priority 0: Direct module match (0.95) ---
    // The extractor set `module` on this ref (e.g., Erlang `lists:map()`,
    // OCaml `List.map`, R `dplyr::mutate`).
    // Use the module-to-files index for precise matching.
    if let Some(module) = ref_module {
        if let Some(id) =
            resolve_via_ref_module(target_name, module, name_to_ids, module_to_files, qname_to_id)
        {
            return Some((id, 0.95));
        }
    }

    // --- Priority 0.5: Chain-prefix import match (0.95) ---
    // When the ref has a chain like `Foo::bar()` or `resolve::resolve_and_write()`,
    // the chain prefix ("Foo" or "resolve") may match an import.  Use the import's
    // module path to narrow candidates: only consider symbols in files whose path
    // matches the imported module.
    //
    // This is the analogue of C# `using Namespace; Namespace.Type.Method()` — the
    // qualified chain tells us which module the target lives in.
    if let Some(prefix) = chain_prefix {
        if let Some(id) = resolve_via_chain_prefix(
            target_name, prefix, file_imports, name_to_ids, parsed,
        ) {
            return Some((id, 0.95));
        }
    }

    // --- Priority 1: Import match (0.95) ---
    if let Some(id) = resolve_via_import(target_name, source_file, file_imports, name_to_ids, symbol_id_map, parsed) {
        return Some((id, 0.95));
    }

    // --- Priority 1.5: Namespace import match (0.92) ---
    // C# `using eShop.Catalog.API.Model` puts the namespace in the import list,
    // not the individual type names.  Look for a symbol whose qualified_name is
    // "{namespace}.{target_name}" for any dotted import in scope.
    if let Some(id) = resolve_via_namespace_import(target_name, file_imports, qname_to_id) {
        return Some((id, 0.92));
    }

    // --- Priority 2: Qualified name (0.90) ---
    if target_name.contains('.') {
        if let Some(&id) = qname_to_id.get(target_name) {
            return Some((id, 0.90));
        }
    }

    // --- Priority 2.5: Same-namespace match (0.85) ---
    // In C#, types in the same namespace are visible without a `using` directive.
    // e.g., Transaction.cs and Category.cs are both in FamilyBudget.Api.Entities —
    // Transaction can use Category without `using FamilyBudget.Api.Entities`.
    if let Some(ns) = source_namespace {
        if let Some(id) = resolve_via_same_namespace(target_name, ns, name_to_ids) {
            return Some((id, 0.85));
        }
    }

    // --- Priority 3: Namespace match (0.80) ---
    if let Some(id) = resolve_via_namespace(target_name, source_file, file_imports, name_to_ids, parsed) {
        return Some((id, 0.80));
    }

    // --- Priority 4: Name + kind (0.50 base, with ambiguity decay) ---
    // Names like `get`, `create`, `update` appear in hundreds of symbols.
    // Resolving them produces false edges that poison graph quality.
    // Hard cap: skip resolution entirely when > 10 kind-compatible candidates.
    // Soft decay: confidence scales as 0.50 / sqrt(candidate_count) so that
    // edge weights reflect how ambiguous the resolution is.
    if let Some(candidates) = name_to_ids.get(target_name) {
        // Filter to kind-compatible candidates first; fall back to all if none match.
        let kind_matched: Vec<_> = candidates
            .iter()
            .filter(|(_, _, sym_kind, _)| kind_matches_symbol_kind(kind, sym_kind))
            .collect();
        let pool: Vec<_> = if kind_matched.is_empty() {
            candidates.iter().collect()
        } else {
            kind_matched
        };

        // Ambiguity threshold: > 10 matching candidates -> too noisy, skip.
        const AMBIGUITY_LIMIT: usize = 10;
        if pool.len() > AMBIGUITY_LIMIT {
            return None;
        }

        if !pool.is_empty() {
            // Prefer same-file, then same-directory, then first candidate.
            let best = pool
                .iter()
                .min_by_key(|(file, _, _, _)| {
                    if *file == source_file { 0 }
                    else if parent_dir(file) == parent_dir(source_file) { 1 }
                    else { 2 }
                })
                .unwrap();
            // Confidence decays with candidate count: 0.50 / sqrt(n).
            let confidence = 0.50 / (pool.len() as f64).sqrt();
            return Some((best.3, confidence));
        }
    }

    None
}

/// Priority 0: Direct module match.
///
/// The extractor set `module` on this ref — e.g. Erlang `lists:map()` has
/// module="lists", OCaml `List.map` has module="List", R `dplyr::mutate`
/// has module="dplyr".
///
/// Resolution strategy (from most to least precise):
///   1. Module name → namespace symbol → find target in that file
///   2. Module name → file stem match → find target in that file
///   3. Qualified name lookup: "{module}.{target}" in qname index
///   4. Fallback: file path contains module (loose, legacy)
fn resolve_via_ref_module(
    target_name: &str,
    module: &str,
    name_to_ids: &FxHashMap<String, Vec<(String, String, String, i64)>>,
    module_to_files: &FxHashMap<String, Vec<String>>,
    qname_to_id: &FxHashMap<String, i64>,
) -> Option<i64> {
    let candidates = name_to_ids.get(target_name)?;

    // --- Strategy 1+2: Module name → files via namespace symbols + file stems ---
    // Look up the module name directly (e.g., "lists" → ["src/lists.erl"])
    // and case-insensitively (e.g., "List" → ["lib/list.ml"]).
    let module_files = module_to_files.get(module);
    let module_lower = module.to_lowercase();
    let module_files_ci = if module_files.is_none() {
        module_to_files.get(&module_lower)
    } else {
        None
    };

    // For dotted modules like "Data.Map", also try the last segment
    let last_segment = module.rsplit('.').next().unwrap_or(module);
    let last_lower = last_segment.to_lowercase();
    let last_segment_files = if last_segment != module {
        module_to_files
            .get(last_segment)
            .or_else(|| module_to_files.get(&last_lower))
    } else {
        None
    };

    // Collect all file paths associated with this module name
    let all_module_files: Vec<&str> = module_files
        .into_iter()
        .chain(module_files_ci)
        .chain(last_segment_files)
        .flat_map(|v| v.iter().map(|s| s.as_str()))
        .collect();

    if !all_module_files.is_empty() {
        // Find candidates in the module's files
        for (file, _qname, _kind, id) in candidates {
            if all_module_files.contains(&file.as_str()) {
                return Some(*id);
            }
        }
    }

    // --- Strategy 3: Qualified name lookup ---
    // Try "{module}.{target}" as a qualified name (handles "List.map", "SysUtils.FreeAndNil")
    let qname_dotted = format!("{module}.{target_name}");
    if let Some(&id) = qname_to_id.get(&qname_dotted) {
        return Some(id);
    }
    // Also try with :: separator (Rust, R)
    let qname_colon = format!("{module}::{target_name}");
    if let Some(&id) = qname_to_id.get(&qname_colon) {
        return Some(id);
    }
    // Clojure: "ns/fn" qualified names
    let qname_slash = format!("{module}/{target_name}");
    if let Some(&id) = qname_to_id.get(&qname_slash) {
        return Some(id);
    }

    // --- Strategy 4: File path contains module (legacy fallback) ---
    let normalized = module
        .replace("::", "/")
        .replace('.', "/")
        .trim_start_matches("crate/")
        .trim_start_matches("./")
        .trim_start_matches("../")
        .to_string();

    let norm_last = normalized.rsplit('/').next().unwrap_or(&normalized);

    // Score candidates: qname match > file match
    let mut best: Option<(i64, bool)> = None;
    for (file, qname, _kind, id) in candidates {
        let qname_match = qname.ends_with(&format!("{norm_last}.{target_name}"))
            || qname.ends_with(&format!(
                "{}.{}",
                normalized.replace('/', "."),
                target_name
            ));
        if qname_match {
            return Some(*id);
        }

        let norm_file = file.replace('\\', "/");
        let file_match =
            norm_file.contains(&normalized) || norm_file.contains(norm_last);
        if file_match && best.map_or(true, |(_, was_file)| !was_file) {
            best = Some((*id, true));
        }
    }

    best.map(|(id, _)| id)
}

/// Priority 0.5: chain-prefix import match.
///
/// When the call has a chain like `resolve::resolve_and_write()`, the prefix
/// `resolve` may match an import (`use crate::indexer::resolve`).  The import's
/// module path tells us which module the target lives in, and we narrow
/// candidates to files that match that module path.
///
/// Works across languages:
///   - Rust:   `use crate::indexer::resolve;` + `resolve::resolve_and_write()`
///   - Python: `import models` + `models.User()`
///   - TS/JS:  `import { api } from './services'` + `api.fetchUser()`
///   - C#:     `using System.Linq;` + `Enumerable.Range()`
fn resolve_via_chain_prefix(
    target_name: &str,
    prefix: &str,
    file_imports: &[(String, Option<String>)],
    name_to_ids: &FxHashMap<String, Vec<(String, String, String, i64)>>,
    parsed: &[ParsedFile],
) -> Option<i64> {
    // Find an import whose name matches the chain prefix.
    let matching_import = file_imports
        .iter()
        .find(|(imported_name, _)| imported_name == prefix)?;

    let module_path = matching_import.1.as_deref().unwrap_or("");

    // Look for target_name among candidates, preferring those in files that
    // match the imported module path.
    let candidates = name_to_ids.get(target_name)?;

    // First pass: strict file-path match against the module.
    for (file_path, _qname, _sym_kind, id) in candidates {
        if file_path_matches_module(file_path, module_path) {
            return Some(*id);
        }
    }

    // Second pass: the prefix itself may be a directory name (Rust modules map
    // to directories).  Check if any candidate file is inside a directory whose
    // name matches the prefix.  e.g., prefix="resolve" matches files in
    // ".../resolve/mod.rs" or ".../resolve/heuristic.rs".
    // Prefer module entry points (mod.rs, __init__.py, index.ts).
    let mut dir_matches: Vec<&(String, String, String, i64)> = candidates
        .iter()
        .filter(|(file_path, _, _, _)| {
            // Check if any path segment matches the prefix.
            file_path.split('/').any(|seg| seg == prefix)
        })
        .collect();

    if !dir_matches.is_empty() {
        // Prefer module entry points.
        dir_matches.sort_by_key(|(file_path, _, _, _)| {
            if is_module_entry_point(file_path) { 0 } else { 1 }
        });
        return Some(dir_matches[0].3);
    }

    // Third pass: the module path itself may contain the prefix as a Rust-style
    // path.  e.g., module "crate::indexer::resolve" → look for files under
    // "indexer/resolve/".
    if !module_path.is_empty() {
        let module_dir = module_path
            .replace("::", "/")
            .replace('.', "/");
        for (file_path, _qname, _kind, id) in candidates {
            // Check if the file contains the module directory segments.
            let normalized = file_path.replace('\\', "/");
            if normalized.contains(&module_dir) {
                return Some(*id);
            }
        }
    }

    // For Rust specifically: if the prefix matches a directory name but the
    // import has no explicit module path (e.g., `use crate::indexer::resolve;`
    // where the extractor stored "resolve" as the import name with module
    // "crate::indexer"), try matching via the ParsedFile paths.
    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        let norm = pf.path.replace('\\', "/");
        // Check if this file is the module entry for the prefix
        if norm.ends_with(&format!("{prefix}/mod.rs"))
            || norm.ends_with(&format!("{prefix}/__init__.py"))
            || norm.ends_with(&format!("{prefix}/index.ts"))
            || norm.ends_with(&format!("{prefix}/index.tsx"))
        {
            // Found the module file — now look for target in siblings
            let module_dir = parent_dir(&norm);
            for (file_path, _qname, _kind, id) in candidates {
                let cf = file_path.replace('\\', "/");
                if cf.starts_with(module_dir) {
                    return Some(*id);
                }
            }
        }
    }

    None
}

/// Priority 1: check if `target_name` is in the file's import list,
/// then find its definition in the imported module.
fn resolve_via_import(
    target_name: &str,
    _source_file: &str,
    file_imports: &[(String, Option<String>)],
    name_to_ids: &FxHashMap<String, Vec<(String, String, String, i64)>>,
    symbol_id_map: &HashMap<(String, String), i64>,
    parsed: &[ParsedFile],
) -> Option<i64> {
    // Find an import that brought `target_name` into scope.
    let matching_import = file_imports
        .iter()
        .find(|(imported_name, _)| imported_name == target_name)?;

    let module_path = matching_import.1.as_deref().unwrap_or("");

    if let Some(candidates) = name_to_ids.get(target_name) {
        for (file_path, qname, _sym_kind, id) in candidates {
            // C# case: module_path is "System.Linq" → file in namespace contains "Linq".
            // TS case: module_path is "./catalog" → file path ends with "catalog.ts".
            if file_path_matches_module(file_path, module_path) {
                let _ = (qname, symbol_id_map, parsed); // used in future enrichment
                return Some(*id);
            }
        }
        // If no path-based match, return the first candidate anyway (weak match).
        if !candidates.is_empty() && !module_path.is_empty() {
            // Only return if there's exactly one candidate with that name
            // (unambiguous even without path check).
            if candidates.len() == 1 {
                return Some(candidates[0].3);
            }
        }
    }
    None
}

/// Priority 1.5: C# namespace-level import match (confidence 0.92).
///
/// C# `using eShop.Catalog.API.Model;` puts the full namespace in the import
/// list, not individual type names.  This function checks: for each dotted
/// import, does `{namespace}.{target_name}` exist as a qualified_name?
///
/// Example: import "eShop.Catalog.API.Model" + target "CatalogItem"
///          → look up "eShop.Catalog.API.Model.CatalogItem" in qname_to_id.
///
/// We check both `imported_name` (first tuple element) and `module_opt`
/// (second element) as the namespace prefix, preferring `module_opt` when
/// it is present and dotted.  This covers cases where the extractor stores
/// the namespace in one field but not the other.
fn resolve_via_namespace_import(
    target_name: &str,
    file_imports: &[(String, Option<String>)],
    qname_to_id: &FxHashMap<String, i64>,
) -> Option<i64> {
    for (imported_name, module_opt) in file_imports {
        // Collect candidate namespace prefixes to try: prefer module_opt if
        // it is non-empty and dotted, then fall back to imported_name.
        let mut prefixes: Vec<&str> = Vec::new();
        if let Some(m) = module_opt.as_deref() {
            if !m.is_empty() {
                prefixes.push(m);
            }
        }
        // Also try imported_name if it differs from module_opt (or module_opt
        // was None).  Avoid duplicates.
        if !prefixes.contains(&imported_name.as_str()) && !imported_name.is_empty() {
            prefixes.push(imported_name.as_str());
        }

        for prefix in prefixes {
            // Only attempt namespace expansion for dotted (multi-segment) prefixes.
            // Single-segment names are handled by P1 (exact name match).
            if !prefix.contains('.') {
                continue;
            }
            let candidate_qname = format!("{prefix}.{target_name}");
            if let Some(&id) = qname_to_id.get(&candidate_qname) {
                return Some(id);
            }
        }
    }
    None
}

/// Priority 3: the source file has `using X` / `import ... from "./X"` and
/// there's a symbol named `target_name` in the file/namespace matching X.
fn resolve_via_namespace(
    target_name: &str,
    _source_file: &str,
    file_imports: &[(String, Option<String>)],
    name_to_ids: &FxHashMap<String, Vec<(String, String, String, i64)>>,
    _parsed: &[ParsedFile],
) -> Option<i64> {
    let candidates = name_to_ids.get(target_name)?;

    for (file_path, qname, _sym_kind, id) in candidates {
        for (_imp_name, module_opt) in file_imports {
            if let Some(module) = module_opt {
                // C# namespace match: the symbol's qualified_name starts with the
                // imported namespace.
                // e.g. qname "FamilyBudget.Api.Entities.Category" starts with
                // imported namespace "FamilyBudget.Api.Entities"
                if qname.starts_with(module.as_str()) {
                    // Confirm the match is at a name boundary (dot or end of string),
                    // not a coincidental prefix like "Foo.Bar" matching namespace "Foo.B".
                    let rest = &qname[module.len()..];
                    if rest.is_empty() || rest.starts_with('.') {
                        return Some(*id);
                    }
                }
                // TypeScript / file-path match.
                if file_path_matches_module(file_path, module) {
                    return Some(*id);
                }
            }
        }
    }
    None
}

/// Priority 2.5: the source symbol is in namespace X, and a candidate's
/// qualified name is `X.TargetName` — same namespace, no import needed.
fn resolve_via_same_namespace(
    target_name: &str,
    source_namespace: &str,
    name_to_ids: &FxHashMap<String, Vec<(String, String, String, i64)>>,
) -> Option<i64> {
    let candidates = name_to_ids.get(target_name)?;
    let expected_qname = format!("{source_namespace}.{target_name}");

    for (_file_path, qname, _sym_kind, id) in candidates {
        if qname == &expected_qname {
            return Some(*id);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Auxiliary functions
// ---------------------------------------------------------------------------

/// Check whether a file path "matches" a module reference.
///
/// Return the parent directory portion of a file path (everything up to the last `/`).
fn parent_dir(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// Check if a file is a module entry point (public re-export surface).
fn is_module_entry_point(path: &str) -> bool {
    let basename = path.rsplit('/').next().unwrap_or(path);
    matches!(
        basename,
        "mod.rs" | "lib.rs"
            | "__init__.py"
            | "index.ts" | "index.tsx" | "index.js" | "index.jsx"
            | "index.mts" | "index.mjs"
    )
}

/// Handles both:
///   - TS relative imports: `./catalog` matches `src/catalog.ts`
///   - C# namespace: `System.Linq` matches a file in namespace `System`
fn file_path_matches_module(file_path: &str, module: &str) -> bool {
    if module.is_empty() {
        return false;
    }
    // Relative TS path: strip leading `./` and common extensions.
    let module_clean = module
        .trim_start_matches("./")
        .trim_start_matches("../");

    // Try suffix match (e.g., "catalog" matches "src/catalog.ts").
    let file_stem = file_path
        .trim_end_matches(".ts")
        .trim_end_matches(".tsx")
        .trim_end_matches(".js")
        .trim_end_matches(".cs");

    if file_stem.ends_with(module_clean) || file_stem.ends_with(&module_clean.replace('.', "/")) {
        return true;
    }

    // C# namespace: "System.Linq" — check if file_path contains "Linq" as a path component.
    let last_segment = module.rsplit('.').next().unwrap_or(module);
    file_path.contains(last_segment)
}

/// Returns true when `sym_kind` is a plausible target for the given edge kind.
///
/// Used by P4 to prefer kind-compatible candidates over incidental name
/// collisions (e.g. a `Calls` ref should prefer a method over a class).
fn kind_matches_symbol_kind(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test"),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class"
                | "struct"
                | "interface"
                | "enum"
                | "enum_member"
                | "type_alias"
                | "namespace"
                | "delegate"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        // Imports, HttpCall, DbEntity, LspResolved — accept any kind.
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Index builders
// ---------------------------------------------------------------------------

/// Build a map from simple name → list of (file_path, qualified_name, kind, symbol_id).
///
/// The `kind` string comes from the parsed symbol data so that P4 can
/// prefer kind-compatible candidates over incidental name collisions.
/// True when a parsed file or symbol_id_map key is an external dependency
/// source rather than a project file. Externals are identified by the
/// synthetic `ext:` virtual path prefix used by `indexer::externals`.
#[inline]
fn is_external_path(path: &str) -> bool {
    path.starts_with("ext:")
}

pub(super) fn build_name_index(
    symbol_id_map: &HashMap<(String, String), i64>,
    parsed: &[ParsedFile],
) -> FxHashMap<String, Vec<(String, String, String, i64)>> {
    // Build a secondary map from (file, qname) → kind string using parsed data.
    // External files are skipped — they belong in SymbolIndex for Tier 1 lookup
    // only, not in the heuristic fallback path (their symbols would pollute
    // cross-language name lookups, e.g. Python `get` matching a TS `get` ref).
    let mut kind_map: FxHashMap<(&str, &str), &str> = FxHashMap::default();
    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        for sym in &pf.symbols {
            kind_map.insert((pf.path.as_str(), sym.qualified_name.as_str()), sym.kind.as_str());
        }
    }

    let mut map: FxHashMap<String, Vec<(String, String, String, i64)>> = FxHashMap::default();
    for ((file, qname), &id) in symbol_id_map {
        if is_external_path(file) {
            continue;
        }
        // Extract the simple name (last segment of the qualified name).
        let simple = qname.rsplit('.').next().unwrap_or(qname.as_str()).to_string();
        let kind = kind_map
            .get(&(file.as_str(), qname.as_str()))
            .copied()
            .unwrap_or("")
            .to_string();
        map.entry(simple)
            .or_default()
            .push((file.clone(), qname.clone(), kind, id));
    }
    map
}

/// Build a map from qualified_name → symbol_id for exact dotted-path matches.
pub(super) fn build_qname_index(
    symbol_id_map: &HashMap<(String, String), i64>,
) -> FxHashMap<String, i64> {
    symbol_id_map
        .iter()
        .filter(|((file, _), _)| !is_external_path(file))
        .map(|((_, qname), &id)| (qname.clone(), id))
        .collect()
}

/// Build a map from file_path → namespace for same-namespace resolution.
///
/// For each file, finds the first `Namespace` symbol and records its
/// qualified name as the file's namespace.
pub(super) fn build_file_namespace_map(parsed: &[ParsedFile]) -> FxHashMap<String, String> {
    let mut map = FxHashMap::default();
    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        if let Some(ns_sym) = pf.symbols.iter().find(|s| s.kind == SymbolKind::Namespace) {
            map.insert(pf.path.clone(), ns_sym.qualified_name.clone());
        }
    }
    map
}

/// Build a reverse index: module/namespace name → file paths.
///
/// Two sources:
///   1. Namespace symbols: Erlang `-module(lists)`, Pascal `unit SysUtils`,
///      Clojure `(ns my.ns)` — explicit declarations.
///   2. File stems: `list.ml` → "list" and "List" (OCaml convention:
///      file modules are capitalized).
///
/// This enables precise module-qualified resolution: given `module="List"`
/// and `target_name="map"`, look up "List" → ["lib/list.ml"] → find "map"
/// among candidates in that file.
pub(super) fn build_module_to_files(parsed: &[ParsedFile]) -> FxHashMap<String, Vec<String>> {
    let mut map: FxHashMap<String, Vec<String>> = FxHashMap::default();

    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        // 1. Namespace symbols → module name (exact, authoritative)
        for sym in &pf.symbols {
            if sym.kind == SymbolKind::Namespace {
                let entry = map.entry(sym.name.clone()).or_default();
                if !entry.contains(&pf.path) {
                    entry.push(pf.path.clone());
                }
            }
        }

        // 2. File stem → module name
        let norm = pf.path.replace('\\', "/");
        if let Some(basename) = norm.rsplit('/').next() {
            if let Some(stem) = basename.rsplit_once('.').map(|(s, _)| s) {
                if !stem.is_empty() {
                    // Original case (e.g., "list" from "list.ml")
                    let entry = map.entry(stem.to_string()).or_default();
                    if !entry.contains(&pf.path) {
                        entry.push(pf.path.clone());
                    }
                    // Capitalized (OCaml/Haskell convention: file → Module)
                    let capitalized = capitalize_first(stem);
                    if capitalized != stem {
                        let entry = map.entry(capitalized).or_default();
                        if !entry.contains(&pf.path) {
                            entry.push(pf.path.clone());
                        }
                    }
                    // Lowercase for case-insensitive lookup
                    let lower = stem.to_lowercase();
                    if lower != stem {
                        let entry = map.entry(lower).or_default();
                        if !entry.contains(&pf.path) {
                            entry.push(pf.path.clone());
                        }
                    }
                }
            }
        }
    }
    map
}

/// Capitalize the first character of a string (for OCaml/Haskell module convention).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Build a per-file import map from the parsed extraction results.
///
/// Returns: file_path → Vec<(imported_name, module_path)>
///
/// For C#: `using FamilyBudget.Api.Entities;`
///         → ("FamilyBudget.Api.Entities", Some("FamilyBudget.Api.Entities"))  (EdgeKind::Imports)
/// For TS: `import { Foo } from "./foo"`
///         → ("Foo", Some("./foo"))  (EdgeKind::TypeRef with module)
pub(super) fn build_import_map(
    parsed: &[ParsedFile],
) -> FxHashMap<String, Vec<(String, Option<String>)>> {
    let mut map: FxHashMap<String, Vec<(String, Option<String>)>> = FxHashMap::default();
    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        for r in &pf.refs {
            match r.kind {
                EdgeKind::TypeRef if r.module.is_some() => {
                    // TypeScript-style: import { Foo } from "./bar"
                    map.entry(pf.path.clone())
                        .or_default()
                        .push((r.target_name.clone(), r.module.clone()));
                }
                EdgeKind::Imports => {
                    // C#-style: using FamilyBudget.Api.Entities
                    // The full namespace is stored in both target_name and module.
                    map.entry(pf.path.clone())
                        .or_default()
                        .push((r.target_name.clone(), r.module.clone()));
                }
                _ => {}
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "heuristic_tests.rs"]
mod tests;
