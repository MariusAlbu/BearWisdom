use crate::types::{EdgeKind, SymbolKind};

#[test]
fn nested_items_keep_distinct_call_sources_and_do_not_duplicate_impl_methods() {
    let source = "fn outer() { struct Model; enum State { Ready } type Alias = Model; fn helper() { first(); } impl Model { fn run(&self) { second(); } } }";
    let result = super::super::extract::extract(source);
    let named = |name: &str| {
        result
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.name == name)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let helper = named("helper");
    let run = named("run");
    let model = named("Model");
    assert_eq!((helper.len(), run.len(), model.len()), (1, 1, 1));
    assert_eq!(result.symbols[run[0]].kind, SymbolKind::Method);
    assert_eq!(result.symbols[run[0]].parent_index, Some(model[0]));
    assert_eq!(named("Ready").len(), 1);
    assert_eq!(named("Alias").len(), 1);
    for (target, source) in [("first", helper[0]), ("second", run[0])] {
        let refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls && r.target_name == target)
            .collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].source_symbol_index, source);
    }
}
