// =============================================================================
// indexer/resolve/rules/csharp/mod.rs — C# resolution rules
//
// Scope rules for C# (all versions through C# 13):
//
//   1. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   2. Same-namespace: types in the same namespace are visible without `using`
//   3. Using directives: `using Namespace;` makes all public types visible
//   4. Fully qualified: dotted names resolve directly
//   5. Visibility: public/internal/protected/private enforcement
//
// Adding new C# features:
//   - New syntax that introduces scope (e.g., file-scoped namespaces) →
//     update the extractor in parser/extractors/csharp.rs to emit the
//     correct scope_path, then this resolver handles it automatically.
//   - New import forms (e.g., global using) → add to build_file_context.
// =============================================================================


use super::{predicates, type_checker::CSharpChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::inheritance;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// C# language resolver.
pub struct CSharpResolver;

impl LanguageResolver for CSharpResolver {
    fn language_ids(&self) -> &[&str] {
        &["csharp", "vbnet"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();
        let mut file_namespace = None;

        // Extract namespace from the first Namespace symbol.
        for sym in &file.symbols {
            if sym.kind == crate::types::SymbolKind::Namespace {
                file_namespace = Some(sym.qualified_name.clone());
                break;
            }
        }

        // Inject global usings from the NuGet manifest (SDK implicit + GlobalUsings.cs).
        // These go first so per-file usings can override.
        if let Some(ctx) = project_ctx {
            let global_usings: &[String] = ctx
                .manifest(ManifestKind::NuGet)
                .map(|m| m.global_usings.as_slice())
                .unwrap_or(&[]);
            for ns in global_usings {
                imports.push(ImportEntry {
                    imported_name: ns.clone(),
                    module_path: Some(ns.clone()),
                    alias: None,
                    is_wildcard: true,
                });
            }
        }

        // Extract per-file using directives from refs with EdgeKind::Imports.
        for r in &file.refs {
            if r.kind == EdgeKind::Imports {
                let module = r.module.as_deref().unwrap_or(&r.target_name);
                imports.push(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(module.to_string()),
                    alias: None,
                    // C# `using Namespace;` is a wildcard import — all public types
                    // in that namespace become visible.
                    is_wildcard: module.contains('.'),
                });
            }
        }

        FileContext {
            file_path: file.path.clone(),
            language: "csharp".to_string(),
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

        // Chain-aware resolution: dispatch to CSharpChecker.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = CSharpChecker.resolve_chain(
                chain_ref, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Normalize: strip `this.` prefix for member access on the current class.
        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["NS.Cls.Method", "NS.Cls", "NS"]
        // Try "NS.Cls.Method.Target", "NS.Cls.Target", "NS.Target"
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2: Same-namespace resolution.
        // In C#, types in the same namespace are visible without a `using` directive.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_same_namespace",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 3: Using directive resolution.
        // `using eShop.Catalog.API.Model;` → try "eShop.Catalog.API.Model.{target}"
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let candidate = format!("{module}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if self.is_visible(file_ctx, ref_ctx, sym)
                            && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "csharp_using_directive",
                                resolved_yield_type: None,
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 5: Field type chain resolution.
        // For `db.SelectFrom` (after stripping `this.`), follow the field's type annotation.
        if effective_target.contains('.') {
            if let Some(dot) = effective_target.find('.') {
                let field_name = &effective_target[..dot];
                let rest = &effective_target[dot + 1..];

                for scope in &ref_ctx.scope_chain {
                    let field_qname = format!("{scope}.{field_name}");
                    if let Some(type_name) = lookup.field_type_name(&field_qname) {
                        let candidate = format!("{type_name}.{rest}");
                        if let Some(sym) = lookup.by_qualified_name(&candidate) {
                            if predicates::kind_compatible(edge_kind, &sym.kind) {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 0.95,
                                    strategy: "csharp_field_type_chain",
                                    resolved_yield_type: None,
                                });
                            }
                        }
                        // Try using directives: {namespace}.{TypeName}.{rest}
                        for import in &file_ctx.imports {
                            if import.is_wildcard {
                                if let Some(module) = &import.module_path {
                                    let candidate = format!("{module}.{type_name}.{rest}");
                                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                                            return Some(Resolution {
                                                target_symbol_id: sym.id,
                                                confidence: 0.90,
                                                strategy: "csharp_field_type_chain",
                                                resolved_yield_type: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // Step 6: Inheritance-chain walk for implicit `this` calls.
        //
        // C# allows bare method calls inside a class body — `MyMethod()` is
        // implicitly `this.MyMethod()` and may target a protected/public method
        // on a base class.  When Steps 1–5 all miss, walk the inherits_map
        // upward from the enclosing class (scope_chain[1]) trying
        // `{ancestor}.{target}`.
        //
        // Only fires for Calls edges on simple (no-dot) names inside a class.
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
                    "csharp_inherited_method",
                ) {
                    return Some(res);
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

        // Import refs (e.g., `using System.Linq;`) — classify the using directive
        // itself as external if the namespace is known-external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            // Sibling workspace project — never external. The target namespace
            // (or its root segment) matches another project's declared_name.
            if let Some(ctx) = project_ctx {
                if matches_workspace_project(ctx, target) {
                    return None;
                }
            }
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, target),
                None => predicates::is_external_namespace_fallback(target),
            };
            if external {
                return Some(target.clone());
            }
            return None;
        }

        // Check file's using directives (includes global usings from ProjectContext)
        // for external namespaces. Return the most specific match.
        let mut best: Option<&str> = None;

        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }

            // Sibling workspace projects are not external — skip before
            // the NuGet/system classifier to avoid root-prefix false
            // positives (e.g. `Shared.*` namespace vs. a `Shared.*`
            // NuGet package on the same prefix).
            if let Some(ctx) = project_ctx {
                if matches_workspace_project(ctx, ns) {
                    continue;
                }
            }

            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, ns),
                None => predicates::is_external_namespace_fallback(ns),
            };

            if external {
                if best.is_none() || ns.len() > best.unwrap().len() {
                    best = Some(ns);
                }
            }
        }

        if best.is_some() {
            return best.map(|s| s.to_string());
        }

        // .NET built-ins classify via the engine's keywords() set
        // populated from csharp/keywords.rs; dotnet_stdlib + nuget walkers
        // emit real symbols for the BCL + declared deps.
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
            "internal" => {
                // Approximate: visible if in the same project (same top-level directory).
                // For a proper check we'd need assembly information.
                true
            }
            "protected" => {
                // Approximate: visible if in the same class hierarchy.
                // Full check would require walking the inheritance chain.
                true
            }
            "private" => {
                // Private: only visible within the same file.
                &*target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a namespace refers to a sibling workspace project.
///
/// The `declared_name` for a .NET workspace package is the .csproj / .fsproj /
/// .vbproj filename stem (A2). MSBuild convention maps that stem to the
/// project's root namespace, so a `using Shared.Foo;` in a consumer project
/// targeting `Shared.csproj` resolves via `declared_name = "Shared"`.
///
/// Handles exact matches and nested namespaces: `Shared`, `Shared.Models`,
/// `Shared.Models.Users` all collapse to the `Shared` workspace package by
/// dot-walking from right to left. (`workspace_package_id` on ProjectContext
/// walks `/` separators for TypeScript-style deep imports — .NET namespaces
/// use `.` so we reimplement the walk locally.)
fn matches_workspace_project(ctx: &ProjectContext, namespace: &str) -> bool {
    if ctx.workspace_pkg_by_declared_name.contains_key(namespace) {
        return true;
    }
    let mut path = namespace;
    while let Some(dot) = path.rfind('.') {
        path = &path[..dot];
        if ctx.workspace_pkg_by_declared_name.contains_key(path) {
            return true;
        }
    }
    false
}

/// Check whether a .NET namespace is external, using the NuGet manifest directly.
///
/// Always-external base prefixes (`System`, `Microsoft`) are checked first.
/// NuGet package names are then checked as namespace prefixes and via root-segment
/// matching (e.g., a "Newtonsoft.Json" dep makes any "Newtonsoft.*" namespace external).
fn is_manifest_external_namespace(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if matches!(root, "System" | "Microsoft") {
        return true;
    }
    if let Some(m) = ctx.manifest(ManifestKind::NuGet) {
        if m.dependencies.contains(ns) {
            return true;
        }
        for dep in &m.dependencies {
            if ns.starts_with(dep.as_str())
                && ns.len() > dep.len()
                && ns.as_bytes()[dep.len()] == b'.'
            {
                return true;
            }
            if let Some(dep_root) = dep.split('.').next() {
                if root == dep_root {
                    return true;
                }
            }
        }
        return false;
    }
    false
}

// ---------------------------------------------------------------------------
// Tests are in resolve_tests.rs, declared in mod.rs
