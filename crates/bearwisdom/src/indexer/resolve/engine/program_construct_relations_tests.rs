use super::*;

#[test]
fn constructor_proof_does_not_accept_foreign_or_incomplete_signature_origins() {
    let arena = std::sync::Arc::new(TypeArena::new());
    let mut view = View::empty();
    view.callable_policy = Some(crate::indexer::programs::CallablePolicy {
        strict_parameters: true,
        strict_nulls: true,
        bivariant_methods: Some(true),
    });
    let tree = Compilation::build(&[], &Default::default(), std::sync::Arc::clone(&arena));
    let lookup = Lookup {
        tree: &tree,
        view: &view,
        source: None,
    };
    let relation = &Relation {
        lookup: &lookup,
        arena: &arena,
    };
    let origin = CallableOrigin::new(
        view.context,
        0,
        crate::types::SourceSpan { start: 1, end: 2 },
    );
    let mut callable = Callable {
        origin,
        generics: vec![],
        parameters: vec![],
        result: arena.intern(Type::Unknown),
        predicate: None,
        complete: true,
    };
    assert!(Eval::new(relation).constructor(&callable, 0).is_none());
    callable.result = arena.intern(Type::Intrinsic(Intrinsic::Number));
    callable.complete = false;
    assert!(Eval::new(relation).constructor(&callable, 0).is_none());
    callable.complete = true;
    assert!(Eval::new(relation).constructor(&callable, 0).is_some());
    callable.origin = CallableOrigin::new(
        crate::type_checker::core::types::NominalContextId::fresh(),
        0,
        origin.signature,
    );
    assert!(Eval::new(relation).constructor(&callable, 0).is_none());
}
