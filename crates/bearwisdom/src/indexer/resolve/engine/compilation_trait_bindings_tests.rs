use super::*;

#[test]
fn empty_compilation_has_no_phantom_trait_environment() {
    let mut tree = Compilation::build(&[], &SymbolIds::default(), Arc::new(TypeArena::new()));
    tree.prepare_trait_sources();
    assert!(tree.trait_graph().definitions.is_empty());
    assert!(tree.trait_file("missing.rs").is_none());
}
