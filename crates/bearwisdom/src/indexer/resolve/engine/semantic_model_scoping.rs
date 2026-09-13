// =============================================================================
// engine/semantic_model_scoping.rs — the modules a chain root is scoped to
//
// A chain whose root is a namespace import, a wildcard import or an explicit
// module qualifier resolves its members inside those modules; these helpers
// name them from the file's import table.
// =============================================================================

use super::*;

/// `true` when the ref's extractor-set `module` is exactly the chain's own
/// qualifier path — every segment but the target, normalized through the active
/// profile into the canonical index qname. That shape means the target is a DIRECT
/// member of the module (`serde_json::from_value` → module `serde_json`,
/// chain `[serde_json, from_value]`), so the module-evidence rungs can bind
/// it. A module tag naming where the chain's root was imported from joins to
/// a different string and is rejected.
pub(super) fn module_is_chain_qualifier(
    r: &crate::types::ExtractedRef,
    chain: &crate::types::MemberChain,
    profile: &LanguageProfile,
) -> bool {
    let Some(module) = r.module.as_deref() else {
        return false;
    };
    let chain_qname = chain.segments[..chain.segments.len() - 1]
        .iter()
        .fold(String::new(), |qname, segment| {
            profile.index_qname_join(&qname, &segment.name)
        });
    chain_qname == profile.index_qname_from_source(module)
}

/// Source-addressed module paths a namespace root may use after its value walk
/// declines. A same-file value wins before any namespace candidate; an imported
/// root must tie its namespace declaration to that import's module/package/path.
/// A same-named namespace elsewhere in the index supplies no module evidence.
pub(super) fn namespace_root_modules(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> Vec<String> {
    let Some(root) = chain.segments.first() else {
        return Vec::new();
    };
    let candidates = lookup.by_name(&root.name);
    if candidates.iter().any(|s| {
        s.file_path.as_ref() == file_ctx.file_path
            && crate::indexer::resolve::engine::kinds::is_value_kind(&s.kind)
    }) {
        return Vec::new();
    }

    let is_namespace = |kind: &str| matches!(kind, "namespace" | "module");
    let mut modules = Vec::new();
    for candidate in candidates.iter().filter(|s| is_namespace(&s.kind)) {
        let imported = file_ctx.imports.iter().any(|import| {
            !import.is_wildcard
                && import.bound_name() == root.name.as_str()
                && namespace_matches_import(candidate, import, file_ctx, lookup, profile)
        });
        let same_package =
            file_package_id.is_some_and(|package_id| candidate.package_id == Some(package_id));
        if imported || same_package {
            push_unique_module(&mut modules, &candidate.qualified_name);
            if imported {
                for import in &file_ctx.imports {
                    if !import.is_wildcard
                        && import.bound_name() == root.name.as_str()
                        && namespace_matches_import(candidate, import, file_ctx, lookup, profile)
                    {
                        if let Some(module) = import.module_path.as_deref() {
                            push_unique_module(&mut modules, module);
                        }
                    }
                }
            }
        }
    }
    // Ambient declarations are source-visible even when an index only keeps
    // them in its ambient scope (rather than in the global simple-name map).
    for candidate in lookup
        .ambient_symbols(&root.name)
        .iter()
        .filter(|candidate| is_namespace(&candidate.kind))
    {
        push_unique_module(&mut modules, &candidate.qualified_name);
    }
    modules
}

/// `true` when the chain's root segment names a namespace/module declaration
/// the current file binds. Exposed to the focused unit tests; production uses
/// [`namespace_root_modules`] to preserve the exact source-addressed modules.
#[cfg(test)]
pub(super) fn chain_root_is_namespace(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
) -> bool {
    !namespace_root_modules(
        chain,
        file_ctx,
        file_package_id,
        lookup,
        &crate::type_checker::profile::language_profile::DEFAULT_PROFILE,
    )
    .is_empty()
}

/// A namespace declaration is import-bound only when the import's module can
/// actually reach it: the resolved module candidate, its workspace package,
/// its qname, or its indexed file path agrees with the import path.
pub(super) fn namespace_matches_import(
    candidate: &crate::indexer::resolve::engine::contract::Symbol,
    import: &crate::indexer::resolve::engine::contract::ImportEntry,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> bool {
    let Some(module) = import.module_path.as_deref() else {
        return false;
    };
    lookup
        .in_module_from(&file_ctx.file_path, module)
        .iter()
        .any(|symbol| symbol.id == candidate.id)
        || lookup
            .workspace_package_id(profile.workspace_specifier_path(module).as_ref())
            .is_some_and(|package_id| candidate.package_id == Some(package_id))
        || lookup
            .by_qualified_name(module)
            .is_some_and(|declared| declared.id == candidate.id)
        || super::super::support::qname_under_module(profile, &candidate.qualified_name, module)
        || super::super::support::file_path_matches_module(&candidate.file_path, module, profile)
}

/// Source-addressed modules named by a wildcard import root (`import * as v`).
pub(super) fn wildcard_root_modules(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
) -> Vec<String> {
    let Some(root) = chain.segments.first() else {
        return Vec::new();
    };
    let mut modules = Vec::new();
    for import in &file_ctx.imports {
        if import.is_wildcard
            && (import.alias.as_deref() == Some(root.name.as_str())
                || import.imported_name == root.name)
        {
            if let Some(module) = import.module_path.as_deref() {
                push_unique_module(&mut modules, module);
            }
        }
    }
    modules
}

pub(super) fn push_unique_module(modules: &mut Vec<String>, module: &str) {
    if !module.is_empty() && !modules.iter().any(|candidate| candidate == module) {
        modules.push(module.to_string());
    }
}

/// `true` when the chain's root segment names a wildcard/namespace import in this
/// file (`import * as v from 'm'` — `is_wildcard`, matched by alias or imported
/// name). The alias names a module, not a value, so `v.member` resolves against
/// the module's exports through the module-scoped ladder rather than the value walk.
#[cfg(test)]
pub(super) fn chain_root_is_wildcard_import(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
) -> bool {
    let Some(root) = chain.segments.first() else {
        return false;
    };
    file_ctx.imports.iter().any(|i| {
        i.is_wildcard
            && (i.alias.as_deref() == Some(root.name.as_str()) || i.imported_name == root.name)
    })
}
