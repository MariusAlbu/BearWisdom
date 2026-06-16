// =============================================================================
// engine/rules/scope_visible — innermost enclosing scope wins
//
// Try `{scope}{sep}{target}` for each scope in the ref's scope chain (innermost
// first) and each separator. Catches a local symbol defined in the source
// symbol's own enclosing type / namespace / module before resolution widens to
// file or project scope.
//
// `separators` is always the universal `.` index join, plus the profile's own
// separator when it differs — so a `::`-keyed index resolves the same
// scope-visible members a `.`-keyed one does. `name_normalization` only fires
// its fallback for a folding language; a case-sensitive language is the exact
// probe alone.
// =============================================================================

use crate::indexer::resolve::engine::support::{normalize_name, strip_self_keyword};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::NameNormalization;

pub struct ScopeVisibleRule;

impl LookupRule for ScopeVisibleRule {
    fn name(&self) -> &'static str {
        "scope_visible"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = strip_self_keyword(ctx.target(), ctx.profile.self_keywords);
        let edge_kind = ctx.edge_kind();
        let norm = ctx.profile.name_normalization;
        let target_norm = normalize_name(norm, target);

        // The universal `.` join, plus the profile separator when it differs.
        let both = [".", ctx.profile.qname_separator];
        let separators: &[&str] = if ctx.profile.qname_separator == "." {
            &both[..1]
        } else {
            &both[..]
        };

        for scope in &ctx.ref_ctx.scope_chain {
            // Exact qname probe — byte-identical under every separator.
            for sep in separators {
                let qname = format!("{scope}{sep}{target}");
                if let Some(sym) = ctx.lookup.by_qualified_name(&qname) {
                    if (ctx.kind)(edge_kind, &sym.kind) {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_scope_visible"),
                        );
                    }
                }
            }
            // Normalized-name fallback: only when the language folds names, and
            // only after the exact probe missed. Compares the scope's members by
            // normalized name so a reference written in a different surface form
            // binds to the scope member.
            if !matches!(norm, NameNormalization::None) {
                for member in ctx.lookup.members_of(scope) {
                    if normalize_name(norm, &member.name) == target_norm
                        && (ctx.kind)(edge_kind, &member.kind)
                    {
                        return LookupResult::Resolved(
                            ctx.resolved(member.id, "default_scope_visible"),
                        );
                    }
                }
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "scope_visible_tests.rs"]
mod tests;
