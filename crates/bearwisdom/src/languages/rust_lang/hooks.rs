// =============================================================================
// languages/rust_lang/hooks.rs — RustHooks: the data-inexpressible seams.
//
// Bare-name, chain, module-anchored, self-keyword, prelude (stdlib ambient
// path), generic-param, and re-export binding all run through the generic
// engine + RUST_PROFILE data — there is no `resolve_ref`. What remains here is
// the per-language code the profile can't express:
//   * classify_external — Cargo.toml / STDLIB_CRATES branding plus the
//     import-table / wildcard / chain / externals-index fallbacks,
//   * build_file_context — the `use`-statement import table and module path,
//   * 15 flow detectors (Axum routes + WS Consumer, Actix resources, route
//     attribute macros, async-graphql, Tauri command, reqwest client, Diesel
//     ORM, sqlx macro, Lettre mailer, rdkafka MQ, redis config lookup, apalis
//     bgjob, tonic gRPC, UDS) plus the let-binding-propagation rewrite.
// =============================================================================
pub(crate) use super::flow_detectors::{
    detect_rust_actix_resource_emission, detect_rust_apalis_bgjob,
    detect_rust_async_graphql_attribute, detect_rust_axum_route_emission,
    detect_rust_axum_ws_consumer, detect_rust_diesel_emission, detect_rust_lettre_mailer,
    detect_rust_rdkafka_mq, detect_rust_redis_config_lookup, detect_rust_reqwest_emission,
    detect_rust_route_attribute_emission, detect_rust_sqlx_macro_emission,
    detect_rust_tauri_command_attribute, detect_rust_tonic_emission, detect_rust_uds_emission,
};
use super::{keywords, predicates};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::types::{EdgeKind, ParsedFile};

pub(super) fn infer_external_inner(
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
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Cargo)
            {
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
    // The manifest is authoritative for crate attribution.
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
        if import.imported_name != simple && Some(import.imported_name.as_str()) != module_root {
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
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Cargo)
            {
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
            if import.imported_name != simple && Some(import.imported_name.as_str()) != module_root
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
            let all_external = matches.iter().all(|s| s.file_path.starts_with("ext:"));
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
    if let Some(rest) = stripped.split_once("/library/").map(|(_, after)| after) {
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

pub(crate) fn detect_flow_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    // First try the chain as-is.
    let direct = detect_flow_inner(file_ctx, ref_ctx);
    if !direct.is_empty() {
        return direct;
    }

    // Let-binding propagation: when the chain root is an Identifier
    // segment, look up the variable's recorded type via field_type
    // (populated for Variable symbols by the Rust extractor). If the
    // type ends with `Client`, rewrite the chain with the type as the
    // new root and rerun the Tonic detector.
    let r = &ref_ctx.extracted_ref;
    let Some(chain) = r.chain.as_ref() else {
        return Vec::new();
    };
    let Some(root_seg) = chain.segments.first() else {
        return Vec::new();
    };
    if !matches!(root_seg.kind, crate::types::SegmentKind::Identifier) {
        return Vec::new();
    }
    // Resolve the variable in the source symbol's scope and read its type.
    let var_qname = match ref_ctx.source_symbol.scope_path.as_deref() {
        Some(scope) => format!("{}.{}", scope, root_seg.name),
        None => root_seg.name.clone(),
    };
    let type_name = match lookup.field_type_str(&var_qname) {
        Some(t) => t.to_string(),
        None => return Vec::new(),
    };
    // Only rewrite for known-Client patterns.
    if !type_name.ends_with("Client") && !type_name.ends_with("Stub") {
        return Vec::new();
    }
    // Build a substituted chain: replace root identifier with the type +
    // synthetic "new" constructor so the existing Tonic detector matches.
    let mut new_segments = vec![
        crate::types::ChainSegment {
            name: type_name.clone(),
            node_kind: "rewritten_var".to_string(),
            kind: crate::types::SegmentKind::Identifier,
            declared_type: None,
            type_args: vec![],
            optional_chaining: false,
            byte_offset: 0,
            declared_type_id: None,
            is_call: false,
            call_args: Vec::new(),
            type_arg_ids: Vec::new(),
        },
        crate::types::ChainSegment {
            name: "new".to_string(),
            node_kind: "rewritten_var".to_string(),
            kind: crate::types::SegmentKind::Property,
            declared_type: None,
            type_args: vec![],
            optional_chaining: false,
            byte_offset: 0,
            declared_type_id: None,
            is_call: false,
            call_args: Vec::new(),
            type_arg_ids: Vec::new(),
        },
    ];
    new_segments.extend(chain.segments.iter().skip(1).cloned());
    let rewritten = crate::types::MemberChain {
        segments: new_segments,
    };
    if let Some(em) = detect_rust_tonic_emission(&rewritten) {
        return vec![em];
    }
    if let Some(em) = detect_rust_reqwest_emission(&rewritten, &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    // HTTP-method route attribute → Consumer HttpCall.
    // Decorators emit a TypeRef with target_name=<verb> and module=URL
    // for actix-web / Rocket `#[get("/x")]` style declarations.
    if r.kind == EdgeKind::TypeRef {
        if let Some(em) =
            detect_rust_route_attribute_emission(r.target_name.as_str(), r.module.as_deref())
        {
            return vec![em];
        }
        if let Some(em) = detect_rust_tauri_command_attribute(r.target_name.as_str()) {
            // The command name = the function's own symbol name;
            // emission carries a wildcard and the pairer matches it
            // against the TS-side invoke(name) producer.
            return vec![em];
        }
        // async-graphql / juniper attribute markers — `#[Object]`,
        // `#[graphql_object]`, `#[SimpleObject]`, `#[Subscription]`.
        if let Some(em) = detect_rust_async_graphql_attribute(r.target_name.as_str()) {
            return vec![em];
        }
        return Vec::new();
    }

    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }

    // SQLx macros — `sqlx::query!`, `sqlx::query_as!`, `query_scalar!`
    // etc. land as Calls refs with `module = Some("sqlx")` and
    // `target_name` carrying the verb. `call_args` holds the entity
    // name (for query_as) and the SQL string (parsed from the macro
    // body by `extract_macro_string_args`).
    if let Some(em) =
        detect_rust_sqlx_macro_emission(r.target_name.as_str(), r.module.as_deref(), &r.call_args)
    {
        return vec![em];
    }

    let Some(chain) = r.chain.as_ref() else {
        return Vec::new();
    };

    if let Some(em) = detect_rust_axum_route_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_actix_resource_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_reqwest_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_diesel_emission(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_tonic_emission(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_lettre_mailer(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_apalis_bgjob(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_rdkafka_mq(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_redis_config_lookup(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_uds_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_axum_ws_consumer(chain) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
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

// =============================================================================
// LanguageEngineHooks impl + static instance.
// =============================================================================

pub struct RustHooks;

impl crate::type_checker::profile::hooks::LanguageEngineHooks for RustHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner_with_lookup(file_ctx, ref_ctx, lookup)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }
}

pub static RUST_HOOKS: RustHooks = RustHooks;
