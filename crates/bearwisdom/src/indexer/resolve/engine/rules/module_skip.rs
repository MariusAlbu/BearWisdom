// =============================================================================
// engine/rules/module_skip — decline any ref whose module field is rejected
//
// STOP GUARD. Mirrors the top-of-ladder check in `run_ladder`:
//
//   if let (Some(skip), Some(module)) = (pd.module_skip, ref.module) {
//       if skip(module) { return None }
//   }
//
// When the ref's extractor-set `module` names a non-project provider and the
// profile supplies a `module_skip` predicate that returns `true` for it, the
// ladder is halted (`Stop`) before any binding strategy can fire. This prevents
// a same-named project symbol from hijacking an externally-declared import.
//
// When either condition is absent (no predicate, or no module on the ref) the
// guard passes through to the next rule.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct ModuleSkipRule;

impl LookupRule for ModuleSkipRule {
    fn name(&self) -> &'static str {
        "module_skip"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let Some(skip) = ctx.profile.module_skip else {
            return LookupResult::Pass;
        };
        let Some(module) = ctx.r().module.as_deref() else {
            return LookupResult::Pass;
        };
        if skip(module) {
            LookupResult::Stop
        } else {
            LookupResult::Pass
        }
    }
}

#[cfg(test)]
#[path = "module_skip_tests.rs"]
mod tests;
