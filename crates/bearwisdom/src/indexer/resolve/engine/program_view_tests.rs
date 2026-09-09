use super::*;
#[test]
fn absent_configuration_does_not_select_an_empty_program() {
    let arena = std::sync::Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), arena);
    assert!(Store::default().for_file(&tree, "source.ts").is_none());
}
