// Tests for the signature line shape.
//
// `format_method_signature`, `format_constructor_signature` and
// `format_parameter_list` take a real `CilObject` to resolve metadata-token
// references, so they are exercised end to end in
// `tests/tests/fsharp_dotnet_constructors.rs`, which indexes an actual DLL and
// asserts the persisted signature strings. What is checked here is the pure
// shaping the two renderers share: how a parameter list is joined and how a
// line is composed from name, generic suffix, parameters and return slot.

use super::{compose_signature_line, format_generic_suffix, join_parameter_list};

#[test]
fn constructor_signature_returns_the_declaring_type() {
    let params = join_parameter_list(&["string".to_string()], false);
    assert_eq!(
        compose_signature_line("Greeter", "", &params, "FakeExt.Greeter"),
        "Greeter(string): FakeExt.Greeter"
    );
}

#[test]
fn generic_constructor_carries_the_declaring_type_parameters() {
    let type_generic_names = vec!["TKey".to_string(), "TValue".to_string()];
    let params = join_parameter_list(&["int".to_string()], false);
    assert_eq!(
        compose_signature_line(
            "Dictionary",
            &format_generic_suffix(&type_generic_names),
            &params,
            "System.Collections.Generic.Dictionary"
        ),
        "Dictionary<TKey, TValue>(int): System.Collections.Generic.Dictionary"
    );
}

#[test]
fn method_signature_is_unchanged_by_the_parameter_extraction() {
    // Ordinary instance method.
    let params = join_parameter_list(&["string".to_string(), "int".to_string()], false);
    assert_eq!(params, "(string, int)");
    assert_eq!(
        compose_signature_line("Greet", "", &params, "string"),
        "Greet(string, int): string"
    );

    // Extension candidate: the synthesized receiver marker prefixes the first
    // parameter only.
    let ext = join_parameter_list(&["string".to_string(), "int".to_string()], true);
    assert_eq!(ext, "(this string, int)");
    assert_eq!(
        compose_signature_line(
            "Take",
            "<T>",
            &ext,
            "System.Collections.Generic.IEnumerable"
        ),
        "Take<T>(this string, int): System.Collections.Generic.IEnumerable"
    );

    // Parameterless method: the marker has nothing to attach to.
    assert_eq!(join_parameter_list(&[], true), "()");
    assert_eq!(
        compose_signature_line("ToString", "", "()", "string"),
        "ToString(): string"
    );
}
