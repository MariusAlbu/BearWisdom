// =============================================================================
// engine/rules/ref_module — module-qualified resolution via `r.module`
//
// The extractor recorded an explicit module prefix on the ref. A module
// spelling denotes either the entity the ref names or the container it lives
// in, so three probes run in order: the module spelling AS the declaration,
// the target as a member under the module, then a same-named candidate whose
// file path stem matches the module name.
//
// Declines immediately when no `module` field is set on the ref.
// =============================================================================

use crate::indexer::resolve::engine::contract::Symbol;
use crate::indexer::resolve::engine::support::{index_qname_leaf, path_stem_matches};
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};

/// The declaration the ref's own module spelling names. The ref's target being
/// the leaf of a QUALIFIED spelling is the evidence that `module` denotes the
/// entity rather than a container — a receiver-valued `module` (`conn.assigns`
/// under target `flash`) fails the leaf test, and a bare spelling equal to the
/// target carries no evidence at all (a separator-less target is the bare-name
/// rungs' job). A declaration's own header names itself without referring to
/// itself, so the source symbol is never the hit.
fn entity_probe<'a>(
    ctx: &'a BinderContext<'_>,
    module: &str,
    indexed_module: &str,
) -> Option<&'a Symbol> {
    if !ctx.profile.is_qualified_name(module) || index_qname_leaf(indexed_module) != ctx.target() {
        return None;
    }
    let sym = ctx.lookup.by_qualified_name(indexed_module)?;
    if ctx.ref_ctx.source_symbol_id == Some(sym.id) {
        return None;
    }
    (ctx.kind)(ctx.edge_kind(), &sym.kind).then_some(sym)
}

/// The member the ref names under its module — `{module}.{target}` in the
/// canonical index spelling.
fn container_probe<'a>(ctx: &'a BinderContext<'_>, module: &str) -> Option<&'a Symbol> {
    let qname = ctx.profile.index_qname_join(module, ctx.target());
    let sym = ctx.lookup.by_qualified_name(&qname)?;
    (ctx.kind)(ctx.edge_kind(), &sym.kind).then_some(sym)
}

/// A same-named candidate whose file path stem matches the module spelling or
/// its leaf — the module names a file rather than an index qname.
fn file_stem_probe<'a>(ctx: &'a BinderContext<'_>, indexed_module: &str) -> Option<&'a Symbol> {
    let edge_kind = ctx.edge_kind();
    let module_lower = indexed_module.to_lowercase();
    let last_seg_lower = index_qname_leaf(indexed_module).to_lowercase();
    for sym in ctx.lookup.by_name(ctx.target()) {
        if !(ctx.kind)(edge_kind, &sym.kind) {
            continue;
        }
        let file_lower = sym.file_path.to_lowercase();
        if path_stem_matches(&file_lower, &module_lower)
            || path_stem_matches(&file_lower, &last_seg_lower)
        {
            return Some(sym);
        }
    }
    None
}

pub struct RefModuleRule;

impl LookupRule for RefModuleRule {
    fn name(&self) -> &'static str {
        "ref_module"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let Some(module) = ctx.r().module.as_deref() else {
            return LookupResult::Pass;
        };
        let indexed_module = ctx.profile.index_qname_from_source(module);

        let hit = entity_probe(ctx, module, &indexed_module)
            .or_else(|| container_probe(ctx, module))
            .or_else(|| file_stem_probe(ctx, &indexed_module));

        match hit {
            Some(sym) => LookupResult::Resolved(ctx.resolved(sym.id, "default_ref_module")),
            None => LookupResult::Pass,
        }
    }
}

#[cfg(test)]
#[path = "ref_module_tests.rs"]
mod tests;
