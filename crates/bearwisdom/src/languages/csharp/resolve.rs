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


use super::{builtins, chain};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
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

        // Inject global usings from ProjectContext (SDK implicit + GlobalUsings.cs).
        // These go first so per-file usings can override.
        if let Some(ctx) = project_ctx {
            for ns in &ctx.global_usings {
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

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) =
                chain::resolve_via_chain(chain_ref, edge_kind, file_ctx, ref_ctx, lookup)
            {
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
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_scope_chain",
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
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_same_namespace",
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
                            && builtins::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "csharp_using_directive",
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_qualified_name",
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
                            if builtins::kind_compatible(edge_kind, &sym.kind) {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 0.95,
                                    strategy: "csharp_field_type_chain",
                                });
                            }
                        }
                        // Try using directives: {namespace}.{TypeName}.{rest}
                        for import in &file_ctx.imports {
                            if import.is_wildcard {
                                if let Some(module) = &import.module_path {
                                    let candidate = format!("{module}.{type_name}.{rest}");
                                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                                        if builtins::kind_compatible(edge_kind, &sym.kind) {
                                            return Some(Resolution {
                                                target_symbol_id: sym.id,
                                                confidence: 0.90,
                                                strategy: "csharp_field_type_chain",
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
            let external = match project_ctx {
                Some(ctx) => ctx.is_external_namespace(target),
                None => builtins::is_external_namespace_fallback(target),
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

            let external = match project_ctx {
                Some(ctx) => ctx.is_external_namespace(ns),
                None => builtins::is_external_namespace_fallback(ns),
            };

            if external {
                if best.is_none() || ns.len() > best.unwrap().len() {
                    best = Some(ns);
                }
            }
        }

        best.map(|s| s.to_string())
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
                target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests are in resolve_tests.rs, declared in mod.rs
