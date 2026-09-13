use crate::languages::typescript::TypeScriptPlugin;
use crate::languages::LanguagePlugin;
use crate::type_checker::core::types::{Intrinsic, Type, TypeArena};

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

#[test]
fn absence_spellings_intern_as_atoms_rather_than_nominals() {
    let plugin = TypeScriptPlugin;
    let arena = TypeArena::new();

    for (spelling, atom) in [("null", Intrinsic::Null), ("undefined", Intrinsic::Undefined)] {
        let id = plugin.intern_type_text(&arena, spelling);
        assert_eq!(
            arena.get(id),
            Type::Intrinsic(atom),
            "`{spelling}` names no declaration, so it must not intern as one"
        );
    }
}

#[test]
fn an_absence_arm_of_a_union_interns_as_an_atom() {
    let plugin = TypeScriptPlugin;
    let arena = TypeArena::new();

    let union = plugin.intern_type_text(&arena, "Widget | null");
    let Type::Union(arms) = arena.get(union) else {
        panic!("`Widget | null` is a union");
    };
    assert!(matches!(arena.get(arms[0]), Type::Class(name) if name == "Widget"));
    assert_eq!(arena.get(arms[1]), Type::Intrinsic(Intrinsic::Null));
}

#[test]
fn a_name_that_merely_starts_with_an_absence_spelling_stays_nominal() {
    let plugin = TypeScriptPlugin;
    let arena = TypeArena::new();

    let nullable = plugin.intern_type_text(&arena, "Nullable");
    assert!(matches!(arena.get(nullable), Type::Class(name) if name == "Nullable"));

    let applied = plugin.intern_type_text(&arena, "undefinedish<Item>");
    assert!(matches!(arena.get(applied), Type::Apply { .. }));
}
