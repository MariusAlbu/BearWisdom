// =============================================================================
// indexer/resolve/rules/c_lang/mod.rs — C/C++ resolution rules
//
// Scope rules for C/C++:
//
//   1. Scope chain walk: innermost namespace/class → outermost.
//   2. `#include`-based import resolution: system headers → stdlib; user
//      headers → project files.
//   3. Namespace-qualified names: `std::vector` → external; `MyNS::Foo` → index.
//   4. Template parameter detection: single uppercase letters and known
//      template-param names are classified as external (template_param namespace).
//
// C/C++ include model:
//   `#include <foo.h>`   → EdgeKind::Imports, target_name = "foo.h"  (system)
//   `#include "bar.h"`   → EdgeKind::Imports, target_name = "bar.h"  (project)
//
// The extractor does not always set `module` for includes; we rely on the
// target_name (the header path) to distinguish system from project headers.
// =============================================================================


use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// C/C++ language resolver.
pub struct CLangResolver;

impl LanguageResolver for CLangResolver {
    fn language_ids(&self) -> &[&str] {
        &["c", "cpp"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // C/C++ uses `#include` — the extractor emits these as EdgeKind::Imports.
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
        FileContext {
            file_path: file.path.clone(),
            language: file.language.clone(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Template parameters and builtins are never in the index.
        if builtins::is_template_param(target) || builtins::is_c_builtin(target) {
            return None;
        }

        // Strip `this->` prefix for member access.
        let effective_target = target
            .strip_prefix("this->")
            .or_else(|| target.strip_prefix("this."))
            .unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}::{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "c_scope_chain",
                    });
                }
            }
            // Also try dot-separated qualified name (some extractors use `.` not `::`).
            let candidate_dot = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate_dot) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "c_scope_chain_dot",
                    });
                }
            }
        }

        // Step 2: Namespace-qualified lookup (e.g., `MyNS::Foo`).
        if effective_target.contains("::") {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "c_qualified_name",
                    });
                }
            }
        }

        // Step 3: Simple name lookup (global / anonymous namespace).
        for sym in lookup.by_name(effective_target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "c_simple_name",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Include directives — classify system headers as external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let header = target.trim_matches(|c| c == '<' || c == '>' || c == '"');
            if builtins::is_system_header(header) {
                return Some("stdlib".to_string());
            }
            // boost or other known-external headers.
            if header.starts_with("boost/") || header.starts_with("gtest/") || header.starts_with("gmock/") {
                return Some("external".to_string());
            }
            return None;
        }

        // Template parameters get their own namespace.
        if builtins::is_template_param(target) {
            return Some("template_param".to_string());
        }

        // C/C++ builtins (stdlib functions, types, macros).
        if builtins::is_c_builtin(target) {
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
        if builtins::is_external_c_namespace(root) {
            return Some(root.to_string());
        }

        None
    }
}
