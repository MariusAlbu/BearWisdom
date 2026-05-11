// =============================================================================
// nim/resolve.rs — Nim resolution rules
//
// Scope rules for Nim:
//
//   1. Scope chain walk: innermost proc/type → outermost.
//   2. Same-file resolution: all top-level symbols visible within the file.
//   3. Import-based resolution:
//        `import module`            → all exported symbols from module
//        `from module import sym`   → only named symbols
//        `include file`             → textual inclusion, all symbols visible
//
// The extractor emits EdgeKind::Imports with:
//   target_name = module name or symbol name
//   module      = module path for `from ... import` forms
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Nim language resolver.
pub struct NimResolver;

impl LanguageResolver for NimResolver {
    fn language_ids(&self) -> &[&str] {
        &["nim"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Every Nim module implicitly imports `system`. Adding it as a
        // wildcard entry here lets the common resolver find builtins like
        // `newException`, `echo`, `cast`, and `GC_*` without requiring an
        // explicit `import system` in the source file.
        imports.push(ImportEntry {
            imported_name: "system".to_string(),
            module_path: Some("system".to_string()),
            alias: None,
            is_wildcard: true,
        });

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // `from module import sym` → module is in r.module, sym in r.target_name
            // `import module`          → module name is r.target_name
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());

            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: r.module.is_none(), // plain `import` = wildcard
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "nim".to_string(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        if let Some(res) = engine::resolve_common("nim", file_ctx, ref_ctx, lookup, predicates::kind_compatible) {
            return Some(res);
        }

        // Nim files expose symbols via their module name (the file stem). When
        // `import foo` or `import std/foo` is present, all exported symbols
        // from `foo.nim` are in scope — but `resolve_common`'s wildcard step
        // can only match when the symbol lives in a file whose stem equals
        // the last path segment of the import. Refs that arrive here without
        // a module qualifier (bare `target_name`, `module=None`) but whose
        // definition is in an external Nim file reachable from ANY of this
        // file's imports fall through that step.
        //
        // Strategy: look up `target` by name and filter candidates to those
        // whose file path contains a segment that matches one of the file's
        // imported module leaf names. This covers:
        //   `import results`     → `some(...)` from results.nim (Opt.some pattern)
        //   `import chronos`     → `newFuture(...)` from chronos.nim
        //   `import std/options` → `some(...)` / `none(...)` from options.nim
        nim_module_file_stem_resolve(file_ctx, target, edge_kind, lookup)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // `import std/X` is the Nim stdlib namespace. The compiler's lib/
        // tree publishes those modules but each one is a *file*, not a
        // symbol — so the heuristic can't bind the import edge. Treat
        // `std/<anything>` as external so the import counts as handled.
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            if let Some(rest) = target.strip_prefix("std/") {
                return Some(format!("ext:nim-stdlib:{rest}"));
            }
            // Bracketed form `std/[strutils, os]` and `from std/X import Y`
            // also reach this resolver — recognise both.
            if target.starts_with("std/") {
                return Some("ext:nim-stdlib".to_string());
            }
            // `pkg/<X>` is the Nimble-package namespace shorthand.
            if let Some(rest) = target.strip_prefix("pkg/") {
                return Some(format!("ext:nim-pkg:{rest}"));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Module-to-file-stem resolution
// ---------------------------------------------------------------------------

/// Resolve a bare Nim ref whose `module` field is absent by matching its
/// name against external symbols in files reachable from the file's imports.
///
/// Nim's `import foo` / `import std/foo` brings every exported symbol from
/// `foo.nim` into scope.  The extractor emits bare target names (no `module`
/// qualifier) because Nim source rarely qualifies calls.  `resolve_common`'s
/// wildcard step handles the common case; this function covers the remainder:
///
/// - Method-call terminals (`Opt.some(...)` → target `some`, no module) where
///   the receiver type's defining module is NOT among the file's direct imports
///   but IS transitively available.
/// - Symbols available via `export` re-exports inside an already-imported module.
///
/// For each candidate in `by_name(target)` whose file path (lowercased) contains
/// a segment equal to the leaf of any import in the file context, return the
/// first kind-compatible match.  Confidence is 0.85 — lower than `resolve_common`'s
/// 0.95 wildcard step to preserve its priority.
fn nim_module_file_stem_resolve(
    file_ctx: &FileContext,
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    // Build a set of leaf module names from the file's imports.
    // `std/options` → `options`, `chronos` → `chronos`, `system` → `system`.
    let import_leaves: Vec<&str> = file_ctx
        .imports
        .iter()
        .filter_map(|imp| imp.module_path.as_deref())
        .map(|mp| {
            // Strip `std/` / `pkg/` prefix then take the final `/`-segment.
            let stripped = mp
                .strip_prefix("std/")
                .or_else(|| mp.strip_prefix("pkg/"))
                .unwrap_or(mp);
            stripped.rsplit('/').next().unwrap_or(stripped)
        })
        .collect();

    if import_leaves.is_empty() {
        return None;
    }

    let by_name = lookup.by_name(target);

    // Prefer symbols from external Nim files (the project files were already
    // tried by `resolve_common`'s same-file and scope-chain steps).
    let nim_external: Vec<&SymbolInfo> = by_name
        .iter()
        .filter(|s| {
            let fp = s.file_path.as_ref();
            (fp.starts_with("ext:nim:") || fp.starts_with("ext:idx:"))
                && fp.to_lowercase().ends_with(".nim")
        })
        .collect();

    if nim_external.is_empty() {
        return None;
    }

    for sym in &nim_external {
        if !predicates::kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        let fl = sym.file_path.to_lowercase().replace('\\', "/");
        for leaf in &import_leaves {
            let leaf_lower = leaf.to_lowercase();
            // File stem match: `.../<leaf>.nim` — direct module file.
            // Path segment match: `.../<leaf>/...` — package subdirectory.
            if fl.ends_with(&format!("/{leaf_lower}.nim"))
                || fl.contains(&format!("/{leaf_lower}/"))
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "nim_module_file_stem",
                    resolved_yield_type: None,
                });
            }
        }
    }

    None
}
