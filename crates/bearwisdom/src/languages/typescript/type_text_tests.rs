use super::TypeScriptPlugin;
use crate::languages::LanguagePlugin;
use crate::type_checker::core::types::{Type, TypeArena};

#[test]
fn type_text_policy_keeps_python_and_go_callable_spellings_opaque() {
    let plugin = TypeScriptPlugin;
    let arena = TypeArena::new();

    let python_callable = plugin.intern_type_text(&arena, "Callable[[Input], Output]");
    assert!(
        matches!(arena.get(python_callable), Type::Class(name) if name == "Callable[[Input], Output]")
    );

    let go_callable = plugin.intern_type_text(&arena, "func(Input) Output");
    assert!(matches!(arena.get(go_callable), Type::Class(name) if name == "func(Input) Output"));

    let thin_arrow = plugin.intern_type_text(&arena, "(Input) -> Output");
    assert!(matches!(arena.get(thin_arrow), Type::Class(name) if name == "(Input) -> Output"));
}

#[test]
fn type_text_policy_keeps_typescript_forms_structural() {
    let plugin = TypeScriptPlugin;
    let arena = TypeArena::new();

    let callback = plugin.intern_type_text(&arena, "(input: Item) => Result");
    assert!(matches!(arena.get(callback), Type::Function { .. }));

    let union = plugin.intern_type_text(&arena, "readonly Item[] | Missing");
    assert!(matches!(arena.get(union), Type::Union(_)));
}
