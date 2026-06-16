// =============================================================================
// languages/typescript/hooks.rs — TypeScriptHooks impl.
//
// Reference resolution is the generic engine (`DefaultResolver` ladder + the
// profile-driven `ChainWalker`) reading `TYPESCRIPT_PROFILE` data; there is no
// `resolve_ref`. What lives here is the per-language code the profile can't
// express:
//   * `classify_external` — external classifier (DOM / React-namespace types,
//     bare-specifier manifest match, alias-passthrough-via-barrel,
//     ambient-global method index lookup).
//   * `detect_flow_emissions` — 30+ flow detectors (NestJS routes / gRPC /
//     Injectable / Inject + Angular Injectable + decorator-driven
//     config/feature-flag/db-query/mailer/mq/bgjob/RPC/tRPC + chain
//     HTTP/db/RPC/route consumer + addService gRPC method expansion +
//     member-access config/feature-flag synthetic Imports refs).
//   * `build_file_context` — the import table, with a NestJS controller-prefix
//     pre-pass and bgjob queue-binding refs.
// `TS_CHAIN_CONFIG` + `ts_root_globals_fallback` back the structured chain
// walker (`resolve_via_chain`), which is engine plumbing, not a resolution seam.
// =============================================================================
use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::indexer::resolve::flow_emit::FlowEmission;
use crate::types::{EdgeKind, ParsedFile};

use super::aliases::{classify_passthrough_alias, is_npm_package_match};
use super::flow_detectors::{BGJOB_QUEUE_BINDING_KEY, CONTROLLER_PREFIX_KEY};

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
    detect_member_access_feature_flag_emission, detect_mq_chain_emission,
    detect_route_decorator_flow_emission, detect_rpc_chain_emission, detect_trpc_chain_emission,
    join_route_segments, lookup_controller_prefix, parse_gql_operation,
};

pub use predicates::is_bare_specifier;

/// Last-resort chain root for a bare-identifier call the import scan missed:
/// jest/vitest `globals: true` injects `vi`/`expect`/`describe`/`test` without
/// an import. The npm ecosystem walker records their type under the synthetic
/// `__npm_globals__.<name>` qname; probe its return/field type as the root.
fn ts_root_globals_fallback(name: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    let candidate = format!("{}.{name}", crate::ecosystem::npm::NPM_GLOBALS_MODULE);
    lookup
        .return_type_str(&candidate)
        .or_else(|| lookup.field_type_str(&candidate))
}

/// TypeScript / JavaScript `ChainConfig` for the unified `resolve_via_chain`.
///
/// TS opts into every chain extension: alias expansion (`type X = Y<Z>`
/// receivers), inheritance climbing on a member miss, external-qname promotion
/// (`Assertion` → `chai.Assertion`), `new X().m()` construction roots, and the
/// jest/vitest ambient-globals root fallback. `static_type_kinds` /
/// `enclosing_type_kinds` match the kinds the TS extractor emits.
pub(crate) static TS_CHAIN_CONFIG: crate::type_checker::chain::ChainConfig =
    crate::type_checker::chain::ChainConfig {
        strategy_prefix: "ts",
        normalize_type: crate::type_checker::chain::identity_normalize,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "struct", "interface"],
        static_type_kinds: &["class", "struct", "interface", "enum", "type_alias"],
        use_generics: true,
        namespace_lookup: crate::type_checker::chain::NamespaceLookup::None,
        kind_compatible: predicates::kind_compatible,
        extensions: crate::type_checker::chain::ChainExtensions {
            expand_aliases: true,
            walk_inheritance: true,
            promote_external_qname: true,
            root_construction: true,
            extension_method_fallback: false,
            root_fallback: Some(ts_root_globals_fallback),
            root_type_access: false,
            qualify_via_imports: false,
        },
    };

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
                if let Some(manifest) = ctx
                    .manifests_for(ref_ctx.file_package_id)
                    .get(&ManifestKind::Npm)
                {
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
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Npm)
            {
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
                            Some(ctx) => {
                                is_manifest_ts_package(ctx, ref_ctx.file_package_id, module_path)
                            }
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
    if let Some(ns) = infer_external_inner(file_ctx, ref_ctx, project_ctx) {
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
        if let Some(ns) =
            classify_passthrough_alias(module, target, ref_ctx.file_package_id, project_ctx, lookup)
        {
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
    let Some(chain_ref) = r.chain.as_ref() else {
        return Vec::new();
    };

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
        let Some(sym) = file.symbols.get(r.source_symbol_index) else {
            continue;
        };
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

// =============================================================================
// LanguageEngineHooks impl + static instance.
// =============================================================================

pub struct TypeScriptHooks;

impl crate::type_checker::profile::hooks::LanguageEngineHooks for TypeScriptHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner_with_lookup(file_ctx, ref_ctx, project_ctx, lookup)
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }
}

pub static TYPESCRIPT_HOOKS: TypeScriptHooks = TypeScriptHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod hooks_tests;
