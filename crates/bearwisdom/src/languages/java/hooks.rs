// =============================================================================
// languages/java/hooks.rs — JavaHooks impl of LanguageEngineHooks.
//
// Phase 6 wave-A skeleton. Future migrations: synthesize_members for
// `@Entity` JPA fields, record canonical constructors, Lombok-generated
// accessors (`@Data`, `@Builder`), Spring `@Configuration`'s @Bean methods.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct JavaHooks;

impl LanguageEngineHooks for JavaHooks {}

pub static JAVA_HOOKS: JavaHooks = JavaHooks;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_checker::core::TypeArena;

    #[test]
    fn synthesize_members_returns_empty_by_default() {
        let arena = TypeArena::new();
        assert!(JavaHooks.synthesize_members("Foo", &[], &arena).is_empty());
    }
}
