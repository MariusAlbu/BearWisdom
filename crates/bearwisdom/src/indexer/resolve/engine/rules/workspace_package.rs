// =============================================================================
// engine/rules/workspace_package — sibling workspace-package scoped bind
//
// When a ref's module (or the import that binds its name) is a BARE module
// specifier (`@org/utils`, not `./utils`) that resolves to a sibling workspace
// package, the target is scoped to that package's symbol set.  Deep imports
// (`@org/utils/sub/mod`) are supported: `support::workspace_sub_path` peels
// trailing segments to find the package root, then the sub-path filters to
// symbols whose file contains it.
//
// A specifier led by `profile.imports.self_package_root` names the CURRENT
// file's own package rather than a sibling by declared name —
// `self_package_sub_path` resolves it against `file_package_id` directly
// instead of `workspace_package_id`'s declared-name table, so a name
// re-exported at the package root binds the same way a direct declaration
// would (both are members of the same package's symbol set).
//
// Gated on `profile.imports.workspace_packages`; the active profile decides
// which source spellings name package modules.
// =============================================================================

use crate::indexer::resolve::engine::contract::types::SymbolInfo;
use crate::indexer::resolve::engine::support::{
    follow_reexports, self_package_sub_path, workspace_pkg_barrels, workspace_sub_path,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};

pub struct WorkspacePackageRule;

impl LookupRule for WorkspacePackageRule {
    fn name(&self) -> &'static str {
        "workspace_package"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !ctx.profile.imports.workspace_packages {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();

        // The specifier comes from the ref's own module field first, then from
        // the import that binds this target by name — its BOUND name, so a
        // rename's original name never diverts the specifier to a module that
        // does not actually bind `target` in this file.
        let specifier: Option<&str> = match ctx.r().module.as_deref() {
            Some(m) => Some(m),
            None => ctx
                .file_ctx
                .imports
                .iter()
                .find(|imp| imp.bound_name() == target)
                .and_then(|imp| imp.module_path.as_deref()),
        };
        let Some(specifier) = specifier else {
            return LookupResult::Pass;
        };
        if !ctx
            .profile
            .source_module_path_policy(specifier)
            .is_bare(specifier)
        {
            return LookupResult::Pass;
        }
        // A bare head that is not itself a declared package name may be a
        // consumer-scoped package alias. Rewrite the profile-qualified
        // head to the target package so lookup, sub-path, and barrel discovery
        // all key on the real member.
        let rewritten;
        let specifier = if ctx.lookup.workspace_package_id(specifier).is_some() {
            specifier
        } else {
            let head = if ctx.profile.imports.self_package_root.is_none()
                || ctx.profile.qname_separator.is_empty()
            {
                specifier
            } else {
                specifier
                    .split(ctx.profile.qname_separator)
                    .next()
                    .unwrap_or(specifier)
            };
            match ctx
                .lookup
                .resolve_package_alias(ctx.ref_ctx.file_package_id, head)
            {
                Some(target) => {
                    rewritten = format!("{target}{}", &specifier[head.len()..]);
                    rewritten.as_str()
                }
                None => specifier,
            }
        };
        let (pkg_id, sub_path) = match self_package_sub_path(ctx.profile, specifier) {
            Some(sub_path) => (ctx.ref_ctx.file_package_id, sub_path),
            None => (
                ctx.lookup.workspace_package_id(specifier),
                workspace_sub_path(ctx.profile, specifier, ctx.lookup),
            ),
        };
        let Some(pkg_id) = pkg_id else {
            return LookupResult::Pass;
        };

        let mut fallback: Option<i64> = None;
        for sym in ctx.lookup.symbols_in_package(pkg_id) {
            if sym.name != target || !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            if let Some(sub) = sub_path.as_deref() {
                if sym.file_path.contains(sub) {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_workspace_package"),
                    );
                }
            }
            if fallback.is_none() {
                fallback = Some(sym.id);
            }
        }
        if let Some(id) = fallback {
            return LookupResult::Resolved(ctx.resolved(id, "default_workspace_package"));
        }

        // The package re-exports the name through its public barrel but does not
        // declare it — `export * from '@org/core'` forwards a sibling workspace
        // package's symbol. Follow the re-export chain from each of the package's
        // `index` barrels to the declaring symbol, which may live in another
        // workspace package. (The bare specifier has no `resolve_module_from`
        // mapping, so the barrel is recovered from the package's own symbol set.)
        let stems = ctx.profile.imports.reexport_barrel_stems;
        for barrel in workspace_pkg_barrels(ctx.lookup, specifier, stems) {
            if let Some(res) = follow_reexports(
                &barrel,
                target,
                edge_kind,
                ctx.kind,
                ctx.lookup,
                0,
                ctx.profile,
            ) {
                return LookupResult::Resolved(res);
            }
        }

        // Sub-path-guided follow: a submodule may re-export the target instead
        // of the package barrel. Follow re-exports from each package file whose
        // path carries the sub-path; bind the unique target and decline when the
        // follow fans out to more than one.
        if let Some(sub) = sub_path.as_deref() {
            let mut hit: Option<SymbolInfo> = None;
            let mut seen: std::collections::BTreeSet<String> = Default::default();
            for sym in ctx.lookup.symbols_in_package(pkg_id) {
                let path = sym.file_path.as_ref();
                if !path.contains(sub) || !seen.insert(path.to_string()) {
                    continue;
                }
                if ctx.lookup.reexports_from(path).is_empty() {
                    continue;
                }
                if let Some(res) = follow_reexports(
                    path,
                    target,
                    edge_kind,
                    ctx.kind,
                    ctx.lookup,
                    0,
                    ctx.profile,
                ) {
                    if let Some(prev) = &hit {
                        if prev.target_symbol_id != res.target_symbol_id {
                            return LookupResult::Pass;
                        }
                    } else {
                        hit = Some(res);
                    }
                }
            }
            if let Some(res) = hit {
                return LookupResult::Resolved(res);
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "workspace_package_tests.rs"]
mod tests;
