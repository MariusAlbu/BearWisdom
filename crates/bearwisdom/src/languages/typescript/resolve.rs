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

use super::{predicates, type_checker::TypeScriptChecker};
use crate::type_checker::TypeChecker;

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use tracing::debug;

pub use predicates::is_bare_specifier;

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
        // step-by-step following field types. Dispatch to the TypeChecker —
        // PR 3 of decision-2026-04-27-e75 routed TS chain logic onto this seam.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = TypeScriptChecker.resolve_chain(
                chain_ref, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Workspace package lookup — highest priority for bare specifiers.
        // `import { foo } from '@myorg/utils'` where `@myorg/utils` is a
        // sibling workspace package. Scope lookup to that package's
        // symbol set and emit at confidence 1.0. Also handles deep imports
        // like `@myorg/utils/sub/mod` by stripping the trailing path.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_bare_specifier(module) {
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
                if !predicates::is_bare_specifier(module_path) {
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
                if sym.name == *target && predicates::kind_compatible(edge_kind, &sym.kind) {
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
                        resolved_yield_type: None,
                    });
                }
            }

            // Also try {module}.{target} as a qualified name (parser may use this form).
            // `all_by_qualified_name` to see past the TypeScript declaration-
            // merging case where the same qname exposes both an interface
            // (not callable) and a variable/function (callable).
            let candidate = format!("{module}.{target}");
            for sym in lookup.all_by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_import",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_import",
                        resolved_yield_type: None,
                    });
                }
            }

            // DefinitelyTyped prefix: when user imports `react` the runtime
            // package ships no types; the types live under `@types/react` and
            // qnames in the index are `@types/react.createContext` etc.
            // Retry the qname with the `@types/` prefix. Also handles the
            // scoped convention (`@scope/pkg` → `@types/scope__pkg`).
            if predicates::is_bare_specifier(module) && !module.starts_with("@types/") {
                let types_candidates = definitely_typed_qname_prefixes(module);
                for types_pkg in &types_candidates {
                    let candidate = format!("{types_pkg}.{target}");
                    for sym in lookup.all_by_qualified_name(&candidate) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            debug!(
                                strategy = "ts_import_definitely_typed",
                                candidate = %candidate,
                                specifier = %module,
                                "resolved"
                            );
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "ts_import_definitely_typed",
                                resolved_yield_type: None,
                            });
                        }
                    }
                }
            }

            // Deep-import qname stripping — same peel as the file_ctx import
            // loop below. When the extractor sets `ref.module = "rxjs/operators"`
            // but externals index the package as `rxjs.*`, retry the qname
            // lookup against progressively-shorter prefixes.
            if predicates::is_bare_specifier(module) && module.contains('/') {
                let mut path = module.as_str();
                while let Some(slash) = path.rfind('/') {
                    let parent = &path[..slash];
                    if parent.starts_with('@') && !parent.contains('/') {
                        break;
                    }
                    path = parent;
                    let candidate = format!("{path}.{target}");
                    for sym in lookup.all_by_qualified_name(&candidate) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            debug!(
                                strategy = "ts_import_deep",
                                candidate = %candidate,
                                specifier = %module,
                                "resolved"
                            );
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "ts_import_deep",
                                resolved_yield_type: None,
                            });
                        }
                    }
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
            //
            // CRITICAL: only return early when the relative module is
            // KNOWN to resolve to an indexed file but doesn't carry the
            // target. When the per-source map has no entry (parse miss,
            // generated file, etc.) we fall through so scope-chain /
            // same-file / heuristic still get a shot — without this,
            // shadowed locals and variables that happen to share an
            // imported name lose their edges.
            if !predicates::is_bare_specifier(module_path) {
                for sym in lookup.in_module_from(&file_ctx.file_path, module_path) {
                    if sym.name == *target
                        && predicates::kind_compatible(edge_kind, &sym.kind)
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
                            resolved_yield_type: None,
                        });
                    }
                }
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
                if let Some(res) =
                    follow_reexports(module_path, target, edge_kind, lookup, 0)
                {
                    return Some(res);
                }
                // Only short-circuit when the import resolved (we know the
                // target file) but the symbol genuinely isn't in it.
                // Otherwise fall through to scope chain / same-file etc.
                if resolved_path.is_some() {
                    return None;
                }
                continue;
            }

            let candidate = format!("{module_path}.{target}");
            // `all_by_qualified_name` covers the TypeScript declaration-merging
            // case: `@angular/core.Injectable` is declared as both an interface
            // (options type) and a variable (decorator function). `by_qname`'s
            // first-wins picks one; a Calls ref against the interface fails
            // kind_compatible and the variable overload never gets checked
            // unless we scan all duplicates.
            for sym in lookup.all_by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_bare_import_qname",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_bare_import_qname",
                        resolved_yield_type: None,
                    });
                }
            }
            // Deep-import qname stripping. When the exact qname misses —
            // `rxjs/operators.tap`, `lodash/fp.get`, `date-fns/utcToZonedTime.format` —
            // strip trailing `/seg` segments and retry. Externals index package
            // source under the package prefix alone (`rxjs.tap`), so the deep
            // import path has to be peeled off before the lookup can match.
            //
            // Scope boundary: stop before stripping a scope-only prefix like
            // `@angular`. `@angular/core/testing.X` strips to `@angular/core.X`
            // (valid package qname) but never down to `@angular.X` — scoped
            // packages always require a package segment after the scope.
            if module_path.contains('/') {
                let mut path = module_path.as_str();
                while let Some(slash) = path.rfind('/') {
                    let parent = &path[..slash];
                    if parent.starts_with('@') && !parent.contains('/') {
                        break;
                    }
                    path = parent;
                    let candidate = format!("{path}.{target}");
                    for sym in lookup.all_by_qualified_name(&candidate) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            debug!(
                                strategy = "ts_bare_import_deep",
                                candidate = %candidate,
                                specifier = %module_path,
                                "resolved"
                            );
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "ts_bare_import_deep",
                                resolved_yield_type: None,
                            });
                        }
                    }
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
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_scope_chain",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        // In TS/JS, symbols in the same file are visible at module scope.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && predicates::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_same_file",
                    qualified_name = %sym.qualified_name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_same_file",
                    resolved_yield_type: None,
                });
            }
        }

        // Step 3: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_qualified_name",
                        target = %target,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_qualified_name",
                        resolved_yield_type: None,
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
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.95,
                                strategy: "ts_field_type_chain",
                                resolved_yield_type: None,
                            });
                        }
                    }

                    // Also try: the type might be in a namespace, search by name.
                    let method_name = rest.split('.').next().unwrap_or(rest);
                    for sym in lookup.by_name(method_name) {
                        if sym.qualified_name.starts_with(type_name)
                            && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "ts_field_type_chain",
                                resolved_yield_type: None,
                            });
                        }
                    }

                    // Type is known but method isn't in our index — it's on the type.
                    // Don't fall through; let infer_external_namespace handle it.
                    break;
                }
            }
        }

        // Final step: npm-globals fallback for bare single-identifier calls.
        // Covers classic-asset-pipeline JS (Rails / PHP / vanilla server-
        // rendered) where `$(...)`, `jQuery(...)`, and similar library globals
        // appear without an `import` statement. Synthetic packages register
        // their globals under the `__npm_globals__.<name>` namespace; the
        // chain walker's Pass 3 already probes this for chain roots, but bare
        // non-chained calls need an explicit final check here.
        //
        // Scoped to single-identifier targets to avoid masking real unresolved
        // refs on dotted chains.
        if matches!(
            edge_kind,
            EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates
        ) && !target.contains('.')
        {
            let globals_candidate = format!(
                "{}.{target}",
                crate::ecosystem::npm::NPM_GLOBALS_MODULE
            );
            if let Some(sym) = lookup.by_qualified_name(&globals_candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.85,
                        strategy: "ts_npm_globals",
                        resolved_yield_type: None,
                    });
                }
            }

            // TS core lib + @types/node fallback. Symbols from
            // `ext:ts:__ts_lib__/...` files keep their bare qname after
            // Pass A (the post-processor skips prefixing for the synthetic
            // module so `HTMLElement.click` stays chain-walkable). That
            // means utility types (`Record`, `Omit`, `Exclude`, `Pick`),
            // DOM constructors (`HTMLElement`, `Document`, `ShadowRoot`),
            // and runtime functions whose `declare global` form lives in
            // `lib.dom.d.ts` / `lib.es*.d.ts` directly aren't reachable
            // through `__npm_globals__.X`. Probe the bare qname and only
            // accept the hit when the defining file is an ambient-global
            // lib file — keeps the fallback from silently grabbing a
            // user-defined `Foo` class with a colliding short name.
            for candidate in lookup.all_by_qualified_name(target) {
                if !crate::indexer::resolve::engine::is_ambient_global_lib_path(
                    &candidate.file_path,
                ) {
                    continue;
                }
                // Standard kind compatibility, plus a TS-lib-specific
                // relaxation: `declare var X: { new(): Y }` is how the
                // core lib encodes constructors (`Audio`, `Proxy`,
                // `FileReader`, `Map`, …). The extractor records those
                // as `variable`, but `new Audio()` carries
                // `EdgeKind::Instantiates` whose default
                // `kind_compatible` only accepts class/function. Trust
                // ambient lib variables for instantiation since the TS
                // type system already has — anything callable as a
                // constructor lands in this shape.
                let kind_ok = predicates::kind_compatible(edge_kind, &candidate.kind)
                    || (matches!(edge_kind, EdgeKind::Instantiates)
                        && candidate.kind == "variable");
                if !kind_ok {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: candidate.id,
                    confidence: 0.85,
                    strategy: "ts_lib_globals",
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
        let target = &ref_ctx.extracted_ref.target_name;

        // DOM interface types (HTML*/SVG*/ARIA*/IDB*/XPath*/MathML*) — global
        // in lib.dom.d.ts and always external. Pattern-based rather than a
        // hardcoded list: lib.dom.d.ts ships hundreds of these and new ones
        // land with each Chrome/Firefox release, so enumerating them by hand
        // is a losing game.
        if predicates::is_dom_interface_type(target) {
            return Some("runtime".to_string());
        }
        // React namespace types (`React.FC`, `React.ReactNode`) — available
        // globally under the JSX runtime without an explicit import in files
        // that use `jsx: react-jsx`.
        if predicates::is_react_namespace_type(target) {
            return Some("runtime".to_string());
        }

        // If the ref itself carries a module path, check it directly.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_bare_specifier(module) {
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
            if !predicates::is_bare_specifier(module_path) {
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
                        if predicates::is_bare_specifier(module_path) {
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

        // No hardcoded "looks like a common DOM/Array/Promise method" fallback
        // here — that's a guess, not a fact. Bare method calls whose receiver
        // type we couldn't infer (or whose receiver resolves internally by
        // coincidence of name) fall through to the heuristic tier so the
        // symbol index answers honestly instead. When lib.dom.d.ts and
        // lib.es5.d.ts are indexed through the externals pipeline, their
        // symbols (`Array.prototype.map`, `Event.composedPath`, etc.) are
        // reachable through the normal by-name lookup.
        None
    }

    // is_visible: default implementation (always true) is correct for TS.
    // TypeScript's `export` keyword controls visibility, but for resolution
    // purposes we treat all indexed symbols as accessible.

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Try the lookup-free path first — covers the common cases.
        if let Some(ns) =
            self.infer_external_namespace(file_ctx, ref_ctx, project_ctx)
        {
            return Some(ns);
        }
        // R2: alias → barrel → external. When `@/foo/bar` resolves through
        // tsconfig paths to a workspace file that ONLY re-exports from a
        // bare external specifier, classify the consumer ref as external
        // using that bare specifier as the namespace. Without this, the
        // ref would fall through to the heuristic and pick a wrong
        // same-named symbol elsewhere in the project.
        let target = &ref_ctx.extracted_ref.target_name;

        // Check the ref's own module first.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if let Some(ns) = classify_passthrough_alias(
                module,
                target,
                ref_ctx.file_package_id,
                project_ctx,
                lookup,
            ) {
                return Some(ns);
            }
        } else {
            // No module on the ref — check file imports.
            for import in &file_ctx.imports {
                if import.imported_name != *target {
                    continue;
                }
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                if let Some(ns) = classify_passthrough_alias(
                    module_path,
                    target,
                    ref_ctx.file_package_id,
                    project_ctx,
                    lookup,
                ) {
                    return Some(ns);
                }
            }
        }

        // Last resort: ambient-global method classification. If the bare
        // target (simple name, no dotted prefix) is a method/property
        // declared in an ambient-global lib file (`lib.dom.d.ts`,
        // `lib.es5.d.ts`, `lib.webworker.d.ts`, `@types/node/*`), the call
        // is a DOM/ES runtime API. Replaces the hardcoded
        // `is_common_builtin_method` list — index-backed, adapts to the
        // project's own TypeScript version.
        //
        // Two triggers, each gated on "ambient name exists":
        //   1. Ref carries a chain — the chain walker already tried and
        //      bailed (we're in Tier 1.5). The receiver is untyped or
        //      resolved to an internal type with no matching member, and
        //      the method name is a known DOM/ES surface. Classify.
        //      Covers `this.theme.set(x)` where `theme = signal<T>()`
        //      can't be typed — `set` only lives on WritableSignal / Map /
        //      Set in lib.*.d.ts, so external is honest.
        //   2. Ref has no chain AND no internal same-name candidate. Bare
        //      call to an ambient name — `setTimeout(...)`, `fetch(...)`
        //      at file scope. Classify when no user-code function
        //      competes.
        if !target.contains('.') && lookup.is_ambient_global_method(target) {
            let has_chain = ref_ctx.extracted_ref.chain.is_some();
            if has_chain {
                return Some("runtime".to_string());
            }
            let has_internal = lookup
                .by_name(target)
                .iter()
                .any(|s| !lookup.is_external_file(&s.file_path));
            if !has_internal {
                return Some("runtime".to_string());
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Classify a consumer ref as external when its import specifier
/// rewrites through a tsconfig alias to a workspace file that only
/// re-exports the target from a bare (external) specifier.
///
/// Common pattern: `apps/x/src/i18n/client/trans.tsx` containing exactly
/// `export { Trans } from "react-i18next"` — the consumer's
/// `@/i18n/client/trans` import is effectively a re-export of the
/// Produce DefinitelyTyped qname prefixes for a bare specifier.
///
/// When a user imports `react` (runtime package with no inline types) the
/// actual type symbols live in `@types/react/*.d.ts` and are indexed under
/// the qname prefix `@types/react.*`. The TS resolver normally looks up
/// `react.createContext` which misses; this helper yields the alternate
/// `@types/`-prefixed candidates to retry.
///
/// Scoped convention: `@scope/pkg` → `@types/scope__pkg` (DefinitelyTyped's
/// escape for the inner `@`). Also yields `@types/pkg` for unscoped names.
fn definitely_typed_qname_prefixes(specifier: &str) -> Vec<String> {
    if specifier.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(rest) = specifier.strip_prefix('@') {
        if let Some(slash) = rest.find('/') {
            let scope = &rest[..slash];
            let pkg = &rest[slash + 1..];
            out.push(format!("@types/{scope}__{pkg}"));
        }
    } else {
        out.push(format!("@types/{specifier}"));
    }
    out
}

/// external `react-i18next` package, not an internal symbol.
///
/// Returns the bare external namespace (e.g. `"react-i18next"`) when the
/// chain holds; `None` otherwise.
fn classify_passthrough_alias(
    spec: &str,
    target: &str,
    package_id: Option<i64>,
    _project_ctx: Option<&ProjectContext>,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    // Need the rewritten bare path. Skip when no alias matches.
    let rewritten = lookup.resolve_tsconfig_alias(package_id, spec)?;

    // Try to locate the resolved file in the index. Walk the same shape
    // resolve_via_alias uses so we land on the actual indexed file.
    const EXTS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];
    let mut candidate_paths: Vec<String> = vec![rewritten.clone()];
    for ext in EXTS {
        candidate_paths.push(format!("{rewritten}{ext}"));
        candidate_paths.push(format!("{rewritten}/index{ext}"));
    }

    for candidate in &candidate_paths {
        let reexports = lookup.reexports_from(candidate);
        if reexports.is_empty() {
            continue;
        }
        // Look for a re-export entry that matches `target` (or a wildcard)
        // and points at a bare specifier. That bare spec is the external
        // namespace this ref resolves through.
        for (exported_name, source_module) in reexports {
            if !predicates::is_bare_specifier(source_module) {
                continue;
            }
            if exported_name != target && exported_name != "*" {
                continue;
            }
            return Some(source_module.clone());
        }
    }
    None
}

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
        if sym.name == *target && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "ts_tsconfig_alias",
                resolved_yield_type: None,
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
            if sym.name == *target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_tsconfig_alias",
                    resolved_yield_type: None,
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

    // Vue SFC default-import case: `import JetLabel from '@/Components/Label.vue'`
    // binds the local name `JetLabel` to the file's default export, but the
    // Vue extractor names the file's component class by its filename stem
    // (`Label`). Looking for a symbol named `JetLabel` in `Label.vue` never
    // hits. Since .vue files are single-default-export by convention, fall
    // back to the single Class symbol in the file.
    if rewritten.ends_with(".vue") {
        for sym in lookup.in_file(rewritten) {
            if sym.kind == "class" && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "ts_vue_default_import",
                    resolved_yield_type: None,
                });
            }
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
        if !predicates::kind_compatible(edge_kind, &sym.kind) {
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
                    resolved_yield_type: None,
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
            resolved_yield_type: None,
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
        if predicates::is_bare_specifier(source_module) {
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
            if sym.name == target_name && predicates::kind_compatible(edge_kind, &sym.kind) {
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
                    resolved_yield_type: None,
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
            if sym.name == target_name && predicates::kind_compatible(edge_kind, &sym.kind) {
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
                    resolved_yield_type: None,
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
