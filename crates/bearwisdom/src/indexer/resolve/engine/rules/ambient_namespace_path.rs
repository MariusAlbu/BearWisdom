// =============================================================================
// engine/rules/ambient_namespace_path — dotted target whose leaf resolves
// under an ambient-namespace qname
//
// A TS `declare global { namespace Express { interface Multer { ... } } }` in
// `@types/multer` indexes members under qnames like
// `@types/multer.Express.Multer.File`. A ref written as `Express.Multer.File`
// doesn't match `qname_exact` because of the package prefix. Look up the leaf
// via `by_name` and accept the kind-compatible candidate whose qname ENDS WITH
// `.{target}`, picking the shallowest path (fewest `/` segments) as a
// tiebreaker. Only fires for a dotted target; a bare name is declined so the
// bare-name rules handle it.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct AmbientNamespacePathRule;

impl LookupRule for AmbientNamespacePathRule {
    fn name(&self) -> &'static str {
        "ambient_namespace_path"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        if !target.contains('.') {
            return LookupResult::Pass;
        }
        let leaf = target.rsplit('.').next().unwrap_or(target);
        let suffix = format!(".{target}");
        let candidates = ctx.lookup.by_name(leaf);
        let best = candidates
            .iter()
            .filter(|sym| sym.qualified_name.ends_with(&suffix))
            .filter(|sym| (ctx.kind)(edge_kind, &sym.kind))
            .min_by_key(|sym| sym.file_path.matches('/').count());
        match best {
            Some(sym) => LookupResult::Resolved(ctx.resolved(sym.id, "default_ambient_namespace_path")),
            None => LookupResult::Pass,
        }
    }
}

#[cfg(test)]
#[path = "ambient_namespace_path_tests.rs"]
mod tests;
