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


use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::type_checker::chain::{
    self, ChainConfig, NamespaceLookup, identity_normalize,
};
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

        // Bare-name walker lookup. dart_sdk + flutter_sdk + pub emit real
        // symbols for dart:core (String, List, Map, Future, Stream),
        // dart:async, dart:io, Flutter widgets, and declared pub deps.
        // ext:-only filter, gated on chain.is_none().
        if ref_ctx.extracted_ref.chain.is_none() && !target.contains('.') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "dart_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Chain-aware resolution: walk MemberChain following field/return types.
        // Dart has generics but no wildcard-import namespace lookup.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            let config = ChainConfig {
                strategy_prefix: "dart",
                normalize_type: identity_normalize,
                has_self_ref: true,
                enclosing_type_kinds: &["class", "enum", "mixin"],
                static_type_kinds: &["class", "enum", "mixin", "type_alias", "extension"],
                use_generics: true,
                namespace_lookup: NamespaceLookup::None,
                kind_compatible: predicates::kind_compatible,
            };
            if let Some(res) = chain::resolve_via_chain(
                &config, chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "dart_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "dart_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Step 3: Simple name lookup across the project.
        for sym in lookup.by_name(effective_target) {
            if predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "dart_by_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Dart bare-name fallback. Continues the cross-language
        // template (PRs 31, 35-40, plus Lua, Go, Rust, Kotlin, Ruby).
        // Dart's `import 'package:foo/bar.dart'` brings whole
        // libraries into scope; bare-name calls fall here when the
        // engine's module/import path can't bind. Gated by `.dart`
        // file extension.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_dart = path.ends_with(".dart");
                if !is_dart {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "dart_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs — classify the URI.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let uri = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            if predicates::is_external_dart_import(uri) {
                // Return just the package/library root.
                let ns = if uri.starts_with("dart:") {
                    "dart.stdlib"
                } else if let Some(pkg_path) = uri.strip_prefix("package:") {
                    // `package:flutter/material.dart` → "flutter"
                    pkg_path.split('/').next().unwrap_or(pkg_path)
                } else {
                    uri
                };
                return Some(ns.to_string());
            }
            // For `package:` URIs not in the hardcoded list, check pubspec.yaml.
            if uri.starts_with("package:") {
                if let Some(ctx) = project_ctx {
                    if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Pubspec) {
                        let pkg_name = uri
                            .strip_prefix("package:")
                            .unwrap_or(uri)
                            .split('/')
                            .next()
                            .unwrap_or(uri);
                        if manifest.dependencies.contains(pkg_name) {
                            return Some(pkg_name.to_string());
                        }
                    }
                }
            }
            return None;
        }


        // Walk imports: if target was imported from an external URI, classify it.
        let simple = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            let uri = import.module_path.as_deref().unwrap_or("");
            if uri.is_empty() {
                continue;
            }

            // For package: URIs, check manifests first before the hardcoded list.
            let pkg_name_from_uri = if uri.starts_with("package:") {
                uri.strip_prefix("package:")
                    .unwrap_or(uri)
                    .split('/')
                    .next()
                    .unwrap_or(uri)
            } else {
                ""
            };

            let is_manifest_external = !pkg_name_from_uri.is_empty()
                && project_ctx
                    .and_then(|ctx| ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Pubspec))
                    .is_some_and(|m| m.dependencies.contains(pkg_name_from_uri));

            // Alias-qualified: `u.Foo` where `import '...' as u`
            if let Some(alias) = &import.alias {
                if alias == simple {
                    if is_manifest_external || predicates::is_external_dart_import(uri) {
                        if uri.starts_with("package:") {
                            return Some(pkg_name_from_uri.to_string());
                        }
                        return Some(uri.to_string());
                    }
                }
            }
            // Wildcard: any name could come from any non-aliased external import.
            if import.alias.is_none() {
                if is_manifest_external {
                    return Some(pkg_name_from_uri.to_string());
                }
                if predicates::is_external_dart_import(uri) {
                    if uri.starts_with("package:") {
                        return Some(pkg_name_from_uri.to_string());
                    }
                    return Some("dart.stdlib".to_string());
                }
            }
        }

        None
    }
}
