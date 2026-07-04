// =============================================================================
// engine/rules/same_file — bare target matches a sibling symbol in the same file
//
// For languages without explicit scope rules (Lua, Bash, OCaml at root, most
// scripting languages), a bare call to a function defined earlier or later in
// the same file is the canonical resolution. Yields to any explicit
// (non-wildcard) import that binds the same name — true lexical locals are
// already handled by scope_visible (which runs first).
//
// `self_keywords` and `name_normalization` are threaded in from the language
// profile; both are identity for case-sensitive languages, so the probe is
// byte-identical in the common case.
// =============================================================================

use crate::indexer::resolve::engine::support::{normalize_name, strip_self_keyword};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct SameFileRule;

impl LookupRule for SameFileRule {
    fn name(&self) -> &'static str {
        "same_file"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = strip_self_keyword(ctx.target(), ctx.profile.self_keywords);
        // Yield to an explicit (non-wildcard) import that binds this name. True
        // lexical locals are already handled by scope_visible (which runs first);
        // a file-level sibling that isn't in the scope chain must not shadow an
        // import that names the same thing. The name an import BINDS is its
        // alias when renamed (`use m::Orig as Bound` brings only `Bound` into
        // scope), else its imported name — a renamed import's ORIGINAL name is
        // free for a same-file declaration to claim.
        if !target.is_empty()
            && ctx
                .file_ctx
                .imports
                .iter()
                .any(|imp| !imp.is_wildcard && imp.bound_name() == target)
        {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let norm = ctx.profile.name_normalization;
        let target_norm = normalize_name(norm, target);
        for sym in ctx.lookup.in_file(&ctx.file_ctx.file_path) {
            if normalize_name(norm, &sym.name) == target_norm && (ctx.kind)(edge_kind, &sym.kind) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_same_file"));
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "same_file_tests.rs"]
mod tests;
