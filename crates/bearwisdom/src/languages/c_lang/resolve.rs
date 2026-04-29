// =============================================================================
// indexer/resolve/rules/c_lang/mod.rs -- C/C++ resolution rules
//
// Scope rules for C/C++:
//
//   1. Scope chain walk: innermost namespace/class -> outermost.
//   2. `#include`-based import resolution: system headers -> stdlib; user
//      headers -> project files.
//   3. Namespace-qualified names: `std::vector` -> external; `MyNS::Foo` -> index.
//   4. Template parameter detection: single uppercase letters and known
//      template-param names are classified as external (template_param namespace).
//
// C/C++ include model:
//   `#include <foo.h>`   -> EdgeKind::Imports, target_name = "foo.h"  (system)
//   `#include "bar.h"`   -> EdgeKind::Imports, target_name = "bar.h"  (project)
//
// The extractor does not always set `module` for includes; we rely on the
// target_name (the header path) to distinguish system from project headers.
// =============================================================================

use super::{predicates, type_checker::CChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Sentinel stored in `FileContext::file_namespace` for C files inside an R
/// package (project has a DESCRIPTION manifest). Lets `resolve()` and
/// `infer_external_namespace()` gate R C API classification without threading
/// `ProjectContext` through the resolution hot-path.
const R_PACKAGE_SENTINEL: &str = "__r_package__";

/// C/C++ language resolver.
pub struct CLangResolver;

impl LanguageResolver for CLangResolver {
    fn language_ids(&self) -> &[&str] {
        &["c", "cpp"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // C/C++ uses `#include` -- the extractor emits these as EdgeKind::Imports.
        // target_name = the header path (e.g., "stdio.h", "vector", "mylib/foo.h").
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let header = r.target_name.trim_matches(|c| c == '<' || c == '>' || c == '"');
            imports.push(ImportEntry {
                imported_name: header.to_string(),
                module_path: Some(header.to_string()),
                alias: None,
                is_wildcard: false,
            });
        }

        // C/C++ files belong to no named namespace by default; namespace
        // declarations are per-block, not file-level.
        //
        // Exception: when the project has a DESCRIPTION manifest the C file
        // lives inside an R package. Store a sentinel in file_namespace so
        // resolve() / infer_external_namespace() can classify R C API
        // symbols (SEXP, PROTECT, Rf_*, ...) without ProjectContext threading.
        let file_namespace = if project_ctx
            .map(|ctx| ctx.manifests.contains_key(&ManifestKind::Description))
            .unwrap_or(false)
        {
            Some(R_PACKAGE_SENTINEL.to_string())
        } else {
            None
        };

        FileContext {
            file_path: file.path.clone(),
            language: file.language.clone(),
            imports,
            file_namespace,
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

        // Template parameters and builtins are never in the index.
        if predicates::is_template_param(target) || predicates::is_c_builtin(target) {
            return None;
        }

        // Chain-aware resolution: walk member chains like `obj.method()` or
        // `this->field.method()` by following field types through the index.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = CChecker.resolve_chain(
                chain_ref, edge_kind, None, ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // R C API symbols (Rinternals.h, Rdefines.h, R_ext/*.h) are never in
        // the project index. Skip the index walk for R package projects.
        if file_ctx.file_namespace.as_deref() == Some(R_PACKAGE_SENTINEL)
            && predicates::is_r_c_api_symbol(target)
        {
            return None;
        }

        // Strip `this->` prefix for member access.
        let effective_target = target
            .strip_prefix("this->")
            .or_else(|| target.strip_prefix("this."))
            .unwrap_or(target);

        // Step 1 (C-specific): Scope chain walk using `::` separator.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}::{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "c_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2 (C-specific): Namespace-qualified lookup (e.g., `MyNS::Foo`).
        if effective_target.contains("::") {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "c_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 3: Common resolution (dot-scope chain, same-file, import-based).
        // `effective_target` may differ from `target` (this-> stripped), so we
        // build a synthetic RefContext-alike by delegating with the original ref_ctx.
        // resolve_common uses ref_ctx.extracted_ref.target_name directly, which is
        // the unstripped `target`. Re-check with the stripped name via same-file lookup.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "c_same_file",
                    resolved_yield_type: None,
                });
            }
        }

        // Step 4 (C-specific): Global by-name fallback.
        //
        // Catches external synthetic symbols (Clay UI macro-generated array
        // functions, SDL API, POSIX extension APIs) that are not reachable via
        // same-file lookup, scope-chain, or import-based paths. Single-char
        // names are skipped to avoid false positives from template parameters
        // that slipped past is_template_param.
        //
        // `kind_compatible` is intentionally NOT checked here: C enum constants
        // (e.g. CLAY_* alignment values) are used both in call-expression
        // positions and as type-refs, but their symbol kind is enum_member which
        // doesn't match either EdgeKind::Calls or EdgeKind::TypeRef in
        // kind_compatible. The global fallback step is conservative — it only
        // fires when all local resolution paths have failed, so the risk of a
        // spurious match is low and bounded by the target name length guard.
        if !effective_target.contains("::") && effective_target.len() > 1 {
            let by_name_hits = lookup.by_name(effective_target);
            if let Some(sym) = by_name_hits.first() {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "c_by_name",
                    resolved_yield_type: None,
                });
            }
        }

        if let Some(res) = engine::resolve_common(
            "c", file_ctx, ref_ctx, lookup, predicates::kind_compatible,
        ) {
            return Some(res);
        }

        // C bare-name fallback. Counterpart to the SCSS / Bash / Python /
        // Java / PowerShell `<lang>_bare_name` template. C has no real
        // namespacing — every external function (libc, POSIX threads,
        // platform APIs) is callable by bare name once its header is
        // included. The engine's module/import/scope path can't bind
        // these. Index-wide name lookup gated by `.c`/`.h`/`.cc`/`.hpp`
        // file-extension keeps cross-language collisions out.
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
            && !target.contains("::")
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_c_or_cpp = path.ends_with(".c")
                    || path.ends_with(".h")
                    || path.ends_with(".cc")
                    || path.ends_with(".cpp")
                    || path.ends_with(".cxx")
                    || path.ends_with(".hpp")
                    || path.ends_with(".hh")
                    || path.ends_with(".hxx")
                    || path.starts_with("ext:c:")
                    || path.starts_with("ext:cpp:");
                if !is_c_or_cpp {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "c_bare_name",
                    resolved_yield_type: None,
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // R C API symbols (Rinternals.h, Rdefines.h, R_ext/*.h).
        // Only classify as external when the C file is inside an R package.
        if file_ctx.file_namespace.as_deref() == Some(R_PACKAGE_SENTINEL)
            && predicates::is_r_c_api_symbol(target)
        {
            return Some("r.c.api".to_string());
        }

        // Include directives -- classify system headers as external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let header = target.trim_matches(|c| c == '<' || c == '>' || c == '"');
            if predicates::is_system_header(header) {
                return Some("stdlib".to_string());
            }
            // boost or other known-external headers.
            if header.starts_with("boost/")
                || header.starts_with("gtest/")
                || header.starts_with("gmock/")
            {
                return Some("external".to_string());
            }
            return None;
        }

        // Template parameters get their own namespace.
        if predicates::is_template_param(target) {
            return Some("template_param".to_string());
        }

        // C/C++ builtins (stdlib functions, types, macros).
        if predicates::is_c_builtin(target) {
            return Some("c.stdlib".to_string());
        }

        // `std::` prefixed names.
        if target.starts_with("std::") || target.starts_with("::std::") {
            return Some("std".to_string());
        }

        // Other known-external namespace prefixes.
        let root = target
            .strip_prefix("::")
            .unwrap_or(target)
            .split("::")
            .next()
            .unwrap_or(target);
        if predicates::is_external_c_namespace(root) {
            return Some(root.to_string());
        }

        None
    }
}
