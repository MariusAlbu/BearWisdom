// =============================================================================
// indexer/resolve/rules/typescript/mod.rs — TypeScript/JavaScript resolution rules
//
// Scope rules for TypeScript and JavaScript (ES2015+ module system):
//
//   1. Import resolution: refs that carry a `module` field come from import
//      statements. If the module is a relative path (starts with "./", "../"),
//      look up the target symbol in that module's file.
//   2. Same-file resolution: symbols defined in the same file are visible at
//      module scope without any import.
//   3. Scope chain walk: innermost scope → outermost, try {scope}.{target}.
//   4. Fully qualified: dotted names resolve directly.
//
// Key differences from C#:
//   - The TS/JS extractor emits import bindings as `EdgeKind::TypeRef` refs
//     (NOT `EdgeKind::Imports`) with the `module` field set to the raw import
//     specifier string (e.g., `"./utils"`, `"react"`).
//   - Bare specifiers (no "./" prefix) are external packages/builtins.
//   - No file-level namespace — `file_namespace` is always `None`.
//   - `build_file_context` collects import entries from any ref that has
//     a `module` field set (i.e., came from an import statement).
//
// Adding new TS features:
//   - New import syntax → update the extractor (parser/extractors/typescript.rs)
//     to emit the ref with the `module` field set; this resolver picks it up.
//   - New scope forms → update scope_path in the extractor; the scope chain
//     walk handles them automatically.
// =============================================================================

use super::{builtins, chain};

use crate::indexer::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use tracing::debug;

pub use builtins::is_bare_specifier;

/// TypeScript and JavaScript language resolver.
pub struct TypeScriptResolver;

impl LanguageResolver for TypeScriptResolver {
    fn language_ids(&self) -> &[&str] {
        &["typescript", "javascript", "tsx", "jsx"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Collect import entries from any ref that has a `module` field set.
        //
        // The TS/JS parser emits one ref per imported binding, e.g.:
        //   import { useState, useEffect } from 'react'
        //     → ref { target_name: "useState",  module: "react",  kind: TypeRef }
        //     → ref { target_name: "useEffect", module: "react",  kind: TypeRef }
        //
        //   import React from 'react'           (default import)
        //     → ref { target_name: "React",     module: "react",  kind: TypeRef }
        //
        //   import { formatDate } from './utils'
        //     → ref { target_name: "formatDate", module: "./utils", kind: TypeRef }
        //
        // We distinguish external (bare) vs relative by the module specifier.
        // is_wildcard is unused in the TS resolver — all TS imports are explicit.
        for r in &file.refs {
            let Some(module_path) = r.module.clone() else {
                continue;
            };
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: false,
            });
        }

