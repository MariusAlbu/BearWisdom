// =============================================================================
// engine/rules/builtin_skip — drain any ref whose target is a language builtin
// =============================================================================
//
// DRAIN GUARD. Mirrors `module_skip`'s STOP guard, keyed on the ref's target
// name instead of its module:
//
//   if let Some(skip) = pd.builtin_skip {
//       if skip(ctx.target()) { return Drained }
//   }
//
// When the profile's `builtin_skip` predicate returns true for the ref's
// target — a scalar type, an intrinsic function, a reserved namespace prefix —
// the ladder halts before any binding strategy can fire. Unlike `Stop`, the
// halt is tagged `Drained`: the target names a language construct, not a
// missing project symbol, so downstream the ref is written to
// `unresolved_refs` with `drained=1` and excluded from the resolution-rate
// denominator instead of counting as a genuine miss.
//
// When no predicate is set, or the predicate declines the target, the guard
// passes through to the next rule.
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};

pub struct BuiltinSkipRule;

impl LookupRule for BuiltinSkipRule {
    fn name(&self) -> &'static str {
        "builtin_skip"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let Some(skip) = ctx.profile.builtin_skip else {
            return LookupResult::Pass;
        };
        if skip(ctx.target()) {
            LookupResult::Drained
        } else {
            LookupResult::Pass
        }
    }
}

#[cfg(test)]
#[path = "builtin_skip_tests.rs"]
mod tests;
