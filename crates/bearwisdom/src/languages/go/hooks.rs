// =============================================================================
// languages/go/hooks.rs — GoHooks impl of LanguageEngineHooks.
//
// Phase 6 wave-A skeleton. Future migrations might extend the structural
// matcher to allow ParamShape compatibility (engine's current structural
// matcher checks name + kind only), expose embedded-type member surfacing,
// or wire go:generate output as synthesized members.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct GoHooks;

impl LanguageEngineHooks for GoHooks {}

pub static GO_HOOKS: GoHooks = GoHooks;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_checker::core::TypeArena;

    #[test]
    fn synthesize_members_returns_empty_by_default() {
        let arena = TypeArena::new();
        assert!(GoHooks.synthesize_members("Foo", &[], &arena).is_empty());
    }
}
