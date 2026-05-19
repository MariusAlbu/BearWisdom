// =============================================================================
// languages/csharp/hooks.rs — CSharpHooks impl of LanguageEngineHooks.
//
// Phase 6 wave-A skeleton. Future migrations: synthesize_members for
// record canonical constructors / `with`-expressions, `[ObservableProperty]`
// MVVM Toolkit source-generated accessors, EF Core `DbSet<T>` accessor
// generation, extension method discovery (extension methods are static
// methods on a static class with `this T` first parameter — engine could
// register them as extensions on T).
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct CSharpHooks;

impl LanguageEngineHooks for CSharpHooks {}

pub static CSHARP_HOOKS: CSharpHooks = CSharpHooks;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_checker::core::TypeArena;

    #[test]
    fn synthesize_members_returns_empty_by_default() {
        let arena = TypeArena::new();
        assert!(CSharpHooks.synthesize_members("Foo", &[], &arena).is_empty());
    }
}
