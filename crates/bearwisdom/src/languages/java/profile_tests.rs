use super::*;
use crate::type_checker::profile::language_profile::{DispatchAxis, SupertypeDiscovery};

#[test]
fn id_matches() {
    assert_eq!(JAVA_PROFILE.id, "java");
}

#[test]
fn structural_choices() {
    assert_eq!(
        JAVA_PROFILE.supertype_discovery,
        SupertypeDiscovery::Explicit
    );
    assert_eq!(JAVA_PROFILE.dispatch_axis, DispatchAxis::Receiver);
    assert!(JAVA_PROFILE.has_generics);
    // java.util.Optional is a class with explicit unwrap methods; engine
    // must NOT peel it.
    assert!(!JAVA_PROFILE.look_through_optional);
}

#[test]
fn this_and_super_are_self_keywords() {
    assert!(JAVA_PROFILE.self_keywords.contains(&"this"));
    assert!(JAVA_PROFILE.self_keywords.contains(&"super"));
}

#[test]
fn primitives_include_jvm_built_in_types() {
    let names: Vec<&str> = JAVA_PROFILE
        .primitive_mapping
        .iter()
        .map(|(n, _)| *n)
        .collect();
    for canonical in ["int", "long", "boolean", "void", "String"] {
        assert!(
            names.contains(&canonical),
            "missing Java primitive: {canonical}"
        );
    }
}
