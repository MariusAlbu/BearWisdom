use super::*;

#[test]
fn display_uses_the_arena_without_querying_legacy_text_when_both_are_present() {
    let arena = TypeArena::new();
    let ty = arena.decl("Display", 71);
    assert_eq!(
        render(Some(ty), Some(&arena), || panic!("canonical display wins")),
        Some("Display".into())
    );
    assert_eq!(
        render(None, Some(&arena), || Some("legacy")),
        Some("legacy".into())
    );
    assert_eq!(
        render(Some(ty), None, || Some("legacy")),
        Some("legacy".into())
    );
    assert_eq!(render(None, None, || None), None);
}
