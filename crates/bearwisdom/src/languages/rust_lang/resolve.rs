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


use super::{keywords, predicates, type_checker::RustChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
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
        let file_namespace = predicates::extract_module_path(file);

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
            let enclosing = super::type_checker::find_enclosing_type(&ref_ctx.scope_chain, lookup)?;
            let sym = lookup.by_qualified_name(&enclosing)?;
            if predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "rust_self_type",
                    resolved_yield_type: None,
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
                            resolved_yield_type: None,
                        });
                    }
                }
            }
            // Even if we can't find the exact method, resolve to the type itself.
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "rust_self_type_fallback",
                resolved_yield_type: None,
            });
        }

        // `Self::X` — TypeRef / Calls where the extractor captured the leaf
        // name in `target` and the prefix in `module`. Treat as a member
        // lookup against the enclosing struct/enum/trait. This rescues enum
        // variant references like `Self::Named` / `Self::Tuple` inside `impl`
        // blocks, which the chain walker can't see when the type-ref pass
        // emits a chain-less ref.
        if ref_ctx.extracted_ref.module.as_deref() == Some("Self") {
            if let Some(enclosing) =
                super::type_checker::find_enclosing_type(&ref_ctx.scope_chain, lookup)
            {
                let candidate = format!("{enclosing}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "rust_self_scoped",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Generic type parameters: single uppercase letters in TypeRef position
        // (e.g., `L`, `M`, `F`, `W`) are generic params, never indexable symbols.
        if edge_kind == EdgeKind::TypeRef {
            let bare = target.trim_start_matches("::");
            if bare.len() == 1 && bare.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                return None;
            }
        }

        // Turbofish / generic type argument targets: `<Vec<_>>`, `<MyMessage>`,
        // `<i64, usize>` etc. are extractor noise — type args mis-emitted as Calls.
        // The leading `<` is the reliable marker; bail early.
        if target.starts_with('<') {
            return None;
        }

        // Two-uppercase-letter numeric suffix generics (P1, T2, etc.) are almost
        // always generic type parameters, not real symbols.
        if target.len() == 2 {
            let mut chars = target.chars();
            let (a, b) = (chars.next().unwrap(), chars.next().unwrap());
            if a.is_ascii_uppercase() && b.is_ascii_digit() {
                return None;
            }
        }

        // Rust stdlib builtins are never in our index — fast exit.
        if predicates::is_rust_builtin(target) {
            return None;
        }

        // Chain-aware resolution: dispatch to RustChecker.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = RustChecker.resolve_chain(
                chain_val, edge_kind, None, ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Module-field resolution for qualified call refs (e.g. `DbPool::new()`
        // where the extractor post-pass set module="crate::db").
        // Only fires for Calls and TypeRef — not Imports (handled separately above).
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef) {
            if let Some(module) = &ref_ctx.extracted_ref.module {
                if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
                    if chain_val.segments.len() >= 2 {
                        let type_name = &chain_val.segments[0].name;
                        // "{type_name}.{target}" — standard Rust method storage form.
                        let candidate = format!("{type_name}.{target}");
                        if let Some(sym) = lookup.by_qualified_name(&candidate) {
                            if predicates::kind_compatible(edge_kind, &sym.kind) {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 1.0,
                                    strategy: "rust_ref_module",
                                    resolved_yield_type: None,
                                });
                            }
                        }
                        // "{last_module_segment}.{type_name}.{target}" — module-qualified form.
                        let last_seg = module.rsplit("::").next().unwrap_or(module.as_str());
                        let candidate2 = format!("{last_seg}.{type_name}.{target}");
                        if let Some(sym) = lookup.by_qualified_name(&candidate2) {
                            if predicates::kind_compatible(edge_kind, &sym.kind) {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 1.0,
                                    strategy: "rust_ref_module",
                                    resolved_yield_type: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Normalize `::` separators to `.` for index lookup.
        let normalized = predicates::normalize_path(target);
        let effective_target = &normalized;

        // Step 1: Scope chain walk (innermost → outermost).
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible_with_signature(edge_kind, sym)
                {
                    let strategy = if sym.kind == "variable" {
                        "rust_scope_chain_callable_var"
                    } else {
                        "rust_scope_chain"
                    };
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy,
                        resolved_yield_type: None,
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
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module",
                        resolved_yield_type: None,
                    });
                }
            }

            // By simple name, preferring same module.
            let candidates = lookup.by_name(effective_target);
            for sym in candidates {
                if predicates::sym_module(sym) == module.as_str()
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module_by_name",
                        resolved_yield_type: None,
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
                    let dot_mod = predicates::normalize_path(mod_path);
                    let candidate = format!("{dot_mod}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if self.is_visible(file_ctx, ref_ctx, sym)
                            && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "rust_wildcard_import",
                                resolved_yield_type: None,
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

            let dot_mod = predicates::normalize_path(mod_path);

            // Try {module}.{name} — most common Rust qualified name form.
            let candidate = format!("{dot_mod}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_import",
                        resolved_yield_type: None,
                    });
                }
            }

            // Also try: just the name, scoped to the module prefix.
            // Two variants: with and without leading "crate." since the symbol
            // index never includes the "crate." prefix in qualified names.
            // Require a trailing "." so "models" doesn't match "modelsfoo.User".
            let dot_mod_stripped = dot_mod
                .strip_prefix("crate.")
                .unwrap_or(dot_mod.as_str());
            let dot_mod_prefix = format!("{}.", dot_mod);
            let dot_mod_stripped_prefix = format!("{}.", dot_mod_stripped);
            for sym in lookup.by_name(effective_target) {
                let qn = sym.qualified_name.as_str();
                let prefix_match = qn.starts_with(dot_mod_prefix.as_str())
                    || qn.starts_with(dot_mod_stripped_prefix.as_str());
                if prefix_match
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "rust_import_prefix",
                        resolved_yield_type: None,
                    });
                }
            }

            // Re-export fallback: `pub use submod::Type` means the symbol lives
            // in a submodule of the imported module. Map the module path to a
            // directory segment and find candidates whose file path sits under
            // that directory subtree.
            //
            // "crate::models" → "models/" (forward-slash, any position in path)
            // "crate::api::handlers" → "api/handlers/"
            let dir_suffix: String = {
                let segs: Vec<&str> = dot_mod_stripped
                    .split('.')
                    .filter(|s| !s.is_empty())
                    .collect();
                if segs.is_empty() {
                    String::new()
                } else {
                    format!("{}/", segs.join("/"))
                }
            };
            if !dir_suffix.is_empty() {
                let candidates: Vec<&crate::indexer::resolve::engine::SymbolInfo> =
                    lookup.by_name(effective_target)
                        .iter()
                        .filter(|sym| {
                            // file_path contains the module's directory anywhere in the path,
                            // using forward slashes (the index normalizes to `/`).
                            let fp = sym.file_path.replace('\\', "/");
                            fp.contains(dir_suffix.as_str())
                                && self.is_visible(file_ctx, ref_ctx, sym)
                                && predicates::kind_compatible(edge_kind, &sym.kind)
                        })
                        .collect();
                if candidates.len() == 1 {
                    return Some(Resolution {
                        target_symbol_id: candidates[0].id,
                        confidence: 0.90,
                        strategy: "rust_reexport_dir",
                        resolved_yield_type: None,
                    });
                }
                // Multiple candidates: prefer the one whose qualified_name is shortest
                // (closest to the module root — less nesting = less ambiguity).
                if !candidates.is_empty() {
                    let best = candidates
                        .iter()
                        .min_by_key(|s| s.qualified_name.len())
                        .unwrap();
                    return Some(Resolution {
                        target_symbol_id: best.id,
                        confidence: 0.85,
                        strategy: "rust_reexport_dir_ambiguous",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 4: Crate-qualified resolution.
        // `crate::foo::Bar` or `foo::Bar` with `::` separators.
        if target.contains("::") || effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }

            // Strip leading "crate." and try again.
            let stripped = effective_target
                .strip_prefix("crate.")
                .unwrap_or(effective_target);
            if stripped != effective_target {
                if let Some(sym) = lookup.by_qualified_name(stripped) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "rust_qualified_name_stripped",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Step 5: Global by-name fallback for Calls.
        // Catches bare function references across module boundaries where the
        // caller has no `use` statement — common in test modules that call
        // helpers defined in sibling test modules (e.g. `test_match`, `test_replace`).
        // Only fires when there is exactly one compatible candidate (unambiguous).
        if edge_kind == EdgeKind::Calls {
            let candidates: Vec<&SymbolInfo> = lookup
                .by_name(effective_target)
                .into_iter()
                .filter(|s| predicates::kind_compatible(edge_kind, &s.kind))
                .collect();
            if candidates.len() == 1 {
                return Some(Resolution {
                    target_symbol_id: candidates[0].id,
                    confidence: 0.80,
                    strategy: "rust_global_name_fallback",
                    resolved_yield_type: None,
                });
            }
            // Multiple candidates: prefer the one in the same file as the caller.
            // This resolves cases where each file defines the same helper function
            // locally (e.g. `test_match` in each `crates/language/src/<lang>.rs`).
            let same_file: Vec<&&SymbolInfo> = candidates
                .iter()
                .filter(|s| s.file_path.as_ref() == file_ctx.file_path.as_str())
                .collect();
            if same_file.len() == 1 {
                return Some(Resolution {
                    target_symbol_id: same_file[0].id,
                    confidence: 0.90,
                    strategy: "rust_same_file_name_fallback",
                    resolved_yield_type: None,
                });
            }
            // Multiple candidates: prefer internal (crate-relative qualified names start with
            // known crate root segments, not external crate names from Cargo deps).
            // Use scope_path presence as a proxy for "came from this crate's source".
            let scoped: Vec<&&SymbolInfo> = candidates
                .iter()
                .filter(|s| s.scope_path.is_some())
                .collect();
            if scoped.len() == 1 {
                return Some(Resolution {
                    target_symbol_id: scoped[0].id,
                    confidence: 0.75,
                    strategy: "rust_global_name_scoped",
                    resolved_yield_type: None,
                });
            }
        }

        // Step 6: Global by-name for TypeRef with single unambiguous match.
        // Catches types used without `use` that exist only once in the crate
        // (common for types pulled in through re-exports or cfg-conditional modules).
        if edge_kind == EdgeKind::TypeRef {
            let candidates: Vec<&SymbolInfo> = lookup
                .by_name(effective_target)
                .into_iter()
                .filter(|s| predicates::kind_compatible(edge_kind, &s.kind))
                .collect();
            if candidates.len() == 1 {
                return Some(Resolution {
                    target_symbol_id: candidates[0].id,
                    confidence: 0.75,
                    strategy: "rust_global_typeref_fallback",
                    resolved_yield_type: None,
                });
            }
        }

        // Rust bare-name fallback. Continues the cross-language
        // template (PRs 31, 35-40, Lua, Go). Rust's `use` brings
        // names into scope and trait methods are callable by bare
        // name — both can leak past the deterministic path when
        // type inference can't fully bind. Also fires for
        // `Implements` edges produced by `impl Trait for Type` —
        // those have no chain context and no module-field path
        // when the extractor stored the qualifier elsewhere
        // (`impl std::ops::Deref` produces target_name="Deref"
        // because the resolver post-pass split the path). Gated
        // by `.rs` file extension and `kind_compatible`.
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if matches!(
            edge_kind,
            EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates | EdgeKind::Implements
        )
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains("::")
            && !target.contains('.')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_rust = path.ends_with(".rs") || path.starts_with("ext:rust:");
                if !is_rust {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "rust_bare_name",
                    resolved_yield_type: None,
                });
            }

            // Name-only chain miss: every project resolution path failed,
            // but the target might live in an external Rust file the
            // demand-driven cargo walker hasn't parsed yet (only
            // `scan_rust_header` ran on it). Record a chain miss with
            // empty `current_type` so `expand.rs::locate_via_symbol_index`
            // falls through to its phase-B bare-name probe against the
            // SymbolLocationIndex. If a hit exists, the file is pulled,
            // parsed, and a second resolve pass picks up the symbol via
            // the same bare-name fallback above.
            //
            // Real motivator: `x.into_response()` against axum's
            // `IntoResponse::into_response`. The axum file is walked
            // (`mod`-tree expansion catches it) but never parsed, because
            // the chain walker can't infer the `impl IntoResponse`
            // receiver type and so emits no chain miss. With this
            // recording, the bare name `into_response` is enough to
            // locate the file.
            //
            // Bounded by the location index: only names that exist
            // somewhere external trigger a file pull. Real typos and
            // bare-noise refs cost a hash lookup and bail.
            let trivial = target.len() < 2
                || target.chars().next().map_or(true, |c| c == '_')
                || !target.chars().any(|c| c.is_alphabetic());
            if !trivial {
                lookup.record_chain_miss(
                    crate::indexer::resolve::engine::ChainMiss {
                        current_type: String::new(),
                        target_name: target.clone(),
                    },
                );
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
            "private" => {
                // Private in Rust = visible only within the same module (same file
                // or same module path). Approximate: same file is always ok.
                &*target.file_path == file_ctx.file_path
            }
            "internal" => {
                // pub(crate) / pub(super) — same directory as an approximation.
                let target_dir = predicates::parent_dir(&target.file_path);
                let source_dir = predicates::parent_dir(&file_ctx.file_path);
                target_dir == source_dir || &*target.file_path == file_ctx.file_path
            }
            _ => true, // public
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

        // Import refs: `use serde::Deserialize` → classify by the first crate segment.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            // First segment of a `::` path identifies the crate.
            let first = import_path.split("::").next().unwrap_or(import_path);
            if matches!(first, "crate" | "self" | "super") {
                return None; // internal
            }
            if keywords::STDLIB_CRATES.contains(&first) {
                return Some("std".to_string());
            }
            let name = first;
            // Manifest-driven: check Cargo.toml dependencies first.
            // Crate names may use hyphens in Cargo.toml but underscores in source.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Cargo) {
                    if manifest.dependencies.contains(name)
                        || manifest.dependencies.contains(&name.replace('_', "-"))
                    {
                        return Some(first.to_string());
                    }
                }
            }
            let is_ext = match project_ctx {
                Some(ctx) => is_manifest_rust_crate(ctx, name),
                None => true, // conservative: treat as external
            };
            if is_ext {
                return Some(first.to_string());
            }
            return None;
        }

        // Bare `::`-paths without a matching `use` import (e.g. inline
        // `anyhow::anyhow!()` or `tracing::info!()`): consult Cargo.toml.
        // The manifest is authoritative — replaces the hardcoded extern-crate
        // list that previously lived in `predicates::is_rust_builtin` and
        // misattributed every match to namespace `"std"`.
        if target.contains("::") {
            let first = target.split("::").next().unwrap_or("");
            if !first.is_empty() && !matches!(first, "crate" | "self" | "super") {
                if keywords::STDLIB_CRATES.contains(&first) {
                    return Some("std".to_string());
                }
                if let Some(ctx) = project_ctx {
                    if is_manifest_rust_crate(ctx, first) {
                        return Some(first.to_string());
                    }
                }
            }
        }

        // Builtin calls / stdlib items — always external.
        if predicates::is_rust_builtin(target) {
            return Some("std".to_string());
        }

        // For non-import refs, check if the target came from an external import.
        // Walk the file's import list, matching either:
        //   - the simple target name (`use serde::Deserialize;` → `Deserialize`)
        //   - the first segment of the ref's module path (`fmt::Formatter`
        //     with `use std::fmt;` → `fmt`)
        let normalized = predicates::normalize_path(target);
        let simple = normalized.split('.').next_back().unwrap_or(&normalized);
        let module_root = ref_ctx
            .extracted_ref
            .module
            .as_deref()
            .and_then(|m| m.split("::").next())
            .filter(|s| !s.is_empty());

        for import in &file_ctx.imports {
            if import.imported_name != simple
                && Some(import.imported_name.as_str()) != module_root
            {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            let first = mod_path.split("::").next().unwrap_or(mod_path);
            if matches!(first, "crate" | "self" | "super") {
                continue;
            }
            if keywords::STDLIB_CRATES.contains(&first) {
                return Some("std".to_string());
            }
            let name = first;
            // Manifest-driven check.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Cargo) {
                    if manifest.dependencies.contains(name)
                        || manifest.dependencies.contains(&name.replace('_', "-"))
                    {
                        return Some(first.to_string());
                    }
                }
            }
            let is_ext = match project_ctx {
                Some(ctx) => is_manifest_rust_crate(ctx, name),
                None => true,
            };
            if is_ext {
                return Some(first.to_string());
            }
        }

        // Structural fallback: any walk-imports candidate from a crate
        // the index has no symbols for is external. Catches re-exports
        // (`pub use foo::Bar` where Bar lives in an external crate that
        // isn't in the symbol set) and dev-deps that the manifest pass
        // missed. Only fires when called via the `_with_lookup` variant.
        if let Some(lookup) = lookup {
            for import in &file_ctx.imports {
                if import.imported_name != simple
                    && Some(import.imported_name.as_str()) != module_root
                {
                    continue;
                }
                let Some(ref mod_path) = import.module_path else {
                    continue;
                };
                let first = mod_path.split("::").next().unwrap_or(mod_path);
                if matches!(first, "crate" | "self" | "super") {
                    continue;
                }
                if !lookup.has_in_namespace(first) {
                    return Some(first.to_string());
                }
            }
        }

        // Wildcard-import fallback: `use proptest::prelude::*;` brings
        // arbitrary names into scope. As the last resort, attribute
        // unresolved targets to the first external wildcard-imported
        // crate (manifest-known or has_in_namespace external). Lossy
        // attribution by design — without trait/symbol resolution
        // through external `.rs`, we can't tell which wildcard sourced
        // the name. Still better than `unresolved_refs`.
        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            let first = mod_path.split("::").next().unwrap_or(mod_path);
            if matches!(first, "crate" | "self" | "super") {
                continue;
            }
            if keywords::STDLIB_CRATES.contains(&first) {
                return Some("std".to_string());
            }
            if let Some(ctx) = project_ctx {
                if is_manifest_rust_crate(ctx, first) {
                    return Some(first.to_string());
                }
            }
            if let Some(lookup) = lookup {
                if !lookup.has_in_namespace(first) {
                    return Some(first.to_string());
                }
            }
        }

        // Builder chain propagation: if the ref has a chain and the root segment
        // was imported from an external crate, classify the whole chain external.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if chain_ref.segments.len() >= 2 {
                let root = &chain_ref.segments[0].name;
                for import in &file_ctx.imports {
                    if import.imported_name != root.as_str() {
                        continue;
                    }
                    if let Some(ref mod_path) = import.module_path {
                        let first = mod_path.split("::").next().unwrap_or(mod_path);
                        let is_ext = if matches!(first, "crate" | "self" | "super") {
                            false
                        } else if keywords::STDLIB_CRATES.contains(&first) {
                            true
                        } else {
                            match project_ctx {
                                Some(ctx) => is_manifest_rust_crate(ctx, first),
                                None => true,
                            }
                        };
                        if is_ext {
                            return Some(format!("{}.*", first));
                        }
                    }
                }
            }
        }

    // Final externals-index fallback: if every project import / wildcard /
    // chain check failed, try a by-name lookup. When `target_name` matches
    // one or more symbols defined in external files (`ext:` path prefix),
    // classify the ref as external. Covers common stdlib method calls
    // (`trim_matches`, `to_string_lossy`, `to_path_buf`, `with_context`,
    // `is_ascii_alphanumeric`) that aren't directly imported but are
    // walked in via rust-stdlib / cargo deps.
    //
    // Conservative: only fires when EVERY matching symbol is external.
    // If any match is project-internal, we let resolution fall through
    // and either resolve to that internal symbol or land as unresolved
    // — `trim_matches`-style names occasionally collide with user-defined
    // helpers, and we don't want to misattribute a real bug to "external".
    if let Some(lookup) = lookup {
        let matches = lookup.by_name(target);
        if !matches.is_empty() {
            let all_external = matches
                .iter()
                .all(|s| s.file_path.starts_with("ext:"));
            if all_external {
                // Pick a representative crate from the first match's file path.
                // `ext:rust:.../core/src/str/mod.rs` → `core`. Fall back to
                // "std" if we can't parse it cleanly — the classification
                // matters for grouping, not for chain walking.
                let crate_name = matches
                    .first()
                    .and_then(|s| classify_external_path_as_crate(&s.file_path))
                    .unwrap_or_else(|| "std".to_string());
                return Some(crate_name);
            }
        }
    }

    None
}