        // TypeScript has no file-level namespace — module identity is the file path.
        FileContext {
            file_path: file.path.clone(),
            language: file.language.clone(),
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

        // Skip EdgeKind::Imports — TS/JS extractor rarely emits these, but be safe.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = chain::resolve_via_chain(chain_ref, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Workspace package lookup — highest priority for bare specifiers.
        // `import { foo } from '@myorg/utils'` where `@myorg/utils` is a
        // sibling workspace package. Scope lookup to that package's
        // symbol set and emit at confidence 1.0. Also handles deep imports
        // like `@myorg/utils/sub/mod` by stripping the trailing path.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if builtins::is_bare_specifier(module) {
                if let Some(res) =
                    resolve_workspace_package(module, target, edge_kind, lookup)
                {
                    return Some(res);
                }
            }
        } else {
            for import in &file_ctx.imports {
                if import.imported_name != *target {
                    continue;
                }
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                if !builtins::is_bare_specifier(module_path) {
                    continue;
                }
                if let Some(res) =
                    resolve_workspace_package(module_path, target, edge_kind, lookup)
                {
                    return Some(res);
                }
            }
        }

        // tsconfig `paths` alias rewrite — before the bare-specifier lookup
        // below tries `in_file(module)`. Lets `@/utils` → `src/utils` resolve
        // through the existing relative-import path.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if let Some(rewritten) =
                lookup.resolve_tsconfig_alias(ref_ctx.file_package_id, module)
            {
                if let Some(res) =
                    resolve_via_alias(&rewritten, target, edge_kind, lookup)
                {
                    return Some(res);
                }
            }
        }

        // If the ref carries a module path, two distinct cases apply:
        //
        // (A) Import-statement refs (no chain): the module is the import source.
        //     If we can't resolve them here, there's nothing more to try — return None.
        //
        // (B) Call refs with a module set by the extractor post-pass (e.g.
        //     `UserService.findOne()` → module="./user.service"): the chain walk
        //     may have failed, but we can still look up the target directly in
        //     the source module before falling through to the scope chain walk.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            // NOTE: Historically this short-circuited on bare specifiers
            // (`react`, `@tanstack/react-query`) because externals weren't
            // indexed. With S5 externals wired in, package source lives in
            // the index under `ext:ts:{pkg}` files and external symbols are
            // qualified with the package name. Fall through to the normal
            // lookups so they can match — if they don't, Tier 1.5 still
            // routes to `ext:{module}` as before.

            // Relative import: look up in the target file by simple name.
            // Use per-source resolution so `./utils` gets the correct file
            // for THIS source rather than whoever resolved it first.
            for sym in lookup.in_module_from(&file_ctx.file_path, module) {
                if sym.name == *target && builtins::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_import_file",
                        file = %module,
                        target = %target,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_import_file",
                    });
                }
            }

            // Also try {module}.{target} as a qualified name (parser may use this form).
            let candidate = format!("{module}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_import",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_import",
                    });
                }
            }

            // Neither direct lookup found anything — the module may be a barrel
            // file that re-exports the symbol from a deeper module.  Follow the
            // re-export chain up to 5 hops.
            if let Some(res) = follow_reexports(module, target, edge_kind, lookup, 0) {
                return Some(res);
            }

            // Case (A): import-statement ref (no chain) — couldn't resolve, stop here.
            // Case (B): call ref with extractor-set module — fall through to scope walk.
            if ref_ctx.extracted_ref.chain.is_none() {
                return None;
            }
            // Fall through — scope chain walk below may still resolve it.
        }

        // Non-import resolution path. Covers:
        //   - Refs with no module field at all.
        //   - Case (B) call refs whose module-based lookup above didn't resolve.

        // Imports-based qualified-name lookup. If the target matches a bare-
        // specifier import, try `{import_module}.{target}` in the index —
        // external packages indexed via S5 (`indexer::externals`) rewrite
        // their symbol qualified_names with the package prefix, so this
        // matches directly for packages in `ext:ts:` files.
        for import in &file_ctx.imports {
            if import.imported_name != *target {
                continue;
            }
            let Some(module_path) = &import.module_path else {
                continue;
            };

            // Relative import (`./x`, `../y`): look up in the target file
            // via the per-source module index. Without this, JSX usage refs
            // like `<Button>` after `import { Button } from "./button"`
            // fall through to the heuristic when the TS resolver should
            // have caught them deterministically.
            if !builtins::is_bare_specifier(module_path) {
                for sym in lookup.in_module_from(&file_ctx.file_path, module_path) {
                    if sym.name == *target
                        && builtins::kind_compatible(edge_kind, &sym.kind)
                    {
                        debug!(
                            strategy = "ts_relative_import",
                            module = %module_path,
                            target = %target,
                            "resolved"
                        );
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "ts_relative_import",
                        });
                    }
                }
                // Follow barrel re-exports when the relative module itself
                // is a barrel that forwards the named export. The
                // per-source resolved path makes this work even when many
                // files share the same `./` specifier.
                let resolved_path = lookup
                    .resolve_module_from(&file_ctx.file_path, module_path)
                    .map(|s| s.to_string());
                if let Some(path) = resolved_path.as_deref() {
                    if let Some(res) =
                        follow_reexports(path, target, edge_kind, lookup, 0)
                    {
                        return Some(res);
                    }
                }
                // Fallback: try the spec itself in case follow_reexports
                // can pick it up via its own module_to_file lookup.
                if let Some(res) =
                    follow_reexports(module_path, target, edge_kind, lookup, 0)
                {
                    return Some(res);
                }
                // Relative import exists but the symbol is not in the
                // referenced file (or any re-export chain). Stop here so
                // the heuristic doesn't produce a spurious match.
                return None;
            }

            let candidate = format!("{module_path}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_bare_import_qname",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_bare_import_qname",
                    });
                }
            }
            // tsconfig `paths` alias: the import specifier may be a
            // package-relative alias (`@/components/x` → `apps/landing/src/components/x`).
            // Try the rewrite before bailing out — without this, JSX usage
            // refs (whose own `module` field is None) fall through to the
            // heuristic when an alias rewrite would have resolved them.
            if let Some(rewritten) =
                lookup.resolve_tsconfig_alias(ref_ctx.file_package_id, module_path)
            {
                if let Some(res) =
                    resolve_via_alias(&rewritten, target, edge_kind, lookup)
                {
                    return Some(res);
                }
            }
            // Import exists but symbol not in index — it's external and
            // uncovered. Stop trying so the heuristic doesn't produce a
            // spurious match on a same-named internal symbol.
            return None;
        }

        // Normalize: strip `this.` prefix for member access on the current class.
        // `this.buildUserRO` → `buildUserRO`, then scope chain resolves it.
        // `this.db.selectFrom` → `db.selectFrom` (still a chain, handled later).
        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["MyClass.method", "MyClass"]
        // Try "MyClass.method.target", "MyClass.target"
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_scope_chain",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        // In TS/JS, symbols in the same file are visible at module scope.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && builtins::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_same_file",
                    qualified_name = %sym.qualified_name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_same_file",
                });
            }
        }

        // Step 3: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_qualified_name",
                        target = %target,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_qualified_name",
                    });
                }
            }
        }

        // Step 4: Field type chain resolution.
        // For `db.selectFrom` (after stripping `this.`), split into field + rest,
        // find the field's type annotation, then look up the method on that type.
        if let Some(dot) = effective_target.find('.') {
            let field_name = &effective_target[..dot];
            let rest = &effective_target[dot + 1..];

            // Try to find the field as a property on enclosing scopes.
            for scope in &ref_ctx.scope_chain {
                let field_qname = format!("{scope}.{field_name}");
                if let Some(type_name) = lookup.field_type_name(&field_qname) {
                    // Found field type. Try {TypeName}.{rest} in the index.
                    let candidate = format!("{type_name}.{rest}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if builtins::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.95,
                                strategy: "ts_field_type_chain",
                            });
                        }
                    }

                    // Also try: the type might be in a namespace, search by name.
                    let method_name = rest.split('.').next().unwrap_or(rest);
                    for sym in lookup.by_name(method_name) {
                        if sym.qualified_name.starts_with(type_name)
                            && builtins::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "ts_field_type_chain",
                            });
                        }
                    }

                    // Type is known but method isn't in our index — it's on the type.
                    // Don't fall through; let infer_external_namespace handle it.
                    break;
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

        // Browser/JS runtime globals — always external.
        if builtins::is_js_runtime_global(target) {
            return Some("runtime".to_string());
        }

        // If the ref itself carries a module path, check it directly.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if builtins::is_bare_specifier(module) {
                // Workspace package: not external. The resolver's main path
                // already handles this at confidence 1.0 — here we just
                // prevent the fallback from reclassifying it as external
                // when the specific symbol wasn't found in the package.
                if let Some(ctx) = project_ctx {
                    if ctx.workspace_package_id(module).is_some() {
                        return None;
                    }
                }
                // Manifest-driven: check package.json dependencies first.
                if let Some(ctx) = project_ctx {
                    if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Npm) {
                        if is_npm_package_match(module, &manifest.dependencies) {
                            return Some(module.clone());
                        }
                    }
                }
                let is_external = match project_ctx {
                    Some(ctx) => is_manifest_ts_package(ctx, ref_ctx.file_package_id, module),
                    // Without ProjectContext, treat all bare specifiers as external.
                    None => true,
                };
                if is_external {
                    return Some(module.clone());
                }
            }
            // Relative import with a module — not external.
            return None;
        }

        // No module on the ref — check the file's import list for this target.
        // If the name was imported from a bare specifier, it's external.
        for import in &file_ctx.imports {
            if import.imported_name != *target {
                continue;
            }
            let Some(module_path) = &import.module_path else {
                continue;
            };
            if !builtins::is_bare_specifier(module_path) {
                continue;
            }
            // Workspace package — not external; let the main resolver path own it.
            if let Some(ctx) = project_ctx {
                if ctx.workspace_package_id(module_path).is_some() {
                    return None;
                }
            }
            // Manifest-driven: check package.json dependencies first.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Npm) {
                    if is_npm_package_match(module_path, &manifest.dependencies) {
                        return Some(module_path.clone());
                    }
                }
            }
            let is_external = match project_ctx {
                Some(ctx) => is_manifest_ts_package(ctx, ref_ctx.file_package_id, module_path),
                None => true,
            };
            if is_external {
                return Some(module_path.clone());
            }
        }

        // Builder chain propagation: if the ref has a chain and the root segment
        // was imported from an external package, classify the whole chain external.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if chain_ref.segments.len() >= 2 {
                let root = &chain_ref.segments[0].name;
                // Check if root was imported from a bare (external) specifier.
                for import in &file_ctx.imports {
                    if import.imported_name != *root {
                        continue;
                    }
                    if let Some(module_path) = &import.module_path {
                        if builtins::is_bare_specifier(module_path) {
                            if let Some(ctx) = project_ctx {
                                if ctx.workspace_package_id(module_path).is_some() {
                                    return None;
                                }
                            }
                            let is_external = match project_ctx {
                                Some(ctx) => is_manifest_ts_package(ctx, ref_ctx.file_package_id, module_path),
                                None => true,
                            };
                            if is_external {
                                return Some(format!("{}.*", module_path));
                            }
                        }
                    }
                }
            }
        }

        // Last resort: common built-in method names that appear on Array, String,
        // Promise, and Object instances. Only classify when we have no other
        // information — all import-based checks have already failed above.
        if builtins::is_common_builtin_method(target) {
            return Some("runtime".to_string());
        }

        None
    }

    // is_visible: default implementation (always true) is correct for TS.
    // TypeScript's `export` keyword controls visibility, but for resolution
    // purposes we treat all indexed symbols as accessible.
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Resolve an import after rewriting the specifier through tsconfig
/// `paths` aliases. The rewritten value is a bare project-relative path
/// stem (e.g. `src/utils`), matched via `in_file` against both exact paths
/// and common TS extensions.
fn resolve_via_alias(
    rewritten: &str,
    target: &str,
    edge_kind: crate::types::EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    // Try the rewritten path directly first — this catches
    // `module_to_file` hits populated by the TS ecosystem resolver.
    for sym in lookup.in_file(rewritten) {
        if sym.name == *target && builtins::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "ts_tsconfig_alias",
            });
        }
    }

    // Common TS/JS file extensions — the rewritten stem usually omits them.
    const EXTS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];

    // Two-pass strategy: first try direct own-symbol matches across every
    // candidate file shape (bare + ext + /index+ext). Only if none of those
    // hit, follow re-export chains — barrel files (`export { X } from './y'`)
    // and single-line re-exports (`export { X } from 'pkg'`) never carry
    // own symbols, so an in_file miss doesn't mean the symbol isn't there.
    let candidates: Vec<String> = {
        let mut v = vec![rewritten.to_string()];
        for ext in EXTS {
            v.push(format!("{rewritten}{ext}"));
            v.push(format!("{rewritten}/index{ext}"));
        }
        v
    };

    for candidate in &candidates {
        for sym in lookup.in_file(candidate) {
            if sym.name == *target && builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_tsconfig_alias",
                });
            }
        }
    }

    // Own-symbol miss — walk re-export chains from every plausible path
    // shape. Follows `export { X } from './y'` and `export * from './z'`
    // up to the existing 5-hop depth limit.
    for candidate in &candidates {
        if let Some(res) = follow_reexports(candidate, target, edge_kind, lookup, 0) {
            return Some(res);
        }
    }

    None
}

