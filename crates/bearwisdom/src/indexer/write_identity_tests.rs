use super::*;
use crate::indexer::lexical::LexicalBindings;

#[test]
fn private_declarations_use_exact_rows_and_skip_missing_or_ambiguous_slots() {
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    for (name, slot) in [("first", 0), ("skipped", 1), ("absent", 2)] {
        let name = graph.intern(name);
        let binding = graph.declare(scope, name, 0, None);
        graph.lexical_only.insert(binding);
        graph.attach_symbol(slot, binding);
    }
    assert_eq!(private_rows(Some(&graph), &[7, 0]).collect::<Vec<_>>(), [7]);
    assert!(private_rows(None, &[7]).next().is_none());
    let first = graph
        .binding_at(5, graph.name_id("first").unwrap())
        .unwrap();
    graph.attach_symbol(3, first);
    assert!(private_rows(Some(&graph), &[7, 0]).next().is_none());
}
