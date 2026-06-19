// =============================================================================
// prolog/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// Prolog predicates are a flat top-level name/arity namespace. A rule-body goal
// calls a sibling predicate bare by functor; the extractor emits it as a Calls
// ref with target_name = bare functor (symbol.name = bare functor,
// qualified_name = functor/arity). With no scope/import structure to bind
// through, `namespaceless_global_type_lookup` binds such a bare call first-match
// to the project's own predicate definition via the dead-last by-name rung.
// A same-named library predicate with no project definition declines and stays
// external; a module-qualified `lists:member` call carries the `:` in its
// target_name, so the bare-name rung never sees it.
// =============================================================================

use super::PROLOG_PROFILE;

#[test]
fn prolog_profile_identity_and_shadow_mode() {
    assert_eq!(PROLOG_PROFILE.id, "prolog");
    assert_eq!(PROLOG_PROFILE.qname_separator, ":");
}

#[test]
fn prolog_namespaceless_global_is_on() {
    // Prolog's flat predicate namespace binds bare functor calls to the
    // project's own predicate definitions via the dead-last by-name rung.
    assert_eq!(
        PROLOG_PROFILE.namespaceless_global_type_lookup,
        crate::type_checker::profile::language_profile::NamespaceScope::Global
    );
}

