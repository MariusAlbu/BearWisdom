// =============================================================================
// indexer/resolve/rules/dart/mod.rs — Dart resolution rules
//
// Scope rules for Dart:
//
//   1. Scope chain walk: innermost class/function → outermost.
//   2. Same-file resolution: all top-level symbols are visible within the file.
//   3. Import-based resolution: `import 'package:foo/bar.dart'` or
//      `import '../models/user.dart'` brings symbols into scope.
//   4. Fully qualified names via `as` prefix aliases.
//
// Dart import model:
//   `import 'dart:core'`                   → stdlib (always in scope)
//   `import 'package:flutter/material.dart'` → external pub package
//   `import '../models/user.dart'`          → project-relative
//   `import 'user.dart' as u`              → aliased import
//
// The extractor emits EdgeKind::Imports with:
//   target_name = the URI string or the local alias
//   module      = the URI string (for `as` imports)
// =============================================================================


use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Dart language resolver.
pub struct DartResolver;

impl LanguageResolver for DartResolver {
    fn language_ids(&self) -> &[&str] {
        &["dart"]
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
            // For Dart, target_name is the import URI (or alias for `as` imports).
            // module is the URI when an alias is present.
            let uri = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != uri {
                Some(r.target_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name: uri.to_string(),
                module_path: Some(uri.to_string()),
                alias,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "dart".to_string(),
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

        // Dart builtins are never in the index.
        if builtins::is_dart_builtin(target) {
            return None;
        }

        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "dart_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "dart_same_file",
                });
            }
        }

        // Step 3: Simple name lookup across the project.
        for sym in lookup.by_name(effective_target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "dart_by_name",
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

        // Import refs — classify the URI.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let uri = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            if builtins::is_external_dart_import(uri) {
                // Return just the package/library root.
                let ns = if uri.starts_with("dart:") {
                    "dart.stdlib"
                } else if let Some(pkg) = uri.strip_prefix("package:") {
                    // `package:flutter/material.dart` → "flutter"
                    pkg.split('/').next().unwrap_or(pkg)
                } else {
                    uri
                };
                return Some(ns.to_string());
            }
            return None;
        }

        // Dart builtins.
        if builtins::is_dart_builtin(target) {
            return Some("dart.core".to_string());
        }

        // Walk imports: if target was imported from an external URI, classify it.
        let simple = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            let uri = import.module_path.as_deref().unwrap_or("");
            if uri.is_empty() {
                continue;
            }
            // Alias-qualified: `u.Foo` where `import '...' as u`
            if let Some(alias) = &import.alias {
                if alias == simple {
                    if builtins::is_external_dart_import(uri) {
                        if uri.starts_with("package:") {
                            let pkg = uri
                                .strip_prefix("package:")
                                .unwrap_or(uri)
                                .split('/')
                                .next()
                                .unwrap_or(uri);
                            return Some(pkg.to_string());
                        }
                        return Some(uri.to_string());
                    }
                }
            }
            // Wildcard: any name could come from any non-aliased external import.
            if import.alias.is_none() && builtins::is_external_dart_import(uri) {
                if uri.starts_with("package:") {
                    // Return the package root as potential namespace.
                    let pkg = uri
                        .strip_prefix("package:")
                        .unwrap_or(uri)
                        .split('/')
                        .next()
                        .unwrap_or(uri);
                    return Some(pkg.to_string());
                }
                return Some("dart.stdlib".to_string());
            }
        }

        None
    }
}
