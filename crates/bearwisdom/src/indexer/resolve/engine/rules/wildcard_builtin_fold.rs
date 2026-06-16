// =============================================================================
// engine/rules/wildcard_builtin_fold — fold a wildcard-builtin family
// target to its ambient base and retry the ambient-package probe
//
// Some SDKs expose a family of generated operations under a shared prefix
// (e.g. `listConnectionStrings`, `listKeys`, `listSecrets`). Rather than
// enumerating every member, the profile declares a `WildcardBuiltin` whose
// `prefix` matches the shared root and `fold_to` names the single ambient
// symbol that stands for the whole family. A target matching `prefix` +
// ASCII-uppercase folds to `fold_to` and retries the ambient-scope lookup.
//
// Gated on `profile.wildcard_builtins`: an empty slice (the default) returns
// Pass immediately. Below the concrete ambient rungs so an enumerated
// same-named builtin binds first.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct WildcardBuiltinFoldRule;

impl LookupRule for WildcardBuiltinFoldRule {
    fn name(&self) -> &'static str {
        "wildcard_builtin_fold"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if ctx.profile.wildcard_builtins.is_empty() {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        let Some(leaf) = ctx
            .profile
            .wildcard_builtins
            .iter()
            .find_map(|wb| wb.fold(target))
        else {
            return LookupResult::Pass;
        };
        super::ambient_scope::resolve_ambient_named(ctx, leaf, "wildcard_builtin_fold")
    }
}

#[cfg(test)]
#[path = "wildcard_builtin_fold_tests.rs"]
mod tests;
