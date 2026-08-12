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
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    ModuleAnchor, ModuleAnchorBind, ModulePrefixRewrites, RelativeMarker, StemSource,
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
        let sep = ctx.profile.qname_separator;
        let rewrites = ctx.profile.imports.module_prefix_rewrites;
        let overload_pick_all = ctx.profile.overload_pick_all;

        let is_relative = match ctx.profile.imports.relative_marker {
            RelativeMarker::None => true,
            RelativeMarker::DotPrefix => module.starts_with('.'),
            RelativeMarker::DotSlashPrefix => {
                module.starts_with("./") || module.starts_with("../")
            }
        };
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
                // qname probe: {prefix}{sep}{target} for each module-prefix
                // candidate under every separator the index might use.
                for prefix in module_prefix_candidates(module, rewrites) {
                    for s in module_anchor_separators(sep) {
                        let qname = format!("{prefix}{s}{target}");
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
                }
                // Directory-containment fallback: map the module to a path
                // fragment and match against file_path.  Bare specifiers under
                // a `decline_bare_directory_match` rewrite axis are declined so
                // a package name never matches a same-named project file.
                if declines_bare_directory_match(module, rewrites) {
                    return LookupResult::Pass;
                }
                let module_as_path = separators_to_slash(module, sep);
                let leaf = module_leaf(module, sep).to_lowercase();
                let candidates = ctx.lookup.by_name(target);
                // Two passes: top-level declarations (qname == bare target)
                // first, member declarations second. A module-qualified target
                // names a top-level item of that module, so `mod::f` must not
                // bind a member `T.f` when the located file set also declares a
                // top-level `f`.
                for top_level_pass in [true, false] {
                    for sym in &candidates {
                        if (sym.qualified_name == target) != top_level_pass {
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
                        if leaf != module_as_path
                            && path_stem_matches(&path.to_lowercase(), &leaf)
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
                    StemSource::ModuleLeaf => {
                        module.rsplit('.').next().unwrap_or(module).to_lowercase()
                    }
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

/// The qname-probe separators: always `.` (the universal index join), plus the
/// profile separator when it differs. A `::`-keyed module probes both
/// `crate::db.new` and `crate::db::new`; a `.`-keyed one probes once.
fn module_anchor_separators(sep: &str) -> impl Iterator<Item = &str> {
    [".", sep].into_iter().take(if sep == "." { 1 } else { 2 })
}

/// Map every module-path separator (`.` and the profile's, e.g. `::`) to `/`
/// for file-path containment matching. `crate::db` → `crate/db`.
fn separators_to_slash(module: &str, sep: &str) -> String {
    let dotted = if sep == "." {
        module.to_string()
    } else {
        module.replace(sep, ".")
    };
    dotted.replace('.', "/")
}

/// The trailing path-segment of a module under either separator — the file-like
/// leaf used as a containment fallback. `crate::db` → `db`, `a.b.c` → `c`.
fn module_leaf<'a>(module: &'a str, sep: &str) -> &'a str {
    let after_profile = if sep != "." {
        module.rsplit(sep).next().unwrap_or(module)
    } else {
        module
    };
    after_profile.rsplit('.').next().unwrap_or(after_profile)
}

/// A bare module specifier names a package, not a project-relative path: it
/// does not start with `.` or `/`, and is not a Windows drive path (`C:/…`).
fn is_bare_module_specifier(spec: &str) -> bool {
    !spec.starts_with('.')
        && !spec.starts_with('/')
        && !(spec.len() >= 2 && spec.as_bytes()[1] == b':')
}

/// The ordered module-prefix candidates the `ByNameUnderModuleDir` anchor
/// probes: the literal `module` first, then — only for a bare specifier under
/// `ModulePrefixRewrites::On` — DefinitelyTyped `@types/` rewrites and
/// deep-import `/seg` peels. A relative specifier or `Off` yields only the
/// literal module.
fn module_prefix_candidates(module: &str, rewrites: ModulePrefixRewrites) -> Vec<String> {
    let mut out = vec![module.to_string()];
    let ModulePrefixRewrites::On {
        definitely_typed,
        deep_import_peel,
        ..
    } = rewrites
    else {
        return out;
    };
    if !is_bare_module_specifier(module) {
        return out;
    }
    if definitely_typed && !module.starts_with("@types/") {
        // `@scope/pkg` → `@types/scope__pkg`; `pkg` → `@types/pkg`.
        if let Some(rest) = module.strip_prefix('@') {
            if let Some(slash) = rest.find('/') {
                let scope = &rest[..slash];
                let pkg = &rest[slash + 1..];
                out.push(format!("@types/{scope}__{pkg}"));
            }
        } else {
            out.push(format!("@types/{module}"));
        }
    }
    if deep_import_peel && module.contains('/') {
        // Strip trailing `/seg` segments, stopping before a bare `@scope`.
        let mut path = module;
        while let Some(slash) = path.rfind('/') {
            let parent = &path[..slash];
            if parent.starts_with('@') && !parent.contains('/') {
                break;
            }
            path = parent;
            out.push(path.to_string());
        }
    }
    out
}

/// Whether the directory-containment fallback of `ByNameUnderModuleDir` is
/// declined — true only for a bare specifier when the rewrites axis sets
/// `decline_bare_directory_match`.
fn declines_bare_directory_match(module: &str, rewrites: ModulePrefixRewrites) -> bool {
    matches!(
        rewrites,
        ModulePrefixRewrites::On {
            decline_bare_directory_match: true,
            ..
        }
    ) && is_bare_module_specifier(module)
}

#[cfg(test)]
#[path = "module_anchor_tests.rs"]
mod tests;
