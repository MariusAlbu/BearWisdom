// =============================================================================
// engine/reexports — follow barrel / re-export chains to a declaring symbol
//
// `export { X } from './x'` and `export * from './x'` hops are walked from a
// module path (or a workspace-package entry) until the module that actually
// declares the target is reached; named entries win over the wildcard sweep
// and the walk is depth-bounded. Also answers the workspace-package questions
// built on the same evidence: a package's barrel files and the symbol a
// package declares under a given name.
//
// Candidate generation and bare-source file-path matching (the "does this file
// plausibly hold the source?" checks that don't touch the graph walk) live in
// the sibling `reexports_candidates` module.
// =============================================================================

use crate::indexer::resolve::engine::contract::{SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE};
pub(crate) use crate::indexer::resolve::engine::reexports_candidates::{
    relative_reexport_candidates, workspace_pkg_barrels, workspace_pkg_declared_symbol,
};
use crate::indexer::resolve::engine::reexports_candidates::{
    resolve_reexport_by_matching_file, resolve_relative_reexport,
};
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::EdgeKind;

/// Walk re-export chains from `module_path` to the module that defines
/// `target_name`, returning the declaring symbol when found. A `module_path` is
/// a resolved file path (or a workspace-package entry whose `reexports_from`
/// fallback resolves it). Both `export { X } from '...'` (named) and `export *
/// from '...'` (wildcard) hops are followed; the wildcard sweep runs after the
/// named entries so a more-specific named re-export wins. `kind_compatible`
/// gates a candidate by the ref's edge kind. Bounded at `MAX_DEPTH` hops.
pub(crate) fn follow_reexports(
    module_path: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    lookup: &dyn SymbolLookup,
    depth: u32,
    profile: &LanguageProfile,
) -> Option<SymbolInfo> {
    const MAX_DEPTH: u32 = 5;
    if depth >= MAX_DEPTH {
        return None;
    }

    let reexports = lookup.reexports_from(module_path);
    if reexports.is_empty() {
        return None;
    }

    let mut wildcard_sources: Vec<&str> = Vec::new();

    for (exported_name, source_module) in reexports {
        // A bare workspace-package source may use a consumer-scoped package
        // alias. Rewrite the profile-delimited alias head to the target package
        // so the hop keys on the real member; the wildcard path keeps the original.
        let workspace_source_module = profile.workspace_specifier_path(source_module);
        let renamed = if profile
            .source_module_path_policy(source_module)
            .is_relative(source_module)
            || lookup
                .workspace_package_id(workspace_source_module.as_ref())
                .is_some()
        {
            None
        } else {
            let Some(head) = profile.workspace_alias_head(source_module) else {
                continue;
            };
            lookup
                .resolve_package_alias(lookup.package_id_for_file(module_path), head)
                .map(|t| format!("{t}{}", &source_module[head.len()..]))
        };
        let source = renamed.as_deref().unwrap_or(source_module);
        let workspace_source = profile.workspace_specifier_path(source);

        // A bare source is followable when it resolves to a file OR names a sibling
        // workspace package whose barrel can be recovered; a true external is skipped.
        if !profile
            .source_module_path_policy(source)
            .is_relative(source)
            && lookup.resolve_module_from(module_path, source).is_none()
            && workspace_pkg_barrels(
                lookup,
                workspace_source.as_ref(),
                profile.imports.reexport_barrel_stems,
            )
            .is_empty()
            && lookup
                .workspace_package_id(workspace_source.as_ref())
                .is_none()
        {
            continue;
        }

        if exported_name == "*" {
            wildcard_sources.push(source_module.as_str());
            continue;
        }

        if exported_name != target_name {
            continue;
        }

        // A bare source naming a sibling workspace package may DECLARE the target
        // directly (a member that owns the type, re-exported through this module),
        // rather than forwarding it onward. Chase one hop into the package's own
        // symbol set (the cross-member re-export seam).
        if !profile
            .source_module_path_policy(source)
            .is_relative(source)
        {
            if let Some(id) = workspace_pkg_declared_symbol(
                lookup,
                workspace_source.as_ref(),
                target_name,
                edge_kind,
                kind_compatible,
            ) {
                return Some(reexport_resolution(id, "reexport_chain"));
            }
        }

        for sym in lookup.in_module_from(module_path, source) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                return Some(reexport_resolution(sym.id, "reexport_chain"));
            }
        }
        if profile
            .source_module_path_policy(source)
            .is_relative(source)
        {
            if let Some(res) = resolve_relative_reexport(
                lookup,
                module_path,
                source,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_chain",
                profile.source_module_path_policy(source),
            ) {
                return Some(res);
            }
        } else if let Some(res) = resolve_reexport_by_matching_file(
            lookup,
            source,
            target_name,
            edge_kind,
            kind_compatible,
            "reexport_chain",
            profile.source_module_path_policy(source),
        ) {
            return Some(res);
        }

        if let Some(res) = follow_reexport_source(
            module_path,
            source,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth,
            profile,
        ) {
            return Some(res);
        }
    }

    for source_module in wildcard_sources {
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                return Some(reexport_resolution(sym.id, "reexport_star"));
            }
        }
        if profile
            .source_module_path_policy(source_module)
            .is_relative(source_module)
        {
            if let Some(res) = resolve_relative_reexport(
                lookup,
                module_path,
                source_module,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_star",
                profile.source_module_path_policy(source_module),
            ) {
                return Some(res);
            }
        } else if let Some(res) = resolve_reexport_by_matching_file(
            lookup,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            "reexport_star",
            profile.source_module_path_policy(source_module),
        ) {
            return Some(res);
        }

        if let Some(res) = follow_reexport_source(
            module_path,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth,
            profile,
        ) {
            return Some(res);
        }
    }

    None
}

