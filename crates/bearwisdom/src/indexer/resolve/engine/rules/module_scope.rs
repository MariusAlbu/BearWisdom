// =============================================================================
// engine/rules/module_scope — bare same-module reference without an import
//
// Gated on `profile.imports.module_scope` (off by default).  Dispatches to one of three
// sub-strategies that differ in how the "module boundary" is defined:
//
//   SameDir             — the source file's immediate parent directory is the
//                         module; binds the FIRST kind-compatible same-dir
//                         candidate (Odin, MATLAB: a same-name duplicate is a
//                         compile error, so first is the only match).
//   SameDirUnique       — same directory boundary, but binds only when EXACTLY
//                         ONE distinct internal kind-compatible declaration
//                         survives after dedup.
//   SourcesTargetSubtree — SwiftPM whole-module layout: every file under one
//                         `Sources/<Target>/` or `Tests/<Target>/` subtree is in
//                         the same module.  Binds iff EXACTLY ONE candidate
//                         survives after (qualified_name, kind) dedup.
//
// Declines on an empty, dotted, `::`, or `/`-bearing target.  Runs late in the
// ladder so any real import or structural rule wins first.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::indexer::resolve::engine::contract::Symbol;
use crate::type_checker::profile::language_profile::ModuleScope;

pub struct ModuleScopeRule;

impl LookupRule for ModuleScopeRule {
    fn name(&self) -> &'static str {
        "module_scope"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        match ctx.profile.imports.module_scope {
            ModuleScope::Off => LookupResult::Pass,
            ModuleScope::SameDir => same_dir(ctx),
            ModuleScope::SameDirUnique => same_dir_unique(ctx),
            ModuleScope::SourcesTargetSubtree => sources_target_subtree(ctx),
        }
    }
}

// =============================================================================
// Sub-strategy implementations
// =============================================================================

/// `SameDir`: bind the first kind-compatible symbol in the same immediate
/// parent directory. No uniqueness requirement — a same-name duplicate is a
/// compile error in the target languages.
fn same_dir(ctx: &BinderContext) -> LookupResult {
    let target = ctx.target();
    if target.is_empty()
        || target.contains('.')
        || target.contains("::")
        || target.contains('/')
    {
        return LookupResult::Pass;
    }
    let edge_kind = ctx.edge_kind();
    let Some(src_parent) = parent_dir_basename(&ctx.file_ctx.file_path) else {
        return LookupResult::Pass;
    };
    for sym in ctx.lookup.by_name(target) {
        if !(ctx.kind)(edge_kind, &sym.kind) {
            continue;
        }
        if parent_dir_basename(&sym.file_path).as_deref() == Some(src_parent.as_str()) {
            return LookupResult::Resolved(ctx.resolved(sym.id, "default_same_dir"));
        }
    }
    LookupResult::Pass
}

/// `SameDirUnique`: same directory boundary as `SameDir`, but binds only when
/// EXACTLY ONE distinct internal kind-compatible declaration survives.
fn same_dir_unique(ctx: &BinderContext) -> LookupResult {
    let target = ctx.target();
    if target.is_empty()
        || target.contains('.')
        || target.contains("::")
        || target.contains('/')
    {
        return LookupResult::Pass;
    }
    let edge_kind = ctx.edge_kind();
    let Some(src_parent) = parent_dir_basename(&ctx.file_ctx.file_path) else {
        return LookupResult::Pass;
    };
    let mut compatible: Vec<&Symbol> = ctx
        .lookup
        .by_name(target)
        .into_iter()
        .filter(|sym| !ctx.lookup.is_external_file(&sym.file_path))
        .filter(|sym| (ctx.kind)(edge_kind, &sym.kind))
        .filter(|sym| {
            parent_dir_basename(&sym.file_path).as_deref() == Some(src_parent.as_str())
        })
        .collect();
    compatible.sort_by(|a, b| {
        a.qualified_name
            .cmp(&b.qualified_name)
            .then(a.kind.cmp(&b.kind))
            .then(a.file_path.cmp(&b.file_path))
    });
    compatible.dedup_by(|a, b| {
        a.qualified_name == b.qualified_name && a.kind == b.kind && a.file_path == b.file_path
    });
    if compatible.len() != 1 {
        return LookupResult::Pass;
    }
    LookupResult::Resolved(ctx.resolved(compatible[0].id, "default_module_scope"))
}

/// `SourcesTargetSubtree`: SwiftPM whole-module case.  A candidate is in-module
/// iff it shares the source's `Sources/<seg>/` (or `Tests/<seg>/`) prefix.
/// Binds iff EXACTLY ONE candidate survives after (qualified_name, kind) dedup.
fn sources_target_subtree(ctx: &BinderContext) -> LookupResult {
    let target = ctx.target();
    if target.is_empty()
        || target.contains('.')
        || target.contains("::")
        || target.contains('/')
    {
        return LookupResult::Pass;
    }
    let edge_kind = ctx.edge_kind();
    let Some(src_prefix) = module_subtree_prefix(&ctx.file_ctx.file_path) else {
        return LookupResult::Pass;
    };
    let mut compatible: Vec<&Symbol> = ctx
        .lookup
        .by_name(target)
        .into_iter()
        .filter(|sym| !ctx.lookup.is_external_file(&sym.file_path))
        .filter(|sym| (ctx.kind)(edge_kind, &sym.kind))
        .filter(|sym| {
            module_subtree_prefix(&sym.file_path).as_deref() == Some(src_prefix.as_str())
        })
        .collect();
    // Dedup on (qualified_name, kind): one logical symbol indexed in multiple
    // rows is a single candidate for ambiguity purposes.
    compatible.sort_by(|a, b| {
        a.qualified_name
            .cmp(&b.qualified_name)
            .then(a.kind.cmp(&b.kind))
    });
    compatible.dedup_by(|a, b| a.qualified_name == b.qualified_name && a.kind == b.kind);
    if compatible.len() != 1 {
        return LookupResult::Pass;
    }
    LookupResult::Resolved(ctx.resolved(compatible[0].id, "default_module_scope"))
}

// =============================================================================
// Private helpers — used only by this rule
// =============================================================================

/// The basename of a file path's immediate parent directory. Path separators
/// are normalized to `/`. Returns `None` when the path has no parent directory.
/// For `pkg/foo/bar.odin` returns `Some("foo")`.
fn parent_dir_basename(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    let (dir, _file) = normalized.rsplit_once('/')?;
    Some(dir.rsplit('/').next().unwrap_or(dir).to_string())
}

/// The SwiftPM module-subtree prefix of a file path: the substring up to and
/// including `Sources/<seg>/` (or `Tests/<seg>/`). Returns `None` when the
/// path has no such prefix.
fn module_subtree_prefix(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    let segs: Vec<&str> = normalized.split('/').collect();
    // Require a root segment, a target segment, and at least one more (the
    // file) so the prefix is `<root>/<target>/`.
    segs.iter().enumerate().find_map(|(i, seg)| {
        if (*seg == "Sources" || *seg == "Tests") && i + 2 < segs.len() {
            Some(segs[..=i + 1].join("/") + "/")
        } else {
            None
        }
    })
}

#[cfg(test)]
#[path = "module_scope_tests.rs"]
mod tests;
