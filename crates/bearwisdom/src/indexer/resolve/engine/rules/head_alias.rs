// =============================================================================
// engine/rules/head_alias — dotted target's HEAD binds to an in-file symbol
//
// A dotted target (`foo.bar`) that starts with a HEAD segment which matches an
// in-file symbol resolves to that in-file symbol. Gated on
// `profile.head_alias`; `HeadAliasBind::Off` (the default) passes immediately.
//
// Declines when the head is empty or contains `_` — a `_`-bearing head is a
// provider resource type, not an alias. When `require_kind` is `Some`, the
// in-file symbol must match that kind; `None` accepts any kind-compatible
// symbol.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::HeadAliasBind;

pub struct HeadAliasRule;

impl LookupRule for HeadAliasRule {
    fn name(&self) -> &'static str {
        "head_alias"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let HeadAliasBind::OnSameFile { require_kind } = ctx.profile.head_alias else {
            return LookupResult::Pass;
        };
        let target = ctx.target();
        let Some(dot) = target.find('.') else {
            return LookupResult::Pass;
        };
        let head = &target[..dot];
        if head.is_empty() || head.contains('_') {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        for sym in ctx.lookup.in_file(&ctx.file_ctx.file_path) {
            if sym.name != head {
                continue;
            }
            if let Some(req) = require_kind {
                if sym.kind != req {
                    continue;
                }
            }
            if (ctx.kind)(edge_kind, &sym.kind) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_head_alias"));
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "head_alias_tests.rs"]
mod tests;