/// Extract a representative crate name from an `ext:` virtual path. Used
/// purely for classification; `"std"` is the fallback when the path shape
/// doesn't match a known ecosystem layout.
fn classify_external_path_as_crate(path: &str) -> Option<String> {
    // Common shapes:
    //   ext:rust:<sysroot>/lib/rustlib/src/rust/library/<crate>/...   → <crate>
    //   ext:rust:<cargo-cache>/registry/src/<reg>/<crate>-<ver>/...    → <crate>
    //   ext:rust:<other>                                              → "std" fallback
    let stripped = path.strip_prefix("ext:rust:").unwrap_or(path);
    // Find a `/library/` segment (rust-stdlib).
    if let Some(rest) = stripped
        .split_once("/library/")
        .map(|(_, after)| after)
    {
        if let Some(crate_seg) = rest.split('/').next() {
            if !crate_seg.is_empty() {
                return Some(crate_seg.to_string());
            }
        }
    }
    // Find a `/registry/src/<reg>/<crate>-<ver>/` segment (cargo cache).
    if let Some((_, after)) = stripped.split_once("/registry/src/") {
        // Skip the registry name.
        if let Some((_, after_reg)) = after.split_once('/') {
            if let Some(crate_dir) = after_reg.split('/').next() {
                // `<crate>-<ver>` → crate is everything before the last `-`.
                if let Some(idx) = crate_dir.rfind('-') {
                    let crate_name = &crate_dir[..idx];
                    if !crate_name.is_empty() {
                        return Some(crate_name.to_string());
                    }
                }
                if !crate_dir.is_empty() {
                    return Some(crate_dir.to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a Rust crate name is an external dependency using the Cargo manifest.
fn is_manifest_rust_crate(ctx: &ProjectContext, name: &str) -> bool {
    keywords::STDLIB_CRATES.contains(&name)
        || ctx.has_dependency(ManifestKind::Cargo, name)
        || ctx.has_dependency(ManifestKind::Cargo, &name.replace('_', "-"))
}
