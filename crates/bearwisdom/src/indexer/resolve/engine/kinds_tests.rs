use super::{is_namespace_kind, is_type_kind, is_value_kind};

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
    for kind in ["class", "struct", "interface", "enum", "trait", "object", "record"] {
        assert!(!is_value_kind(kind), "{kind} is a type kind");
    }
    for kind in ["variable", "constant", "const", "field", "property", "parameter"] {
        assert!(!is_type_kind(kind), "{kind} is a value kind");
    }
}

#[test]
fn namespace_kinds_are_declaration_containers() {
    assert!(is_namespace_kind("namespace"));
    assert!(is_namespace_kind("module"));
    assert!(!is_namespace_kind("class"));
    assert!(!is_namespace_kind("function"));
    assert!(!is_namespace_kind("variable"));
}
