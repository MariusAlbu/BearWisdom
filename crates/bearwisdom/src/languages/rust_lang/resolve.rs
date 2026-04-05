// =============================================================================
// indexer/resolve/rules/rust_lang/mod.rs — Rust resolution rules
//
// Scope rules for Rust:
//
//   1. Chain-aware resolution: walk MemberChain step-by-step following
//      field types / return types.
//   2. Scope chain walk: innermost → outermost, try {scope}.{target}.
//   3. Same-module resolution: symbols whose qualified_name shares the same
//      module path are visible without a `use` statement.
//   4. Import-based resolution: `use foo::bar::Baz` makes `Baz` visible.
//   5. Crate-qualified: `foo::bar::Baz` with `::` separators → convert to
//      dot form and resolve directly.
//
// Rust visibility:
//   `pub`      → Public  (visible everywhere)
//   `pub(crate)` → Internal (visible within the crate)
//   `pub(super)` → similar to Internal; approximated as "same dir"
//   (none)     → Private (visible in the same module only)
//
// Import format from the Rust extractor:
//   For `use serde::Deserialize;`:
//     target_name = "Deserialize", module = "serde"
//   For `use crate::models::User;`:
//     target_name = "User",        module = "crate::models"
//   EdgeKind::Imports is used for use-declarations.
//
// Call/type-ref format:
//   `User::new()` → target_name = "User::new" or "new", module = None
//   `SomeStruct`  → target_name = "SomeStruct",            module = None
//
// Key constraint: The extractor may represent `::` paths either as
// `target_name` with embedded `::` OR as a simple name with a `module`
// field. Both forms are handled here.
// =============================================================================


use super::{builtins, chain};
use crate::indexer::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Rust language resolver.
pub struct RustResolver;

impl LanguageResolver for RustResolver {
    fn language_ids(&self) -> &[&str] {
        &["rust"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Derive the module path for this file. The Rust extractor sets
        // scope_path on top-level symbols to reflect the module path
        // (e.g., "crate::models" for a symbol in src/models.rs).
        // We take it from the first top-level symbol's scope_path.
        let file_namespace = builtins::extract_module_path(file);

        // Build import entries from EdgeKind::Imports refs.
        // The Rust extractor emits one ref per `use` item brought into scope:
        //   use serde::Deserialize;
        //     → ref { target_name: "Deserialize", module: Some("serde"), kind: Imports }
        //   use crate::models::User;
        //     → ref { target_name: "User", module: Some("crate::models"), kind: Imports }
        //   use std::collections::HashMap;
        //     → ref { target_name: "HashMap", module: Some("std::collections"), kind: Imports }
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }

            let module_path = r.module.clone().or_else(|| {
                // If no module field, try splitting target_name on "::"
                // e.g., target_name = "serde::Deserialize"
                if r.target_name.contains("::") {
                    let (mod_part, _name) = r.target_name.rsplit_once("::")?;
                    Some(mod_part.to_string())
                } else {
                    None
                }
            });

            // The imported name is the last segment of the path.
            let imported_name = if r.target_name.contains("::") {
                r.target_name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&r.target_name)
                    .to_string()
            } else {
                r.target_name.clone()
            };

            // Wildcard import: `use foo::bar::*`
            let is_wildcard = imported_name == "*";

