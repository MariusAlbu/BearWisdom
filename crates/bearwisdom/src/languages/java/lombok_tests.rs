// =============================================================================
// java/lombok_tests.rs — Lombok accessor synthesis.
//
// Tests run the real Java extractor so the recognizer is exercised against
// genuine annotation refs + field signatures, then call the recognizer on that
// output.
// =============================================================================

use super::extract::extract;
use super::lombok::synthesize_lombok_accessors;

/// Synthesize accessors for `source`, returning sorted (qualified_name, signature) pairs.
fn synth(source: &str) -> Vec<(String, String)> {
    let r = extract(source);
    let mut v: Vec<(String, String)> = synthesize_lombok_accessors(source, &r.symbols, &r.refs)
        .into_iter()
        .map(|s| (s.qualified_name, s.signature.unwrap_or_default()))
        .collect();
    v.sort();
    v
}

/// Just the synthesized qualified names, sorted.
fn qnames(source: &str) -> Vec<String> {
    synth(source).into_iter().map(|(q, _)| q).collect()
}

#[test]
fn data_synthesizes_getters_and_setters() {
    let got = synth(
        "@Data\npublic class User {\n    private String name;\n    private int age;\n}",
    );
    assert!(
        got.contains(&("User.getName".to_string(), "String getName()".to_string())),
        "{got:?}"
    );
    assert!(
        got.contains(&("User.setName".to_string(), "void setName(String name)".to_string())),
        "{got:?}"
    );
    assert!(
        got.contains(&("User.getAge".to_string(), "int getAge()".to_string())),
        "{got:?}"
    );
    assert!(
        got.contains(&("User.setAge".to_string(), "void setAge(int age)".to_string())),
        "{got:?}"
    );
    assert_eq!(got.len(), 4, "exactly four accessors expected: {got:?}");
}

#[test]
fn getter_annotation_yields_only_getters() {
    assert_eq!(
        qnames("@Getter public class User { private String name; }"),
        vec!["User.getName".to_string()]
    );
}

#[test]
fn setter_annotation_yields_only_setters() {
    assert_eq!(
        qnames("@Setter public class User { private String name; }"),
        vec!["User.setName".to_string()]
    );
}

#[test]
fn value_yields_only_getters() {
    // @Value is immutable — getters, no setters.
    assert_eq!(
        qnames("@Value public class Point { private int x; }"),
        vec!["Point.getX".to_string()]
    );
}

#[test]
fn boolean_primitive_uses_is_prefix() {
    let q = qnames("@Data public class Flag { private boolean active; }");
    assert!(q.contains(&"Flag.isActive".to_string()), "{q:?}");
    assert!(q.contains(&"Flag.setActive".to_string()), "{q:?}");
    assert!(!q.contains(&"Flag.getActive".to_string()), "{q:?}");
}

#[test]
fn boolean_wrapper_uses_get_prefix() {
    // A `Boolean` (wrapper) field uses get, not is — only the primitive is special.
    let q = qnames("@Getter public class Flag { private Boolean active; }");
    assert_eq!(q, vec!["Flag.getActive".to_string()]);
}

#[test]
fn explicit_method_wins_over_synthesized() {
    let q = qnames(
        "@Data public class User {\n    private String name;\n    public String getName() { return name; }\n}",
    );
    // getName already declared → not synthesized; setName still synthesized.
    assert!(!q.contains(&"User.getName".to_string()), "{q:?}");
    assert!(q.contains(&"User.setName".to_string()), "{q:?}");
}

#[test]
fn combined_getter_and_setter_annotations() {
    let q = qnames("@Getter\n@Setter\npublic class User { private String name; }");
    assert!(q.contains(&"User.getName".to_string()), "{q:?}");
    assert!(q.contains(&"User.setName".to_string()), "{q:?}");
}

#[test]
fn synthesized_accessors_carry_package_qname() {
    let q = qnames(
        "package com.example.model;\n@Data public class User { private String name; }",
    );
    assert!(q.contains(&"com.example.model.User.getName".to_string()), "{q:?}");
    assert!(q.contains(&"com.example.model.User.setName".to_string()), "{q:?}");
}

#[test]
fn no_lombok_yields_nothing() {
    assert!(synth("public class Plain { private String name; }").is_empty());
}

#[test]
fn field_type_colliding_with_lombok_name_is_not_an_annotation() {
    // `private Value total;` is a field whose TYPE is named Value — NOT a
    // @Value annotation. Java attributes the field's type to the enclosing
    // class, so a class-sourced "Value" TypeRef exists; the `@`-byte-offset
    // discriminator keeps it from being read as an annotation.
    assert!(
        synth("public class Order { private Value total; }").is_empty(),
        "a field type named like a Lombok annotation must not trigger synthesis"
    );
}

#[test]
fn data_class_with_lombok_named_field_type_synthesizes_only_its_accessors() {
    // @Data drives synthesis; the field's Value type ref must not double-count.
    let q = qnames("@Data public class Order { private Value total; }");
    assert_eq!(
        q,
        vec!["Order.getTotal".to_string(), "Order.setTotal".to_string()]
    );
}
