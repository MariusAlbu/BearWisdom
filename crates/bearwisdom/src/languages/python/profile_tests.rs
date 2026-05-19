use super::*;
use crate::type_checker::profile::language_profile::{
    DispatchAxis, KindCompatibility, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn id_matches_language_extractor() {
    assert_eq!(PYTHON_PROFILE.id, "python");
}

#[test]
fn structural_choices_match_python_semantics() {
    assert_eq!(PYTHON_PROFILE.supertype_discovery, SupertypeDiscovery::Explicit);
    assert_eq!(PYTHON_PROFILE.dispatch_axis, DispatchAxis::Receiver);
    assert!(PYTHON_PROFILE.has_generics);
    assert!(!PYTHON_PROFILE.has_sum_types);
    assert!(PYTHON_PROFILE.look_through_optional);
}

#[test]
fn self_and_cls_are_self_keywords() {
    assert!(PYTHON_PROFILE.self_keywords.contains(&"self"));
    assert!(PYTHON_PROFILE.self_keywords.contains(&"cls"));
}

#[test]
fn coroutine_and_awaitable_are_async_wrappers() {
    let aw = PYTHON_PROFILE.async_wrappers;
    assert!(aw.contains(&"Coroutine"));
    assert!(aw.contains(&"Awaitable"));
}

#[test]
fn primitives_include_python_built_in_types() {
    let names: Vec<&str> = PYTHON_PROFILE
        .primitive_mapping
        .iter()
        .map(|(n, _)| *n)
        .collect();
    for canonical in ["str", "int", "float", "bool", "bytes", "None"] {
        assert!(names.contains(&canonical), "missing Python primitive: {canonical}");
    }
}

#[test]
fn calls_kind_table_accepts_class_and_function() {
    // Python class objects ARE callable (instantiation); `Foo()` produces
    // a Foo. The Calls kind matrix must include Class.
    let t = PYTHON_PROFILE.kind_compatible_table;
    for kind in [SymbolKind::Class, SymbolKind::Function, SymbolKind::Method] {
        assert!(KindCompatibility::check(t, EdgeKind::Calls, kind));
    }
}
