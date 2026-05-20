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
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::flow_emit::FlowEmission;
use crate::types::{EdgeKind, ParsedFile};
use tracing::debug;

use super::aliases::{
    classify_passthrough_alias, definitely_typed_qname_prefixes, follow_reexports,
    is_npm_package_match, resolve_via_alias, resolve_workspace_package,
    sub_path_for_deep_import,
};
use super::flow_detectors::{
    BGJOB_QUEUE_BINDING_KEY, CONTROLLER_PREFIX_KEY,
};

// Re-export the helpers that the sibling test file and other TS modules
// historically reached via `super::resolve::X`. Keeps the existing
// pub(crate) call surface stable across the carve-up into aliases.rs and
// flow_detectors.rs.
pub(crate) use super::aliases::is_manifest_ts_package;
pub(crate) use super::flow_detectors::{
    canonical_rpc_key, detect_addservice_object_keys, detect_angular_injectable_emission,
    detect_bgjob_chain_emission, detect_chain_flow_emission, detect_chain_route_consumer,
    detect_config_call_emission, detect_db_query_emission, detect_db_query_emission_with_imports,
    detect_decorator_flow_emission, detect_decorator_flow_emission_with_imports,
    detect_feature_flag_chain_emission, detect_grpc_decorator_flow_emission,
    detect_mailer_chain_emission, detect_member_access_config_emission,
    detect_member_access_feature_flag_emission, detect_mq_chain_emission, detect_rpc_chain_emission,
    detect_route_decorator_flow_emission, detect_trpc_chain_emission, join_route_segments,
    lookup_controller_prefix, parse_gql_operation,
};

pub use predicates::is_bare_specifier;

/// TypeScript and JavaScript language resolver.
///
/// **Phase 5 archive status:** since the engine pivot (commit `8b1b89f9`),
/// chain-bearing refs in TypeScript / TSX / JavaScript / JSX route through
/// `crate::type_checker::Engine::resolve` first; this resolver is consulted
/// only as a fallback when the engine declines, and for bare-name refs that
/// the engine doesn't yet handle. The entry points remain for fallback
/// coverage of TS patterns the engine's chain walker doesn't yet model
/// (declaration merging, ambient global synthesis, decorator-driven member
/// synthesis).
pub struct TypeScriptResolver;

