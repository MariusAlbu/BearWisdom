// =============================================================================
// engine/rules/ambient_prefix_strip — strip a namespace alias prefix and
// retry the ambient-scope lookup
//
// A target like `sys.concat` or `az.resourceId` carries a profile-declared
// namespace alias prefix (`sys`, `az`) that is not a real module path — it's a
// local shorthand that resolves to an ambient symbol. Strip the prefix and
// delegate to the ambient-scope lookup by the bare leaf name.
//
// Gated on `profile.ambient_namespace_prefixes`: an empty slice (the default)
// returns Pass immediately so the rung is free for every non-opted language.
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};

pub struct AmbientPrefixStripRule;

impl LookupRule for AmbientPrefixStripRule {
    fn name(&self) -> &'static str {
        "ambient_prefix_strip"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if ctx.profile.ambient_namespace_prefixes.is_empty() {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        let Some((prefix, leaf)) = ctx.profile.split_source_qualified_name(target) else {
            return LookupResult::Pass;
        };
        if leaf.is_empty() || !ctx.profile.ambient_namespace_prefixes.contains(&prefix) {
            return LookupResult::Pass;
        }
        super::ambient_scope::resolve_ambient_named(ctx, leaf, "ambient_prefix_strip")
    }
}

#[cfg(test)]
#[path = "ambient_prefix_strip_tests.rs"]
mod tests;
