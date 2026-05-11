// =============================================================================
// erlang/resolve.rs — Erlang resolution rules
//
// Scope rules for Erlang:
//
//   1. Scope chain walk: innermost function → module level.
//   2. Same-file resolution: all top-level functions in the module are visible.
//   3. Import-based resolution: `-import(Module, [Fun/Arity]).` and
//      `-include("header.hrl").` bring external symbols into scope.
//
// Erlang import model:
//   `-module(mod_name).`          → declares the module name
//   `-import(Module, [Fun/Arity]).` → imports specific functions from a module
//   `-include("header.hrl").`       → textual include (local header)
//   Module:function()               → remote call (not an import)
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Erlang language resolver.
pub struct ErlangResolver;

impl LanguageResolver for ErlangResolver {
    fn language_ids(&self) -> &[&str] {
        &["erlang"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // After the extractor update, -import(lists, [map/2, foldl/3]) emits:
            //   target_name = "map/2",  module = Some("lists")
            //   target_name = "foldl/3", module = Some("lists")
            // The fallback (empty funs list) emits:
            //   target_name = "lists", module = Some("lists"), is_wildcard treated below.
            //
            // Include and wildcard module-level entries are non-function references
            // (path strings or bare module names) and stay as wildcards.
            let is_function_import = r.target_name.contains('/');
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone().or_else(|| Some(r.target_name.clone())),
                alias: None,
                is_wildcard: !is_function_import,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "erlang".to_string(),
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

        // OTP/stdlib lookup — `target_name` is now emitted as "name/arity" by
        // the extractor, so `by_name("self/0")` directly matches the OTP index.
        // Scoped to ext:erlang: to avoid C/C++ symbols from NIF-adjacent builds.
        if edge_kind == EdgeKind::Calls && !target.contains(':') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:erlang:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "erlang_otp_arity",
                    resolved_yield_type: None,
                });
            }
        }

        // -import(Module, [Fun/N]) resolution.
        //
        // The extractor emits each imported function as an Imports ref with
        // `target_name = "fun/arity"` and `module = Some(source_module)`.
        // When a Calls ref matches an imported name+arity pair, resolve to any
        // symbol with that name in the declared source module.
        if edge_kind == EdgeKind::Calls && !target.contains(':') {
            if let Some(res) = self.resolve_via_import(file_ctx, target, lookup) {
                return Some(res);
            }
        }

        // Run common resolution.
        if let Some(res) = engine::resolve_common("erlang", file_ctx, ref_ctx, lookup, predicates::kind_compatible) {
            return Some(res);
        }

        // Exact same-file arity lookup.
        //
        // Call-site refs now carry "name/arity" so this is a direct by_name hit
        // against same-file symbols. Falls back to basename prefix for refs
        // emitted without arity (fallback path in collect_calls).
        if edge_kind == EdgeKind::Calls {
            for sym in lookup.in_file(&file_ctx.file_path) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                if sym.name == target.as_str() {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "erlang_same_file_arity",
                        resolved_yield_type: None,
                    });
                }
            }
            // Basename fallback for dynamic / variable calls that lack arity suffix.
            let target_base = target.split('/').next().unwrap_or(target.as_str());
            if !target_base.is_empty() && !target.contains('/') {
                for sym in lookup.in_file(&file_ctx.file_path) {
                    if !predicates::kind_compatible(edge_kind, &sym.kind) {
                        continue;
                    }
                    let sym_base = sym.name.split('/').next().unwrap_or(&sym.name);
                    if sym_base == target_base {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.8,
                            strategy: "erlang_same_file_base",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Cross-file project-internal arity lookup.
        //
        // Handles calls to functions defined in other .erl files within the same
        // project (e.g. helper modules in test/). Excluded from same-file pass
        // above; `by_name` finds every symbol with the given "name/arity" key
        // across all project files. OTP symbols were already handled above, so
        // we skip ext: paths here to avoid false matches.
        if edge_kind == EdgeKind::Calls && !target.contains(':') {
            for sym in lookup.by_name(target) {
                if sym.file_path.starts_with("ext:") {
                    continue;
                }
                if sym.file_path.as_ref() == file_ctx.file_path.as_str() {
                    continue; // already checked in same-file pass
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "erlang_cross_file_arity",
                    resolved_yield_type: None,
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // erlang_otp walker emits real symbols and resolve() above binds
        // them. Names that exhaust resolve() stay unresolved rather than
        // being blanket-classified as `builtin`.
        None
    }

    // No infer_external_namespace_with_lookup override — default delegates
    // to infer_external_namespace which returns None.
}

impl ErlangResolver {
    /// Look up `name/arity` against the file's `-import(Module, [fun/N])` declarations.
    /// Returns the first OTP or project symbol matching the declared source module.
    fn resolve_via_import(
        &self,
        file_ctx: &FileContext,
        target: &str,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        for entry in &file_ctx.imports {
            if entry.imported_name != target {
                continue;
            }
            // entry.module_path holds the source module name.
            let source_mod = entry.module_path.as_deref()?;
            for sym in lookup.by_name(target) {
                // Match symbols whose file path contains the module name.
                // OTP symbols: ext:erlang:stdlib/lists.erl → contains "lists".
                // Project symbols: bare file path ending in <module>.erl.
                let path = sym.file_path.as_ref();
                let matches = path.contains(source_mod)
                    || path
                        .rsplit('/')
                        .next()
                        .and_then(|f| f.strip_suffix(".erl"))
                        .map(|stem| stem == source_mod)
                        .unwrap_or(false);
                if matches && predicates::kind_compatible(EdgeKind::Calls, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.92,
                        strategy: "erlang_import_arity",
                        resolved_yield_type: None,
                    });
                }
            }
        }
        None
    }
}