impl TypeScriptResolver {

    
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
pub(crate) fn resolve(
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
        // step-by-step following field types. Dispatch to the TypeChecker.
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
                        flow_emit: None,
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
                        flow_emit: None,
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
                                flow_emit: None,
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
                                flow_emit: None,
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
                            flow_emit: None,
                        });
                    }
                }
                // Single-default-export component files (.vue/.astro/.svelte):
                // the importer uses its own local binding name, so the name check
                // above never matches the file-stem class symbol. Accept the file's
                // single class symbol as the default export.
                const SFC_EXTS: &[&str] = &[".vue", ".astro", ".svelte"];
                if SFC_EXTS.iter().any(|ext| module_path.ends_with(ext)) {
                    for sym in lookup.in_module_from(&file_ctx.file_path, module_path) {
                        if sym.kind == "class" && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            debug!(
                                strategy = "ts_sfc_default_import",
                                module = %module_path,
                                target = %target,
                                "resolved via SFC class symbol"
                            );
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.95,
                                strategy: "ts_sfc_default_import",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
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
                        flow_emit: None,
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
                                flow_emit: None,
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
                        flow_emit: None,
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
                    flow_emit: None,
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
                        flow_emit: None,
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
                if let Some(type_name) = lookup.field_type_str(&field_qname) {
                    // Found field type. Try {TypeName}.{rest} in the index.
                    let candidate = format!("{type_name}.{rest}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.95,
                                strategy: "ts_field_type_chain",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                    }

                    // Also try: the type might be in a namespace, search by name.
                    let method_name = rest.split('.').next().unwrap_or(rest);
                    for sym in lookup.by_name(method_name) {
                        if sym.qualified_name.starts_with(type_name.as_str())
                            && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "ts_field_type_chain",
                                resolved_yield_type: None,
                                flow_emit: None,
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
                        flow_emit: None,
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
                    flow_emit: None,
                });
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    // is_visible: default implementation (always true) is correct for TS.
    // TypeScript's `export` keyword controls visibility, but for resolution
    // purposes we treat all indexed symbols as accessible.

}

pub(crate) fn infer_external_inner(
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

pub(crate) fn infer_external_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    // Try the lookup-free path first — covers the common cases.
    if let Some(ns) =
        infer_external_inner(file_ctx, ref_ctx, project_ctx)
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

pub(crate) fn detect_flow_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    // Decorator-based detection: TypeRef refs whose target_name matches a
    // well-known decorator (NestJS guards, TypeORM/Sequelize entity markers, etc.).
    if r.kind == EdgeKind::TypeRef {
        // Decorator refs no longer carry first_arg in r.module — read it
        // from r.call_args[0] (CallArg::StringLit) instead. r.module is
        // reserved for import-source paths.
        let decorator_first_arg: Option<&str> = r.call_args.iter().find_map(|a| {
            if let crate::types::CallArg::StringLit(s) = a {
                Some(s.as_str())
            } else {
                None
            }
        });
        // NestJS HTTP route decorators (Consumer-role HttpCall). Checked
        // first because @Get/@Post/etc. would otherwise pass through the
        // generic decorator handler unrecognised.
        if let Some(emission) = detect_route_decorator_flow_emission(
            r.target_name.as_str(),
            decorator_first_arg,
            ref_ctx.source_symbol.qualified_name.as_str(),
            file_ctx,
        ) {
            return vec![emission];
        }
        // NestJS gRPC decorators (Consumer-role RpcCall). Reads the second
        // call argument (method name) when present and falls back to the
        // enclosing method's name otherwise.
        if let Some(emission) = detect_grpc_decorator_flow_emission(
            r.target_name.as_str(),
            &r.call_args,
            ref_ctx.source_symbol.name.as_str(),
        ) {
            return vec![emission];
        }
        if let Some(emission) = detect_decorator_flow_emission_with_imports(
            r.target_name.as_str(),
            decorator_first_arg,
            Some(ref_ctx.source_symbol.name.as_str()),
            &file_ctx.imports,
        ) {
            return vec![emission];
        }
        // DiBinding: `@Inject('TOKEN')` decorator (NestJS explicit DI).
        // Emits a single-ended DiBinding row tagged container=nestjs.
        // `service_symbol_id` carries 0 as a placeholder — the field
        // isn't written to the `flow_edges` table; only the `edge_type`
        // column is consulted for downstream queries.
        if r.target_name == "Inject" {
            if let Some(token) = decorator_first_arg {
                if !token.is_empty() {
                    return vec![FlowEmission::DiBinding {
                        service_symbol_id: 0,
                        container: Some(format!("nestjs:{}", token)),
                    }];
                }
            }
            return vec![FlowEmission::DiBinding {
                service_symbol_id: 0,
                container: Some("nestjs".to_string()),
            }];
        }
        // DiBinding: `@Injectable()` decorator (Angular service marker).
        // Emits a single-ended DiBinding tagged container=angular so the
        // architecture overview clusters Angular services next to NestJS
        // providers. Constructor-injection consumer sites are not
        // resolved here — the pair-keyed match against component
        // constructors would need cross-symbol state that the resolver
        // doesn't carry; the single-ended emission still clusters.
        if let Some(emission) = detect_angular_injectable_emission(r.target_name.as_str()) {
            return vec![emission];
        }
        return Vec::new();
    }

    // Imports-kind synthetic refs from member-access detection (see
    // `calls::emit_config_lookup_ref`). The extractor emits these for
    // `process.env.X`, `import.meta.env.X`, and feature-flag-shaped
    // member access (`featureFlags.X`, `<...featureFlagsManager>.value.X`).
    // Routed here so the chain shape drives a single-ended `ConfigLookup`
    // or `FeatureFlag` emission without polluting the `unresolved_refs`
    // table (Imports refs are classified as external by the loop).
    if r.kind == EdgeKind::Imports {
        if let Some(chain) = &r.chain {
            if let Some(emission) = detect_member_access_config_emission(chain) {
                return vec![emission];
            }
            if let Some(emission) = detect_member_access_feature_flag_emission(chain) {
                return vec![emission];
            }
        }
        return Vec::new();
    }

    // Call-based detection: Calls and Instantiates refs that carry a chain.
    // Instantiates emits a single-segment chain `[ConstructorName]` plus
    // `call_args`, so constructor-keyed patterns flow through the same
    // detector surface as method calls (`new Worker('queue', ...)`).
    if r.kind != EdgeKind::Calls && r.kind != EdgeKind::Instantiates {
        return Vec::new();
    }
    let Some(chain_ref) = r.chain.as_ref() else { return Vec::new(); };

    // gRPC `server.addService(SvcDef, { m1: h, m2: h })`: expand to one
    // Consumer emission per registered method when the object-literal
    // keys are captured. Falls back to the wildcard form when the second
    // argument isn't a plain object.
    if let Some(expanded) = detect_addservice_object_keys(chain_ref, &r.call_args, file_ctx) {
        return expanded;
    }

    detect_chain_flow_emission(chain_ref, &r.call_args, file_ctx)
        .map(|e| vec![e])
        .unwrap_or_default()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // NestJS controller-prefix pre-pass: stash one synthetic ImportEntry
    // per `@Controller(...)` decorator so method-level `@Get/@Post/...`
    // refs can recover the route prefix when assembling their full
    // pattern. The key is `__ts_controller_prefix__:<class-qualified-name>`
    // and the value is the prefix string (or empty when the prefix arg
    // is not a string literal — e.g. `@Controller(RouteKey.User)`).
    //
    // Both `import { Controller } from '@nestjs/common'` AND `@Controller(...)`
    // produce TypeRef refs with target_name="Controller". The decorator
    // ref now carries its first_arg in `call_args` (not `module`); the
    // import ref has `module = Some("@nestjs/common")` and empty
    // call_args. Use the presence of `module` to skip imports — only
    // decorator refs feed the prefix lookup.
    for r in &file.refs {
        if r.kind != EdgeKind::TypeRef || r.target_name != "Controller" {
            continue;
        }
        if r.module.is_some() {
            // Import ref — `import { Controller } from '@nestjs/common'`.
            continue;
        }
        let Some(sym) = file.symbols.get(r.source_symbol_index) else { continue; };
        let prefix = r
            .call_args
            .iter()
            .find_map(|a| match a {
                crate::types::CallArg::StringLit(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        imports.push(ImportEntry {
            imported_name: format!("{CONTROLLER_PREFIX_KEY}{}", sym.qualified_name),
            module_path: Some(prefix),
            alias: None,
            is_wildcard: false,
        });
    }

    // Background-job queue-binding entries: `const NAME = new Queue("Q")`
    // emits a synthetic `EdgeKind::Imports` ref keyed
    // `__ts_bgjob_queue_binding__:NAME` with module `"Q"` (see
    // `calls::emit_new_ref`). The generic import loop below picks them up
    // alongside real imports — no special-case handling needed here.

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
