use super::*;

#[test]
fn application_construction_does_not_compare_display_names() {
    let arena = TypeArena::new();
    let base = arena.decl("Same", 1);
    let arg = arena.decl("Same", 2);
    assert_eq!(application_target(&arena, base, &[]), base);
    assert_eq!(
        arena.get(application_target(&arena, base, &[arg])),
        Type::Apply {
            base,
            args: vec![arg]
        }
    );
}
