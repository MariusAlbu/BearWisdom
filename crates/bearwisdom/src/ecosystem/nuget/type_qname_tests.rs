// Tests for `type_qname.rs`.
//
// `join_type_name` is the whole qualification rule and takes plain strings, so
// it is exercised directly. `assembly_type_defs` and `qualified_type_name`
// need a real `CilObject`; their coverage lives in
// `tests/tests/dotnet_externals.rs`, which indexes actual NuGet DLLs.

use super::join_type_name;

#[test]
fn nested_chain_takes_the_outermost_namespace() {
    let enclosing = vec!["JSType".to_string()];
    let (qname, scope) = join_type_name(
        "System.Runtime.InteropServices.JavaScript",
        &enclosing,
        "Any",
    );
    assert_eq!(
        qname,
        "System.Runtime.InteropServices.JavaScript.JSType.Any"
    );
    assert_eq!(
        scope.as_deref(),
        Some("System.Runtime.InteropServices.JavaScript.JSType")
    );
}

#[test]
fn nested_chain_keeps_every_enclosing_segment_in_order() {
    let enclosing = vec!["Outer".to_string(), "Middle".to_string()];
    let (qname, scope) = join_type_name("Acme.Lib", &enclosing, "Inner");

    assert_eq!(qname, "Acme.Lib.Outer.Middle.Inner");
    assert_eq!(scope.as_deref(), Some("Acme.Lib.Outer.Middle"));
}

#[test]
fn namespaced_top_level_type_qualifies_on_its_namespace() {
    let (qname, scope) = join_type_name("Acme.Lib", &[], "Widget");

    assert_eq!(qname, "Acme.Lib.Widget");
    assert_eq!(scope.as_deref(), Some("Acme.Lib"));
}

#[test]
fn namespaceless_top_level_type_keeps_its_bare_name() {
    let (qname, scope) = join_type_name("", &[], "Widget");

    assert_eq!(qname, "Widget");
    assert_eq!(scope, None);
}

#[test]
fn namespaceless_nested_chain_qualifies_on_the_enclosing_types_alone() {
    let enclosing = vec!["Outer".to_string()];
    let (qname, scope) = join_type_name("", &enclosing, "Inner");

    assert_eq!(qname, "Outer.Inner");
    assert_eq!(scope.as_deref(), Some("Outer"));
}
