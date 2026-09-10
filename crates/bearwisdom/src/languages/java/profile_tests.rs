use super::*;
use crate::type_checker::profile::language_profile::{
    DelegateShape, DispatchAxis, KindCompatibility, SupertypeDiscovery,
};

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

#[test]
fn enum_member_is_not_a_call_target() {
    // Precision guard: in Java an enum value (`Color.RED`) is a value read,
    // not a construction — a `Calls` ref must never bind to an `EnumMember`.
    assert!(!KindCompatibility::check(
        JAVA_KIND_TABLE,
        crate::types::EdgeKind::Calls,
        crate::types::SymbolKind::EnumMember,
    ));
}

fn delegate_shape(name: &str) -> Option<DelegateShape> {
    JAVA_PROFILE
        .delegate_wrappers
        .iter()
        .find_map(|(candidate, shape)| (*candidate == name).then_some(*shape))
}

#[test]
fn generic_jdk_functional_interfaces_describe_callback_parameter_positions() {
    // Consumers, predicates, and fixed-primitive-return functions put every
    // callback parameter in a generic slot. The remaining listed interfaces
    // reserve their final generic slot for the callback return.
    for name in [
        "java.util.function.Consumer",
        "java.util.function.BiConsumer",
        "java.util.function.Predicate",
        "java.util.function.BiPredicate",
        "java.util.function.ToIntFunction",
        "java.util.function.ToLongFunction",
        "java.util.function.ToDoubleFunction",
        "java.util.function.ToIntBiFunction",
        "java.util.function.ToLongBiFunction",
        "java.util.function.ToDoubleBiFunction",
        "java.util.function.UnaryOperator",
    ] {
        assert_eq!(delegate_shape(name), Some(DelegateShape::AllParams));
    }
    for name in [
        "java.util.function.Function",
        "java.util.function.BiFunction",
        "java.util.function.Supplier",
    ] {
        assert_eq!(delegate_shape(name), Some(DelegateShape::LastIsReturn));
    }

    // An import spelling alone has no durable type identity, and a project
    // type with the same leaf must never be treated as a JDK callback wrapper.
    for name in ["Function", "com.acme.Function"] {
        assert_eq!(delegate_shape(name), None);
    }

    // These types have a fixed primitive input or repeat one generic input, so
    // DelegateShape cannot represent their complete parameter list soundly.
    for name in [
        "java.util.function.BinaryOperator",
        "java.util.function.IntConsumer",
        "java.util.function.IntFunction",
        "java.util.function.ObjIntConsumer",
    ] {
        assert_eq!(delegate_shape(name), None);
    }
}
