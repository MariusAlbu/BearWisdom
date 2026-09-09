use super::*;

#[test]
fn physical_navigation_rows_do_not_collapse_configured_type_identities() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let legacy = arena.decl("Shared", 3);
    let left = arena.decl_in(a, "Shared", 3);
    let right = arena.decl_in(b, "Shared", 3);
    assert_ne!(left, right);
    assert_ne!(left, legacy);
    assert_ne!(right, legacy);
    assert_eq!(arena.decl_in(a, "poisoned", 3), left);
    assert_eq!(
        arena.lookup(&Type::Decl {
            symbol_id: 3,
            qname: "different".into(),
            context: Some(a)
        }),
        Some(left)
    );
    assert_eq!(
        arena.intern(Type::Decl {
            symbol_id: 3,
            qname: "different".into(),
            context: Some(a)
        }),
        left
    );
    assert!(matches!(arena.get(right), Type::Decl { symbol_id: 3, .. }));
}

#[test]
fn concurrent_display_payloads_cannot_split_nominal_identity() {
    let arena = TypeArena::new();
    let context = NominalContextId::fresh();
    let barrier = std::sync::Barrier::new(12);
    let ids = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..12)
            .map(|i| {
                let arena = &arena;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    if i % 2 == 0 {
                        arena.decl_in(context, &format!("name {i}"), 4)
                    } else {
                        arena.intern(Type::Decl {
                            symbol_id: 4,
                            qname: format!("display {i}"),
                            context: Some(context),
                        })
                    }
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(ids.iter().all(|id| *id == ids[0]));
    assert_eq!(arena.len(), 1);
}

#[test]
fn nested_context_provenance_rejects_foreign_and_legacy_children() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let left = arena.decl_in(a, "Shared", 3);
    let right = arena.decl_in(b, "Shared", 3);
    let scalar = arena.primitive(super::super::PrimKind::Bool);
    let good = arena.intern(Type::Apply {
        base: left,
        args: vec![scalar],
    });
    assert!(arena.accepts_nominal_context(good, Some(a)));
    assert!(!arena.accepts_nominal_context(good, Some(b)));
    assert!(!arena.accepts_nominal_context(good, None));
    for child in [right, arena.decl("Shared", 3), arena.class("Shared")] {
        let mixed = arena.intern(Type::Apply {
            base: left,
            args: vec![child],
        });
        let nested = arena.intern(Type::Function {
            params: vec![scalar],
            return_: arena.intern(Type::Optional(mixed)),
        });
        for context in [None, Some(a), Some(b)] {
            assert!(!arena.accepts_nominal_context(nested, context));
        }
    }
    assert!(arena.accepts_nominal_context(scalar, None));
    assert!(arena.accepts_nominal_context(scalar, Some(a)));
}

#[test]
fn hydration_remints_contexts_without_changing_type_slots_or_navigation_rows() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let left = arena.decl_in(a, "Shared", 3);
    let left_child = arena.decl_in(a, "Child", 4);
    let right = arena.decl_in(b, "Shared", 3);
    let nested = arena.intern(Type::Apply {
        base: left,
        args: vec![left_child],
    });
    let restored = TypeArena::new();
    assert_eq!(
        restored.restore_snapshot(&arena.serialize_snapshot()),
        arena.len()
    );
    let Type::Decl {
        context: Some(new_a),
        symbol_id: 3,
        ..
    } = restored.get(left)
    else {
        panic!()
    };
    let Type::Decl {
        context: Some(new_b),
        symbol_id: 3,
        ..
    } = restored.get(right)
    else {
        panic!()
    };
    assert_ne!(new_a, a);
    assert_ne!(new_b, b);
    assert_ne!(new_a, new_b);
    assert!(restored.accepts_nominal_context(nested, Some(new_a)));
    assert!(!restored.accepts_nominal_context(nested, Some(a)));
    assert_eq!(restored.decl_in(new_a, "poisoned", 3), left);
    assert_eq!(restored.decl_in(new_a, "poisoned", 4), left_child);
    let newer = TypeArena::new();
    newer.restore_snapshot(&arena.serialize_snapshot());
    assert!(!newer.accepts_nominal_context(nested, Some(new_a)));
}

#[test]
fn pre_context_snapshot_remains_unconfigured() {
    let arena = TypeArena::new();
    assert_eq!(
        arena.restore_snapshot(r#"[[{"Decl":{"symbol_id":12,"qname":"Old"}}],[]]"#),
        1
    );
    let id = arena.decl("poisoned", 12);
    assert_eq!(id.index(), 0);
    assert!(arena.accepts_nominal_context(id, None));
    assert!(!arena.accepts_nominal_context(id, Some(NominalContextId::fresh())));
}
