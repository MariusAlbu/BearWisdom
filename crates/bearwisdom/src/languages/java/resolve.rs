// =============================================================================
// indexer/resolve/rules/java/mod.rs — Java resolution rules
//
// Scope rules for Java:
//
//   1. Chain-aware resolution: walk MemberChain following field/return types.
//   2. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   3. Same-package resolution: types in the same package are visible without
//      explicit import (Java package visibility).
//   4. Import resolution: `import com.foo.Bar;` makes Bar directly visible.
//   5. Wildcard import: `import com.foo.*;` makes all types in that package visible.
//   6. Fully qualified names: dotted names resolve directly.
//
// Java import model:
//   The Java extractor emits EdgeKind::Imports refs for import statements:
//     import com.foo.Bar;      → target_name = "Bar",   module = "com.foo.Bar"
//     import com.foo.*;        → target_name = "*",      module = "com.foo"
//
//   Same-package visibility mirrors C# same-namespace: all types in the same
//   package (first N dotted segments of qualified_name) are visible without import.
//
// Adding new Java features:
//   - New import forms (e.g., static imports) → add to build_file_context.
//   - New scope forms → update scope_path in the extractor; scope chain handles them.
// =============================================================================


use super::{builtins, chain};
use crate::indexer::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Java language resolver.
pub struct JavaResolver;

impl LanguageResolver for JavaResolver {
    fn language_ids(&self) -> &[&str] {
        &["java", "groovy"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract the package declaration from symbols.
        // Java extractor emits a Namespace symbol whose qualified_name is the package.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        // Extract per-file import directives from EdgeKind::Imports refs.
        // Java extractor emits:
        //   import com.foo.Bar;   → target_name = "Bar", module = "com.foo.Bar"
        //   import com.foo.*;     → target_name = "*",   module = "com.foo"
        //   import static ...;    → skipped (captured as Calls/TypeRef by extractor)
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module = r.module.as_deref().unwrap_or(&r.target_name);
            let is_wildcard = r.target_name == "*";

            if is_wildcard {
                // `import com.foo.*;` — all public types in the package visible.
                imports.push(ImportEntry {
                    imported_name: String::new(),
                    module_path: Some(module.to_string()),
                    alias: None,
                    is_wildcard: true,
                });
            } else {
                // `import com.foo.Bar;` — exact type import.
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
            language: "java".to_string(),
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

        // Skip import refs themselves — they're not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) =
                chain::resolve_via_chain(chain_val, edge_kind, file_ctx, ref_ctx, lookup)
            {
                return Some(res);
            }
        }

        // Normalize: strip `this.` prefix for member access on the current class.
        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["com.example.MyClass.myMethod", "com.example.MyClass", "com.example"]
        // Try "com.example.MyClass.myMethod.Target", "com.example.MyClass.Target", etc.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-package resolution.
        // In Java, types in the same package are visible without an explicit import.
        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_same_package",
                    });
                }
            }
        }

        // Step 3: Exact import resolution.
        // `import com.foo.Bar;` → target "Bar" resolves to "com.foo.Bar"
        for import in &file_ctx.imports {
            if import.is_wildcard {
                continue;
            }
            if import.imported_name == effective_target {
                if let Some(module) = &import.module_path {
                    if let Some(sym) = lookup.by_qualified_name(module) {
                        if builtins::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "java_import",
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Wildcard import resolution.
        // `import com.foo.*;` → try "com.foo.{target}"
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
                            strategy: "java_wildcard_import",
                        });
                    }
                }
            }
        }

        // Step 5: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_qualified_name",
                    });
                }
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (e.g., `import org.springframework.web.bind.annotation.*;`) —
        // classify the import itself as external if its namespace is known-external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);

            // Manifest-driven: check Maven and Gradle group IDs first.
            // Maven/Gradle group IDs (e.g., "org.springframework") are stored in dependencies.
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

            if builtins::is_external_java_namespace(import_path, project_ctx) {
                return Some(import_path.to_string());
            }
            return None;
        }

        // Java builtins (methods always in scope without import).
        if builtins::is_java_builtin(target) {
            return Some("java.lang".to_string());
        }

        // Check exact import entries for this target name.
        for import in &file_ctx.imports {
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }

            // For exact imports: the target name must match.
            if !import.is_wildcard && import.imported_name != *target {
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

            // For wildcard imports: the candidate would be `ns.target`.
            // Either way, check if the namespace is external.
            if builtins::is_external_java_namespace(ns, project_ctx) {
                return Some(ns.to_string());
            }
        }

        // Check if the target itself looks like a fully-qualified external name.
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
            "public" => true,
            // package-private (no modifier): visible within the same package.
            "package" => {
                // Approximate: same top-level package prefix.
                let target_pkg = builtins::first_segment(&target.file_path);
                let source_pkg = builtins::first_segment(&file_ctx.file_path);
                target_pkg == source_pkg
            }
            "protected" => {
                // Accessible from same package or subclasses.
                // Approximate: allow (full check requires inheritance info).
                true
            }
            "private" => {
                // Only visible within the same file (same class declaration).
                &*target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