/// Resolve an import that targets a sibling workspace package.
///
/// When `module_specifier` matches a package's `declared_name` (exact or by
/// prefix for deep imports like `@myorg/utils/sub/mod`), scope the symbol
/// lookup to that package and return a confidence-1.0 resolution.
///
/// For deep imports we prefer a symbol whose file path contains the import's
/// sub-path, falling back to the first kind-compatible same-name symbol in
/// the package. That keeps multi-file workspace packages resolving correctly
/// without needing to map each sub-path to an exact file.
fn resolve_workspace_package(
    module_specifier: &str,
    target: &str,
    edge_kind: crate::types::EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let pkg_id = lookup.workspace_package_id(module_specifier)?;

    // If this was a deep import, compute the sub-path so we can prefer a
    // matching file. For an exact match the sub-path is empty.
    let sub_path = sub_path_for_deep_import(module_specifier, lookup);

    let syms = lookup.symbols_in_package(pkg_id);
    let mut fallback: Option<&SymbolInfo> = None;
    for sym in syms {
        if sym.name != target {
            continue;
        }
        if !builtins::kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        if let Some(sub) = &sub_path {
            if sym.file_path.contains(sub.as_str()) {
                debug!(
                    strategy = "ts_workspace_pkg",
                    module = %module_specifier,
                    target = %target,
                    sub = %sub,
                    "resolved (deep import)"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_workspace_pkg",
                });
            }
        }
        if fallback.is_none() {
            fallback = Some(sym);
        }
    }

    fallback.map(|sym| {
        debug!(
            strategy = "ts_workspace_pkg",
            module = %module_specifier,
            target = %target,
            "resolved"
        );
        Resolution {
            target_symbol_id: sym.id,
            confidence: 1.0,
            strategy: "ts_workspace_pkg",
        }
    })
}