            imports.push(ImportEntry {
                imported_name,
                module_path,
                alias: None,
                is_wildcard,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "rust".to_string(),
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

        // Skip import refs — they declare scope, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // `Self` → resolve to the enclosing struct/enum/trait.
        if target == "Self" {
            let enclosing = chain::find_enclosing_type(&ref_ctx.scope_chain, lookup)?;
            let sym = lookup.by_qualified_name(&enclosing)?;
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "rust_self_type",
                });
            }
            // For Calls to Self (e.g. Self::new()), look for a method
            // on the enclosing type.
            if edge_kind == EdgeKind::Calls {
                // Try to find a constructor or associated function.
                for child in lookup.in_namespace(&enclosing) {
                    if child.name == "new"
                        && matches!(child.kind.as_str(), "method" | "function" | "constructor")
                    {
                        return Some(Resolution {
                            target_symbol_id: child.id,
                            confidence: 0.95,
                            strategy: "rust_self_constructor",
                        });
                    }
                }
            }
            // Even if we can't find the exact method, resolve to the type itself.
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "rust_self_type_fallback",
            });
        }

        // Rust stdlib builtins are never in our index — fast exit.
        if builtins::is_rust_builtin(target) {
            return None;
        }

        // Chain-aware resolution.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = chain::resolve_via_chain(chain_val, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Normalize `::` separators to `.` for index lookup.
        let normalized = builtins::normalize_path(target);
        let effective_target = &normalized;

        // Step 1: Scope chain walk (innermost → outermost).
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-module resolution.
        // Symbols in the same module are visible without `use`.
        if let Some(module) = &file_ctx.file_namespace {
            let candidate = format!("{module}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module",
                    });
                }
            }

            // By simple name, preferring same module.
            let candidates = lookup.by_name(effective_target);
            for sym in candidates {
                if builtins::sym_module(sym) == module.as_str()
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module_by_name",
                    });
                }
            }
        }

        // Step 3: Import-based resolution.
        // `use foo::bar::Baz` → look for `Baz` in the symbol index,
        // preferring symbols whose qualified_name starts with the import's module path.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                // Wildcard: find anything in the imported module matching the name.
                if let Some(ref mod_path) = import.module_path {
                    let dot_mod = builtins::normalize_path(mod_path);
                    let candidate = format!("{dot_mod}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if self.is_visible(file_ctx, ref_ctx, sym)
                            && builtins::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "rust_wildcard_import",
                            });
                        }
                    }
                }
                continue;
            }

            // Named import: the imported_name must match.
            if import.imported_name != *effective_target {
                continue;
            }

            let Some(ref mod_path) = import.module_path else {
                continue;
            };

            let dot_mod = builtins::normalize_path(mod_path);

            // Try {module}.{name} — most common Rust qualified name form.
            let candidate = format!("{dot_mod}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_import",
                    });
                }
            }

            // Also try: just the name, scoped to the module prefix.
            for sym in lookup.by_name(effective_target) {
                if sym.qualified_name.starts_with(dot_mod.as_str())
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "rust_import_prefix",
                    });
                }
            }
        }

        // Step 4: Crate-qualified resolution.
        // `crate::foo::Bar` or `foo::Bar` with `::` separators.
        if target.contains("::") || effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_qualified_name",
                    });
                }
            }

            // Strip leading "crate." and try again.
            let stripped = effective_target
                .strip_prefix("crate.")
                .unwrap_or(effective_target);
            if stripped != effective_target {
                if let Some(sym) = lookup.by_qualified_name(stripped) {
                    if builtins::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "rust_qualified_name_stripped",
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

        // Import refs: `use serde::Deserialize` → classify by the first crate segment.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            // First segment of a `::` path identifies the crate.
            let first = import_path.split("::").next().unwrap_or(import_path);
            match first {
                "crate" | "self" | "super" => return None, // internal
                "std" | "core" | "alloc" => return Some("std".to_string()),
                name => {
                    // Manifest-driven: check Cargo.toml dependencies first.
                    // Crate names may use hyphens in Cargo.toml but underscores in source.
                    if let Some(ctx) = project_ctx {
                        if let Some(manifest) = ctx.manifests.get(&ManifestKind::Cargo) {
                            if manifest.dependencies.contains(name)
                                || manifest.dependencies.contains(&name.replace('_', "-"))
                            {
                                return Some(first.to_string());
                            }
                        }
                    }
                    let is_ext = match project_ctx {
                        Some(ctx) => ctx.is_external_rust_crate(name),
                        None => true, // conservative: treat as external
                    };
                    if is_ext {
                        return Some(first.to_string());
                    }
                    return None;
                }
            }
        }

        // Builtin calls / stdlib items — always external.
        if builtins::is_rust_builtin(target) {
            return Some("std".to_string());
        }

        // For non-import refs, check if the target came from an external import.
        // Walk the file's import list for a matching imported_name.
        let normalized = builtins::normalize_path(target);
        let simple = normalized.split('.').next_back().unwrap_or(&normalized);

        for import in &file_ctx.imports {
            if import.imported_name != simple {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            let first = mod_path.split("::").next().unwrap_or(mod_path);
            match first {
                "crate" | "self" | "super" => continue,
                "std" | "core" | "alloc" => return Some("std".to_string()),
                name => {
                    // Manifest-driven check.
                    if let Some(ctx) = project_ctx {
                        if let Some(manifest) = ctx.manifests.get(&ManifestKind::Cargo) {
                            if manifest.dependencies.contains(name)
                                || manifest.dependencies.contains(&name.replace('_', "-"))
                            {
                                return Some(first.to_string());
                            }
                        }
                    }
                    let is_ext = match project_ctx {
                        Some(ctx) => ctx.is_external_rust_crate(name),
                        None => true,
                    };
                    if is_ext {
                        return Some(first.to_string());
                    }
                }
            }
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
            "private" => {
                // Private in Rust = visible only within the same module (same file
                // or same module path). Approximate: same file is always ok.
                target.file_path == file_ctx.file_path
            }
            "internal" => {
                // pub(crate) / pub(super) — same directory as an approximation.
                let target_dir = builtins::parent_dir(&target.file_path);
                let source_dir = builtins::parent_dir(&file_ctx.file_path);
                target_dir == source_dir || target.file_path == file_ctx.file_path
            }
            _ => true, // public
        }
    }
}
