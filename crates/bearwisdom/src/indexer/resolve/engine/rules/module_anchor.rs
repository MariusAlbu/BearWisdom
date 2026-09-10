// =============================================================================
// engine/rules/module_anchor — ref.module drives a module-anchored bind
//
// Fires when the profile opts in via `ModuleAnchor::On(bind)` and the ref
// carries an extractor-set `module` field.  Five bind variants choose how the
// module + target pair is resolved:
//
//   NameExactKind          — in_module_from(file, module) → name == target && kind
//   PreferNamedElseFirst   — in_module_from → same-named (case-insensitive) else first
//   ByNameUnderModuleDir   — qname probe {prefix}{sep}{target}, then dir-containment
//   ByFileStem             — by_name(target) whose file stem equals module leaf
//   MemberOfModuleType     — members_of(module) → normalized name == target && kind
//
// A relative module (controlled by `relative_marker`) runs the profile-configured
// bind; an absolute module routes to `ByNameUnderModuleDir` regardless of bind.
// The `module_anchor_terminal` guard that stops the ladder after a miss lives in a
// separate `ModuleAnchorTerminalRule`.
// =============================================================================

use crate::indexer::resolve::engine::support::{normalize_name, path_stem_matches};
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::{
    ModuleAnchor, ModuleAnchorBind, ModulePrefixRewrites, StemSource,
};

pub struct ModuleAnchorRule;

impl LookupRule for ModuleAnchorRule {
    fn name(&self) -> &'static str {
        "module_anchor"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let ModuleAnchor::On(bind) = ctx.profile.imports.module_anchor else {
            return LookupResult::Pass;
        };
        let Some(module) = ctx.r().module.as_deref() else {
            return LookupResult::Pass;
        };
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        let norm = ctx.profile.name_normalization;
        let rewrites = ctx.profile.imports.module_prefix_rewrites;
        let overload_pick_all = ctx.profile.overload_pick_all;

        let is_relative = ctx.profile.module_anchor_is_relative(module);
        // Absolute modules route to ByNameUnderModuleDir regardless of the
        // profile's configured bind; relative modules use the configured bind.
        let effective_bind = if is_relative {
            bind
        } else {
            ModuleAnchorBind::ByNameUnderModuleDir
        };

        match effective_bind {
            ModuleAnchorBind::NameExactKind => {
                for sym in ctx.lookup.in_module_from(&ctx.file_ctx.file_path, module) {
                    if sym.name == target && (ctx.kind)(edge_kind, &sym.kind) {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_module_anchor"),
                        );
                    }
                }
            }
            ModuleAnchorBind::PreferNamedElseFirst => {
                let syms = ctx.lookup.in_module_from(&ctx.file_ctx.file_path, module);
                let pick = syms
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(target))
                    .or_else(|| syms.first());
                if let Some(sym) = pick {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "default_module_anchor"));
                }
            }
            ModuleAnchorBind::ByNameUnderModuleDir => {
                // Each source prefix is converted through the active profile
                // before probing the canonical index qname.
                for prefix in module_prefix_candidates(module, rewrites) {
                    let qname = ctx.profile.index_qname_join(&prefix, target);
                    if overload_pick_all {
                        for sym in ctx.lookup.all_by_qualified_name(&qname) {
                            if (ctx.kind)(edge_kind, &sym.kind) {
                                return LookupResult::Resolved(
                                    ctx.resolved(sym.id, "default_module_anchor"),
                                );
                            }
                        }
                    } else if let Some(sym) = ctx.lookup.by_qualified_name(&qname) {
                        if (ctx.kind)(edge_kind, &sym.kind) {
                            return LookupResult::Resolved(
                                ctx.resolved(sym.id, "default_module_anchor"),
                            );
                        }
                    }
                }
                // Directory-containment fallback: map the module to a path
                // fragment and match against file_path. A profile adapter may
                // decline this fallback for a source module spelling.
                if declines_bare_directory_match(module, rewrites) {
                    return LookupResult::Pass;
                }
                let module_as_path = ctx.profile.index_qname_path_from_source(module);
                let leaf = ctx.profile.simple_name(module).to_lowercase();
                let indexed_target = ctx.profile.index_qname_from_source(target);
                let candidates = ctx.lookup.by_name(target);
                // Two passes: top-level declarations (qname == bare target)
                // first, member declarations second. A module-qualified target
                // names a top-level item of that module, so `mod::f` must not
                // bind a member `T.f` when the located file set also declares a
                // top-level `f`.
                for top_level_pass in [true, false] {
                    for sym in &candidates {
                        if (sym.qualified_name == indexed_target) != top_level_pass {
                            continue;
                        }
                        if !(ctx.kind)(edge_kind, &sym.kind) {
                            continue;
                        }
                        let path = sym.file_path.replace('\\', "/");
                        if path.contains(&module_as_path) {
                            return LookupResult::Resolved(
                                ctx.resolved(sym.id, "default_module_anchor"),
                            );
                        }
                        if leaf != module_as_path && path_stem_matches(&path.to_lowercase(), &leaf)
                        {
                            return LookupResult::Resolved(
                                ctx.resolved(sym.id, "default_module_anchor"),
                            );
                        }
                    }
                }
            }
            ModuleAnchorBind::ByFileStem { against } => {
                let leaf = match against {
                    StemSource::ModuleLeaf => ctx.profile.simple_name(module).to_lowercase(),
                };
                for sym in ctx.lookup.by_name(target) {
                    if !(ctx.kind)(edge_kind, &sym.kind) {
                        continue;
                    }
                    if path_stem_matches(&sym.file_path.to_lowercase(), &leaf) {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_module_anchor"),
                        );
                    }
                }
            }
            ModuleAnchorBind::MemberOfModuleType => {
                let target_norm = normalize_name(norm, target);
                for member in ctx.lookup.members_of(module) {
                    if normalize_name(norm, &member.name) == target_norm
                        && (ctx.kind)(edge_kind, &member.kind)
                    {
                        return LookupResult::Resolved(
                            ctx.resolved(member.id, "default_module_anchor"),
                        );
                    }
                }
            }
        }
        LookupResult::Pass
    }
}

// =============================================================================
// Private helpers — used only by this rule
// =============================================================================

/// The ordered module-prefix candidates the `ByNameUnderModuleDir` anchor
/// probes. Profiles that opt into rewrites delegate candidate construction to
/// their language or ecosystem adapter; the resolver only preserves order.
fn module_prefix_candidates(module: &str, rewrites: ModulePrefixRewrites) -> Vec<String> {
    match rewrites {
        ModulePrefixRewrites::Off => vec![module.to_string()],
        ModulePrefixRewrites::On {
            candidate_prefixes, ..
        } => candidate_prefixes(module),
    }
}

/// Whether the profile's adapter declines the directory-containment fallback
/// of `ByNameUnderModuleDir` for this source module spelling.
fn declines_bare_directory_match(module: &str, rewrites: ModulePrefixRewrites) -> bool {
    match rewrites {
        ModulePrefixRewrites::Off => false,
        ModulePrefixRewrites::On {
            declines_directory_match,
            ..
        } => declines_directory_match(module),
    }
}

#[cfg(test)]
#[path = "module_anchor_tests.rs"]
mod tests;
