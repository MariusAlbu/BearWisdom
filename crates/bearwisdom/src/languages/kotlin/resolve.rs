// =============================================================================
// indexer/resolve/rules/kotlin/mod.rs — Kotlin resolution rules
//
// Scope rules for Kotlin:
//
//   1. Scope chain walk: innermost scope → outermost, try {scope}.{target}.
//   2. Same-package resolution: types in the same package are visible without
//      an explicit import (mirrors Java package visibility).
//   3. Exact import resolution: `import com.foo.Bar` → Bar directly visible.
//   4. Wildcard import: `import com.foo.*` → all types in that package visible.
//   5. Fully qualified names: dotted names resolve directly.
//
// Kotlin import model:
//   The Kotlin extractor emits EdgeKind::Imports refs for import statements:
//     import com.foo.Bar    → target_name = "Bar",  module = "com.foo.Bar"
//     import com.foo.*      → target_name = "*",    module = "com.foo"
// =============================================================================


use super::builtins;
use crate::indexer::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Kotlin language resolver.
pub struct KotlinResolver;

impl LanguageResolver for KotlinResolver {
    fn language_ids(&self) -> &[&str] {
        &["kotlin"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract the package declaration from Namespace symbols.
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
            let is_wildcard = r.target_name == "*";

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
            language: "kotlin".to_string(),
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

        // Kotlin builtins are never in the project index.
        if builtins::is_kotlin_builtin(target) {
            return None;
        }

        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "kotlin_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-package resolution.
        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "kotlin_same_package",
                    });
                }
            }
        }

        // Step 3: Exact import resolution.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                continue;
            }
            // Check both imported_name and alias.
            let name_match = import.imported_name == effective_target
                || import.alias.as_deref() == Some(effective_target);
            if !name_match {
                continue;
            }
            if let Some(module) = &import.module_path {
                if let Some(sym) = lookup.by_qualified_name(module) {
                    if builtins::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "kotlin_import",
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
                    if builtins::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "kotlin_wildcard_import",
                        });
                    }
                }
            }
        }

        // Step 5: Fully qualified name.
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "kotlin_qualified_name",
                    });
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

        // Import refs — classify the import path itself.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);

            // Manifest-driven: check Maven and Gradle group IDs first.
            if let Some(ctx) = project_ctx {
                for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                    if let Some(manifest) = ctx.manifests.get(&kind) {
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

            if builtins::is_external_kotlin_namespace(import_path, project_ctx) {
                return Some(import_path.to_string());
            }
            return None;
        }

        // Kotlin builtins.
        if builtins::is_kotlin_builtin(target) {
            return Some("kotlin.stdlib".to_string());
        }

        // Walk imports for a match on this target name.
        for import in &file_ctx.imports {
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }
            if !import.is_wildcard && import.imported_name != *target
                && import.alias.as_deref() != Some(target.as_str())
            {
                continue;
            }

            // Manifest-driven check on import namespace.
            if let Some(ctx) = project_ctx {
                for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                    if let Some(manifest) = ctx.manifests.get(&kind) {
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

            if builtins::is_external_kotlin_namespace(ns, project_ctx) {
                return Some(ns.to_string());
            }
        }

        // Fully-qualified target.
        if builtins::effective_target_is_external(target, project_ctx) {
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
            "public" | "internal" => true,
            "protected" => true, // allow — full check needs inheritance info
            "private" => target.file_path == file_ctx.file_path,
            _ => true,
        }
    }
}