/// Compute the sub-path portion of a deep workspace import.
///
/// Finds the longest declared_name prefix (exact match) and returns the
/// remainder after the boundary `/`. Returns `None` when `specifier` is
/// itself the declared_name (no deep path) or when no workspace package
/// matches.
fn sub_path_for_deep_import(specifier: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    if lookup.is_workspace_declared_name(specifier) {
        return None;
    }
    let mut path = specifier;
    while let Some(slash) = path.rfind('/') {
        path = &path[..slash];
        if lookup.is_workspace_declared_name(path) {
            return Some(specifier[path.len() + 1..].to_string());
        }
    }
    None
}

/// Follow re-export chains through barrel files.
///
/// When `in_file(module_path)` returns no match for `target_name`, this
/// function checks whether `module_path` is a barrel file that re-exports
/// the symbol from another module — and recurses until the definition is
/// found or the depth limit is reached.
///
/// Handles:
///   `export { X } from './y'`   — named re-export; follow to `./y`
///   `export { X as Z } from './y'` — aliased; the stored `target_name` is the
///                                    *original* name (before `as`), matching
///                                    what we're looking for in the source file
///   `export * from './y'`       — wildcard; try `target_name` in every
///                                 wildcard source module
fn follow_reexports(
    module_path: &str,
    target_name: &str,
    edge_kind: crate::types::EdgeKind,
    lookup: &dyn SymbolLookup,
    depth: u32,
) -> Option<Resolution> {
    const MAX_DEPTH: u32 = 5;
    if depth >= MAX_DEPTH {
        return None;
    }

    let reexports = lookup.reexports_from(module_path);
    if reexports.is_empty() {
        return None;
    }

    // Collect wildcard sources separately — they are tried only when no named
    // re-export matched, to avoid false positives from `export * from`.
    let mut wildcard_sources: Vec<&str> = Vec::new();

    for (exported_name, source_module) in reexports {
        if builtins::is_bare_specifier(source_module) {
            continue;
        }

        if exported_name == "*" {
            wildcard_sources.push(source_module.as_str());
            continue;
        }

        if exported_name != target_name {
            continue;
        }

        // Named match: look up `target_name` in `source_module` from the
        // CONTAINING barrel's perspective. `./quick-create-button` inside
        // `apps/web/.../index.ts` resolves to a different file than the
        // same spec from elsewhere — per-source resolution is mandatory.
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && builtins::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_reexport_chain",
                    via = %module_path,
                    source = %source_module,
                    target = %target_name,
                    depth = depth,
                    "resolved via re-export"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_reexport_chain",
                });
            }
        }

        // Not directly in `source_module` — recurse (it may itself be a barrel).
        // Prefer the resolved file path so the next hop's reexports_from
        // lookup hits its file-keyed map directly.
        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) = follow_reexports(next_path, target_name, edge_kind, lookup, depth + 1) {
            return Some(res);
        }
    }

    // No named match. Try wildcard sources in order.
    for source_module in wildcard_sources {
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && builtins::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_reexport_star",
                    via = %module_path,
                    source = %source_module,
                    target = %target_name,
                    depth = depth,
                    "resolved via export-star"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "ts_reexport_star",
                });
            }
        }

        // Recurse into wildcard sources too — chase via resolved file path.
        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) = follow_reexports(next_path, target_name, edge_kind, lookup, depth + 1) {
            return Some(res);
        }
    }

    None
}

