// =============================================================================
// indexer/resolve/rules/java.rs — Java resolution rules
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

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

/// Java language resolver.
pub struct JavaResolver;

impl LanguageResolver for JavaResolver {
    fn language_ids(&self) -> &[&str] {
        &["java"]
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
        if let Some(chain) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = resolve_via_chain(chain, edge_kind, file_ctx, ref_ctx, lookup) {
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
                    && kind_compatible(edge_kind, &sym.kind)
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
                    && kind_compatible(edge_kind, &sym.kind)
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
                        if kind_compatible(edge_kind, &sym.kind) {
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
                    if kind_compatible(edge_kind, &sym.kind) {
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
                if kind_compatible(edge_kind, &sym.kind) {
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
            if is_external_java_namespace(import_path, project_ctx) {
                return Some(import_path.to_string());
            }
            return None;
        }

        // Java builtins (methods always in scope without import).
        if is_java_builtin(target) {
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

            // For wildcard imports: the candidate would be `ns.target`.
            // Either way, check if the namespace is external.
            if is_external_java_namespace(ns, project_ctx) {
                return Some(ns.to_string());
            }
        }

        // Check if the target itself looks like a fully-qualified external name.
        if effective_target_is_external(target, project_ctx) {
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
                let target_pkg = first_segment(&target.file_path);
                let source_pkg = first_segment(&file_ctx.file_path);
                target_pkg == source_pkg
            }
            "protected" => {
                // Accessible from same package or subclasses.
                // Approximate: allow (full check requires inheritance info).
                true
            }
            "private" => {
                // Only visible within the same file (same class declaration).
                target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Chain-aware resolution
// ---------------------------------------------------------------------------

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `this.repo.findById()` with chain `[this, repo, findById]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "com.example.OrderService")
/// 2. `repo` → look up "com.example.OrderService.repo" field → declared_type = "OrderRepo"
/// 3. `findById` → look up "com.example.OrderRepo.findById" → resolved!
fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known class/type? (static access: `ClassName.method()`)
            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "interface" | "enum" | "type_alias"
                )
            });
            if is_type {
                Some(name.clone())
            } else {
                // Is it a field on the enclosing class?
                let mut found = None;
                for scope in &ref_ctx.scope_chain {
                    let field_qname = format!("{scope}.{name}");
                    if let Some(type_name) = lookup.field_type_name(&field_qname) {
                        found = Some(type_name.to_string());
                        break;
                    }
                }
                found.or_else(|| segments[0].declared_type.clone())
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Phase 2: Walk intermediate segments, following field types or return types.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_name(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }
        if let Some(next_type) = lookup.return_type_name(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }

        // Try via import namespaces: {namespace}.{current_type}.{field}
        let mut found = false;
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let qualified_member = format!("{module}.{member_qname}");
                    if let Some(next_type) = lookup.field_type_name(&qualified_member) {
                        current_type = next_type.to_string();
                        found = true;
                        break;
                    }
                    if let Some(next_type) = lookup.return_type_name(&qualified_member) {
                        current_type = next_type.to_string();
                        found = true;
                        break;
                    }
                }
            }
        }
        if found {
            continue;
        }

        return None;
    }

    // Phase 3: Resolve the final segment on the resolved type.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "java_chain_resolution",
            });
        }
    }

    // Try via wildcard imports: {namespace}.{resolved_type}.{method}
    for import in &file_ctx.imports {
        if import.is_wildcard {
            if let Some(module) = &import.module_path {
                let ns_candidate = format!("{module}.{candidate}");
                if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                    if kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "java_chain_resolution",
                        });
                    }
                }
            }
        }
    }

    // Try by name, scoped to the resolved type.
    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "java_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class/interface from the scope chain.
///
/// Java scope_chain: `["com.example.OrderService.create", "com.example.OrderService", "com.example"]`
/// We want "com.example.OrderService".
fn find_enclosing_class(scope_chain: &[String], lookup: &dyn SymbolLookup) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "interface" | "enum") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: second-to-last is often the class.
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check that the edge kind is compatible with the symbol kind.
fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Return the first directory segment of a path — used as a crude package boundary.
fn first_segment(path: &str) -> &str {
    match path.find('/') {
        Some(pos) => &path[..pos],
        None => path,
    }
}

/// Always-external Java namespace roots (stdlib + test frameworks).
const ALWAYS_EXTERNAL: &[&str] = &[
    "java",
    "javax",
    "jakarta",
    "org.junit",
    "sun",
    "com.sun",
];

/// Check whether a Java namespace or import path is external.
fn is_external_java_namespace(ns: &str, project_ctx: Option<&ProjectContext>) -> bool {
    // Always-external first.
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }

    // Check ProjectContext external prefixes (from pom.xml / build.gradle).
    if let Some(ctx) = project_ctx {
        return ctx.is_external_namespace(ns);
    }

    false
}

/// Check whether a target reference that is already fully-qualified looks external.
fn effective_target_is_external(target: &str, project_ctx: Option<&ProjectContext>) -> bool {
    if !target.contains('.') {
        return false;
    }
    is_external_java_namespace(target, project_ctx)
}

/// Java built-in methods always in scope without import (java.lang.*).
fn is_java_builtin(name: &str) -> bool {
    // Extract the object prefix for dotted names like `System.out.println`.
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // java.lang types always visible
        "System" | "String" | "Integer" | "Long" | "Double" | "Float"
            | "Boolean" | "Byte" | "Short" | "Character"
            | "Object" | "Class" | "Enum" | "Record"
            | "Math" | "StrictMath"
            | "StringBuilder" | "StringBuffer"
            | "Thread" | "Runnable"
            | "Exception" | "RuntimeException" | "Error"
            | "Iterable" | "Comparable" | "Cloneable"
            | "AutoCloseable" | "Override" | "Deprecated" | "SuppressWarnings"
            // Pseudo-builtin calls
            | "super" | "this"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "java_tests.rs"]
mod tests;
