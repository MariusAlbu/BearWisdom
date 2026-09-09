use super::*;
#[test]
fn unavailable_configuration_is_not_an_unconfigured_lookup() {
    let arena = std::sync::Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), arena);
    let view = View::empty();
    let lookup = Lookup {
        tree: &tree,
        view: &view,
        source: None,
    };
    let mut graph = crate::indexer::lexical::LexicalBindings::default();
    assert!(lookup.nominal_context().is_some());
    assert_eq!(
        lookup.source_global_type(graph.intern("Absent")),
        Some(None)
    );
    assert!(lookup
        .global_value(graph.intern("Absent"))
        .declaration
        .is_none());
}
