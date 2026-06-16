// =============================================================================
// engine/rules/qname_exact — exact qualified-name match on a dotted target
//
// A dotted target (`Catalog.Service.List`, `tokio::runtime::spawn`, `a/b/c`) is
// the syntactic shape the extractor produced for a qualified call; if it matches
// a stored qname exactly, that is the answer. Declines a bare (separator-less)
// target — that is the bare-name rules' job.
//
// `overload_pick_all` scans every overload under the qname for the first
// kind-compatible one (declaration merging exposes interface + variable under
// one qname). The case-folding fallback only fires for a name-folding language;
// a case-sensitive language is the byte-exact probe alone.
// =============================================================================

use crate::indexer::resolve::engine::support::normalize_name;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::NameNormalization;

pub struct QnameExactRule;

impl LookupRule for QnameExactRule {
    fn name(&self) -> &'static str {
        "qname_exact"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        if !target.contains('.') && !target.contains("::") && !target.contains('/') {
            return LookupResult::Pass;
        }

        // Declaration-merging: scan every overload for the first kind-compatible
        // one rather than `by_qualified_name`'s first-wins pick.
        if ctx.profile.overload_pick_all {
            for sym in ctx.lookup.all_by_qualified_name(target) {
                if (ctx.kind)(edge_kind, &sym.kind) {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "default_qname_exact"));
                }
            }
            return LookupResult::Pass;
        }

        if let Some(sym) = ctx.lookup.by_qualified_name(target) {
            if (ctx.kind)(edge_kind, &sym.kind) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_qname_exact"));
            }
        }

        // Case-folding fallback: a folding language whose call site differs only
        // in surface case from the keyed qname misses the byte-exact probe.
        // Scan the leaf's `by_name` candidates and accept one whose whole qname
        // folds equal to the target. `NameNormalization::None` borrows
        // identically on both sides, so this never fires for a case-sensitive
        // language.
        let norm = ctx.profile.name_normalization;
        if !matches!(norm, NameNormalization::None) {
            let leaf = target.rsplit(['.', ':', '/']).next().unwrap_or(target);
            let target_norm = normalize_name(norm, target);
            for sym in ctx.lookup.by_name(leaf) {
                if normalize_name(norm, &sym.qualified_name) == target_norm
                    && (ctx.kind)(edge_kind, &sym.kind)
                {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "default_qname_exact"));
                }
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "qname_exact_tests.rs"]
mod tests;
