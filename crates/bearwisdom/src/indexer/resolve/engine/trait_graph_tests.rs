use super::*;

#[test]
fn unrelated_caller_has_no_bound_assumptions() {
    let arena = TypeArena::new();
    let mut graph = Graph::default();
    let bound = Obligation {
        subject: arena.decl("Input", 1),
        trait_type: arena.decl("Trait", 2),
    };
    graph.bounds.insert(10, vec![bound]);
    graph.parents.insert(11, 10);
    assert_eq!(graph.assumptions(11, &arena), Ok(vec![bound]));
    assert!(graph.assumptions(12, &arena).unwrap().is_empty());
}
