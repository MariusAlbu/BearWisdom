use super::{is_constructor_kind, is_shape_only_kind, is_type_kind, is_value_kind};

#[test]
fn type_kinds_are_class_like_declarations() {
    assert!(is_type_kind("class"));
    assert!(is_type_kind("interface"));
    assert!(is_type_kind("trait"));
    assert!(!is_type_kind("function"));
    assert!(!is_type_kind("namespace"));
    assert!(!is_type_kind("variable"));
}

#[test]
fn value_kinds_are_typed_bindings() {
    assert!(is_value_kind("variable"));
    assert!(is_value_kind("parameter"));
    assert!(is_value_kind("field"));
    assert!(is_value_kind("property"));
    assert!(!is_value_kind("class"));
    assert!(!is_value_kind("function"));
    assert!(!is_value_kind("namespace"));
}

/// The two classes are disjoint: no kind both owns a member set and is a
/// typed binding.
#[test]
fn type_and_value_kinds_are_disjoint() {
    for kind in [
        "class",
        "struct",
        "interface",
        "enum",
        "trait",
        "object",
        "record",
    ] {
        assert!(!is_value_kind(kind), "{kind} is a type kind");
    }
    for kind in [
        "variable",
        "constant",
        "const",
        "field",
        "property",
        "parameter",
    ] {
        assert!(!is_type_kind(kind), "{kind} is a value kind");
    }
}

/// A shape-only kind is a type kind whose name reaches no value: it can be
/// annotated with, never evaluated. Every other type kind binds a receiver its
/// own name denotes.
#[test]
fn shape_only_kinds_are_type_kinds_that_bind_no_value() {
    assert!(is_shape_only_kind("interface"));
    assert!(is_shape_only_kind("type_alias"));
    assert!(!is_shape_only_kind("class"));
    assert!(!is_shape_only_kind("enum"));
    assert!(!is_shape_only_kind("namespace"));
    for kind in [
        "variable",
        "constant",
        "const",
        "field",
        "property",
        "parameter",
    ] {
        assert!(!is_shape_only_kind(kind), "{kind} is a value kind");
    }
}

#[test]
fn constructor_kind_is_neither_a_type_nor_a_value() {
    assert!(is_constructor_kind("constructor"));
    assert!(!is_constructor_kind("function"));
    assert!(!is_constructor_kind("class"));
    assert!(!is_type_kind("constructor"));
    assert!(!is_value_kind("constructor"));
}