/// Recurse `follow_reexports` into a re-export's source module. A relative or
/// resolvable source recurses on its resolved file path; a bare workspace-package
/// source recurses on each of the package's re-exporting `index` barrels (the
/// bare specifier has no `resolve_module_from` mapping, so the barrel is
/// recovered from the package's own symbol set).
#[allow(clippy::too_many_arguments)]
fn follow_reexport_source(
    module_path: &str,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    lookup: &dyn SymbolLookup,
    depth: u32,
    profile: &LanguageProfile,
) -> Option<SymbolInfo> {
    if let Some(next) = lookup.resolve_module_from(module_path, source_module) {
        let next = next.to_string();
        return follow_reexports(
            &next,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
            profile,
        );
    }
    if !profile
        .source_module_path_policy(source_module)
        .is_relative(source_module)
    {
        let workspace_source = profile.workspace_specifier_path(source_module);
        for barrel in workspace_pkg_barrels(
            lookup,
            workspace_source.as_ref(),
            profile.imports.reexport_barrel_stems,
        ) {
            if let Some(res) = follow_reexports(
                &barrel,
                target_name,
                edge_kind,
                kind_compatible,
                lookup,
                depth + 1,
                profile,
            ) {
                return Some(res);
            }
        }
        return None;
    }
    // A relative source with no `resolve_module_from` mapping: recurse on each
    // candidate file the joined-and-normalized path could name, so a multi-hop
    // relative re-export chain (`./a` re-exports from `./b`) still threads.
    for next in relative_reexport_candidates(
        lookup,
        module_path,
        source_module,
        profile.source_module_path_policy(source_module),
    ) {
        if let Some(res) = follow_reexports(
            &next,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
            profile,
        ) {
            return Some(res);
        }
    }
    None
}

/// A resolved-via-re-export `SymbolInfo` tagged with the hop strategy.
pub(crate) fn reexport_resolution(id: i64, strategy: &'static str) -> SymbolInfo {
    SymbolInfo {
        target_symbol_id: id,
        confidence: RESOLVED_CONFIDENCE,
        strategy,
        resolved_yield_type: None,
        flow_emit: None,
    }
}
