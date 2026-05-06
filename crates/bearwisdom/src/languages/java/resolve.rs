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


use super::{predicates, type_checker::JavaChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::inheritance;
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

        // Bare-name walker lookup. jdk_src + maven (sources jars) emit real
        // symbols for java.lang types (String, Integer, Object), exception
        // hierarchy, Object methods, Stream / Collection / List APIs, etc.
        // ext:-only filter so chain walker / scope / same-package paths
        // still win for project symbols. Skip when the ref has a chain so
        // the chain walker's receiver-type context wins.
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
                    strategy: "java_synthetic_global",
                    resolved_yield_type: None,
                });
            }
        }

        // Chain-aware resolution: dispatch to JavaChecker.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = JavaChecker.resolve_chain(
                chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
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
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_scope_chain",
                        resolved_yield_type: None,
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
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_same_package",
                        resolved_yield_type: None,
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
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "java_import",
                                resolved_yield_type: None,
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
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "java_wildcard_import",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Step 5: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 6: Inheritance-chain walk for implicit `this` calls.
        //
        // Java and Groovy both allow bare method calls inside a class body —
        // `myMethod()` means `this.myMethod()` and can target a parent class.
        // When Steps 1–5 all miss, walk `inherits_map` upward from the
        // enclosing class (scope_chain[1]) trying `{ancestor}.{target}` at
        // each level (depth ≤ 10 to guard against cycles).
        //
        // Fires for: EdgeKind::Calls, simple (no-dot) name, inside a class.
        if edge_kind == EdgeKind::Calls && !effective_target.contains('.') {
            if let Some(calling_class) =
                inheritance::enclosing_class_from_scope(&ref_ctx.scope_chain)
            {
                if let Some(res) = inheritance::resolve_via_inheritance(
                    calling_class,
                    effective_target,
                    edge_kind,
                    file_ctx,
                    ref_ctx,
                    lookup,
                    predicates::kind_compatible,
                    |fc, rc, sym| self.is_visible(fc, rc, sym),
                    "java_inherited_method",
                ) {
                    return Some(res);
                }
            }
        }

        // Java bare-name fallback. Counterpart to the SCSS / Bash /
        // Python `<lang>_bare_name` resolver steps. Java chain refs
        // through Spring fluent APIs (`mockMvc.perform(...).andExpect(
        // model().attributeHasErrors(...))`), Stream / Optional methods,
        // and AssertJ matchers leave the chain walker without a usable
        // declared type by the leaf segment. The leaf method itself IS
        // in the externals index — it just can't be bound by chain
        // walking alone.
        //
        // Index-wide `by_name` lookup gated by `.java` file path and
        // `kind_compatible`. Cross-language collisions can't leak
        // because the file-extension filter excludes Python / TS /
        // etc. defining identically-named methods.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !effective_target.contains('.')
        {
            for sym in lookup.by_name(effective_target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_java = path.ends_with(".java")
                    || path.ends_with(".jar")
                    || path.starts_with("ext:java:")
                    || path.starts_with("ext:idx:");
                if !is_java {
                    continue;
                }
                // Honor Java visibility — private methods aren't reachable
                // across files even by bare name. `is_visible` runs the
                // same checks as the deterministic resolution paths so a
                // private cross-file method stays unresolved.
                if !self.is_visible(file_ctx, ref_ctx, sym) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "java_bare_name",
                    resolved_yield_type: None,
                });
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
        infer_external_inner(file_ctx, ref_ctx, project_ctx, None)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
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
                let target_pkg = predicates::first_segment(&target.file_path);
                let source_pkg = predicates::first_segment(&file_ctx.file_path);
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

fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        if let Some(ctx) = project_ctx {
            for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&kind) {
                    if manifest.dependencies.iter().any(|group_id| {
                        import_path == group_id
                            || import_path.starts_with(group_id.as_str())
                                && import_path.as_bytes().get(group_id.len()) == Some(&b'.')
                    }) {
                        return Some(import_path.to_string());
                    }
                }
            }
        }
        if predicates::is_external_java_namespace(import_path, project_ctx) {
            return Some(import_path.to_string());
        }
        if let Some(lookup) = lookup {
            // No internal symbols under this fully-qualified namespace
            // → external. Catches transitive deps and stdlib packages
            // not in `is_external_java_namespace`'s known prefix list.
            if !lookup.has_in_namespace(import_path) {
                return Some(import_path.to_string());
            }
        }
        return None;
    }

    for import in &file_ctx.imports {
        let ns = import.module_path.as_deref().unwrap_or("");
        if ns.is_empty() {
            continue;
        }
        if !import.is_wildcard && import.imported_name != *target {
            continue;
        }
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
        if predicates::is_external_java_namespace(ns, project_ctx) {
            return Some(ns.to_string());
        }
        if let Some(lookup) = lookup {
            if !lookup.has_in_namespace(ns) {
                return Some(ns.to_string());
            }
        }
    }

    if predicates::effective_target_is_external(target, project_ctx) {
        return Some(target.clone());
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

