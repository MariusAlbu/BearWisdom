// =============================================================================
// engine/rules/module_anchor_terminal — halt the ladder after a missed anchor
//
// STOP GUARD. Mirrors the terminal check in `run_ladder` that follows
// `resolve_via_module_anchor`:
//
//   if pd.imports.module_anchor_terminal
//       && matches!(pd.imports.module_anchor, ModuleAnchor::On(_))
//       && ref.module.is_some()
//       && ref.kind != EdgeKind::Imports
//   {
//       return None;
//   }
//
// When the profile opts into module-anchored binding (`ModuleAnchor::On(…)`)
// and marks the anchor as terminal, a non-`Imports` ref that carries a module
// field but was NOT bound by `ModuleAnchorRule` must not fall through to the
// bare-name strategies. A same-named local symbol would silently hijack an
// external-prefix ref. The guard fires `Stop` to end the ladder honestly.
//
// When any condition is absent (terminal flag off, anchor off, no module on the
// ref, or an Imports edge kind) the guard passes through.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::ModuleAnchor;
use crate::types::EdgeKind;

pub struct ModuleAnchorTerminalRule;

impl LookupRule for ModuleAnchorTerminalRule {
    fn name(&self) -> &'static str {
        "module_anchor_terminal"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if ctx.profile.imports.module_anchor_terminal
            && matches!(ctx.profile.imports.module_anchor, ModuleAnchor::On(_))
            && ctx.r().module.is_some()
            && ctx.edge_kind() != EdgeKind::Imports
        {
            LookupResult::Stop
        } else {
            LookupResult::Pass
        }
    }
}

#[cfg(test)]
#[path = "module_anchor_terminal_tests.rs"]
mod tests;
