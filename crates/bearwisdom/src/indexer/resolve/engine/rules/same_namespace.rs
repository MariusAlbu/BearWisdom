// =============================================================================
// engine/rules/same_namespace — same-file-namespace lookup
//
// In C#, types in the same namespace are visible without a `using`. When the
// source file declares namespace X, a candidate whose qname is `X.{target}` is
// in scope. A case-folding fallback handles languages whose call-site casing
// may differ from the declared member name; a case-sensitive language uses the
// byte-exact probe alone.
// =============================================================================

use crate::indexer::resolve::engine::support::normalize_name;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::NameNormalization;

pub struct SameNamespaceRule;

impl LookupRule for SameNamespaceRule {
    fn name(&self) -> &'static str {
        "same_namespace"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        let norm = ctx.profile.name_normalization;

        let ns = match ctx.file_ctx.file_namespace.as_deref() {
            Some(ns) if !ns.is_empty() => ns,
            _ => return LookupResult::Pass,
        };

        let expected = format!("{ns}.{target}");

        // Byte-exact probe via `by_name`: fast path for case-sensitive languages.
        for sym in ctx.lookup.by_name(target) {
            if sym.qualified_name == expected && (ctx.kind)(edge_kind, &sym.kind) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_same_namespace"));
            }
        }

        // Case-folding fallback: scan all members of the namespace and accept
        // one whose qname folds equal to `{ns}.{target}`. Gated on a non-identity
        // spec — a case-sensitive language never reaches this path.
        if !matches!(norm, NameNormalization::None) {
            let expected_norm = normalize_name(norm, &expected);
            for sym in ctx.lookup.in_namespace(ns) {
                if normalize_name(norm, &sym.qualified_name) == expected_norm
                    && (ctx.kind)(edge_kind, &sym.kind)
                {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "default_same_namespace"));
                }
            }
        }

        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "same_namespace_tests.rs"]
mod tests;
