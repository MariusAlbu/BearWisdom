use super::*;

#[test]
fn modules_have_type_namespace_syntax_and_function_returns_are_captured() {
    assert_eq!(FORMS.module, "mod_item");
    assert_eq!(FORMS.types.return_type, "return_type");
}
