// =============================================================================
// indexer/resolve/rules/scala/mod.rs — Scala resolution rules
//
// Scope rules for Scala:
//
//   1. Scope chain walk: innermost class/object/function → outermost.
//   2. Same-package resolution: types in the same package are visible without
//      explicit import (Scala package visibility).
//   3. Exact import resolution: `import com.foo.Bar` → Bar directly visible.
//   4. Wildcard import: `import com.foo._` → all types in that package visible.
//   5. Fully qualified names: dotted names resolve directly.
//
// Scala import model:
//   `import com.foo.Bar`   → target_name = "Bar",  module = "com.foo.Bar"
//   `import com.foo._`     → target_name = "_",    module = "com.foo"
//   `import com.foo.{A, B}` → two separate Imports refs
// =============================================================================


use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::type_checker::chain::{
    self, ChainConfig, NamespaceLookup, identity_normalize,
};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Scala language resolver.
pub struct ScalaResolver;

impl LanguageResolver for ScalaResolver {
    fn language_ids(&self) -> &[&str] {
        &["scala"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract package declaration.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module = r.module.as_deref().unwrap_or(&r.target_name);
            // Scala wildcard is `_` (Scala 2) or `*` (Scala 3).
            let is_wildcard = r.target_name == "_" || r.target_name == "*";

            if is_wildcard {
                imports.push(ImportEntry {
                    imported_name: String::new(),
                    module_path: Some(module.to_string()),
                    alias: None,
                    is_wildcard: true,
                });
            } else {
                imports.push(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(module.to_string()),
                    alias: None,
                    is_wildcard: false,
                });
            }
        }

        FileContext {
            file_path: file.path.clone(),
            language: "scala".to_string(),
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

        // Bare-name walker lookup. scala_stdlib + jdk_src + maven (sources
        // jars) emit real symbols for the Scala stdlib (List, Option, Either,
        // Try), JVM types, and declared deps. ext:-only filter so chain
        // walker / scope / wildcard-import paths still win for project
        // symbols.
        if !target.contains('.') {
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
                    strategy: "scala_synthetic_global",
                    resolved_yield_type: None,
                });
            }
        }

        // Chain-aware resolution: walk MemberChain following field/return types.
        // Scala is JVM-like with wildcard imports and companion objects.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            let config = ChainConfig {
                strategy_prefix: "scala",
                normalize_type: identity_normalize,
                has_self_ref: true,
                enclosing_type_kinds: &["class", "trait", "object"],
                static_type_kinds: &["class", "trait", "object", "enum", "type_alias"],
                use_generics: true,
                namespace_lookup: NamespaceLookup::WildcardOnly,
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
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "scala_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2: Same-package resolution.
        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "scala_same_package",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 3: Exact import resolution.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                continue;
            }
            let name_match = import.imported_name == effective_target
                || import.alias.as_deref() == Some(effective_target);
            if !name_match {
                continue;
            }
            if let Some(module) = &import.module_path {
                if let Some(sym) = lookup.by_qualified_name(module) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "scala_import",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Step 4: Wildcard import resolution.
        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            if let Some(module) = &import.module_path {
                let candidate = format!("{module}.{effective_target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "scala_wildcard_import",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Step 5: Fully qualified name.
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "scala_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 6: Implicit imports — Scala compiles every file with the
        // equivalent of `import java.lang._; import scala._; import
        // scala.Predef._`. Bare references to `Throwable`, `Exception`,
        // `Object`, `Class`, `Number`, `Integer`, etc. are valid Scala
        // because java.lang is implicitly imported. Try the three implicit
        // namespaces in compiler order; first-hit wins.
        if !effective_target.contains('.') {
            for prefix in &["java.lang", "scala", "scala.Predef"] {
                let candidate = format!("{prefix}.{effective_target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "scala_implicit_import",
                            resolved_yield_type: None,
                        });
                    }
                }
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

        // Import refs.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);

            // Manifest-driven: check Maven and Gradle group IDs first.
            if let Some(ctx) = project_ctx {
                for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                    if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&kind) {
                        if manifest.dependencies.iter().any(|group_id| {
                            import_path == group_id
                                || import_path.starts_with(group_id.as_str())
                                    && import_path.as_bytes().get(group_id.len())
                                        == Some(&b'.')
                        }) {
                            return Some(import_path.to_string());
                        }
                    }
                }
            }

            if predicates::is_external_scala_namespace(import_path, project_ctx) {
                return Some(import_path.to_string());
            }
            return None;
        }

        // Walk imports for a match.
        for import in &file_ctx.imports {
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }
            if !import.is_wildcard
                && import.imported_name != *target
                && import.alias.as_deref() != Some(target.as_str())
            {
                continue;
            }

            // Manifest-driven check on import namespace.
            if let Some(ctx) = project_ctx {
                for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                    if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&kind) {
                        if manifest.dependencies.iter().any(|group_id| {
                            ns == group_id
                                || ns.starts_with(group_id.as_str())
                                    && ns.as_bytes().get(group_id.len()) == Some(&b'.')
                        }) {
                            return Some(ns.to_string());
                        }
                    }
                }
            }

            if predicates::is_external_scala_namespace(ns, project_ctx) {
                return Some(ns.to_string());
            }
        }

        // Fully-qualified target.
        if predicates::effective_target_is_external(target, project_ctx) {
            return Some(target.clone());
        }

        None
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" => true,
            "protected" => true,
            "private" => &*target.file_path == file_ctx.file_path,
            // `private[pkg]` / `protected[pkg]` — allow (full check needs package graph).
            _ => true,
        }
    }
}