/// Check whether a bare module specifier is an external npm package or Node.js built-in,
/// using the project manifest (package.json) directly.
///
/// M2: scoped to the source file's `package_id` when available so a package
/// that doesn't declare a dep in its own package.json doesn't inherit it
/// from a sibling workspace package.
pub(crate) fn is_manifest_ts_package(
    ctx: &ProjectContext,
    package_id: Option<i64>,
    specifier: &str,
) -> bool {
    if specifier.starts_with("node:") {
        return true;
    }
    if let Some(m) = ctx.manifests_for(package_id).get(&ManifestKind::Npm) {
        let deps = &m.dependencies;
        if deps.contains(specifier) {
            return true;
        }
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if deps.contains(path) {
                return true;
            }
        }
        return false;
    }
    false
}

/// Check whether a bare module specifier matches any npm package in the manifest.
///
/// Handles exact matches and deep import paths:
///   `"react"` → matches `"react"` in dependencies.
///   `"@tanstack/react-query"` → matches `"@tanstack/react-query"`.
///   `"react-dom/client"` → matches `"react-dom"` after stripping the subpath.
fn is_npm_package_match(
    specifier: &str,
    deps: &std::collections::HashSet<String>,
) -> bool {
    if deps.contains(specifier) {
        return true;
    }
    // Deep import path: strip trailing subpath segments until a match is found.
    let mut path = specifier;
    while let Some(slash) = path.rfind('/') {
        path = &path[..slash];
        if deps.contains(path) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
