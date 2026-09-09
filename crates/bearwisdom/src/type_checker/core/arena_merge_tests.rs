use super::merge_snapshot_into;

#[test]
fn merging_nominals_preserves_program_separation_and_remaps_navigation_rows() {
    use crate::type_checker::core::types::NominalContextId;
    let source = TypeArena::new();
    let destination = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let left = source.decl_in(a, "Shared", 41);
    let right = source.decl_in(b, "Shared", 41);
    let child = source.decl_in(a, "Child", 42);
    let applied = source.intern(Type::Apply {
        base: left,
        args: vec![child],
    });
    let remap =
        merge_snapshot_into(&source.serialize_snapshot(), &destination, &|id| id + 100).unwrap();
    let target = remap.type_id(left).unwrap();
    let other = remap.type_id(right).unwrap();
    let Type::Decl {
        symbol_id: 141,
        context: Some(context),
        ..
    } = destination.get(target)
    else {
        panic!()
    };
    assert_ne!(context, a);
    assert_ne!(target, other);
    assert_eq!(destination.decl_in(context, "poisoned", 141), target);
    assert!(destination.accepts_nominal_context(remap.type_id(applied).unwrap(), Some(context)));
    assert!(!destination.accepts_nominal_context(other, Some(context)));
    assert!(!destination.accepts_nominal_context(target, Some(a)));
    let second =
        merge_snapshot_into(&source.serialize_snapshot(), &destination, &|id| id + 100).unwrap();
    assert_ne!(
        second.type_id(left),
        Some(target),
        "an independent imported snapshot cannot revive a context handle"
    );
}

#[test]
fn call_site_inference_regions_preserve_identity_and_remap_their_caller() {
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
    let src = TypeArena::new();
    let region = Lifetime::Inference {
        owner: 41,
        byte: 70,
    };
    let slot = src.intern(Type::Region(region));
    let sibling = src.intern(Type::Region(Lifetime::Inference {
        owner: 42,
        byte: 70,
    }));
    let other_call = src.intern(Type::Region(Lifetime::Inference {
        owner: 41,
        byte: 71,
    }));
    assert_ne!(slot, sibling);
    assert_ne!(slot, other_call);
    let reference = src.intern(Type::Indirect {
        kind: Indirection::Reference(region),
        mutability: Mutability::Shared,
        inner: src.decl("C", 51),
    });
    let snapshot = src.serialize_snapshot();
    let restored = TypeArena::new();
    restored.restore_snapshot(&snapshot);
    assert_eq!(restored.get(slot), src.get(slot));
    assert_eq!(restored.get(reference), src.get(reference));
    let dst = TypeArena::new();
    dst.class("occupied");
    let map = merge_snapshot_into(&snapshot, &dst, &|id| id + 1000).unwrap();
    assert_eq!(
        dst.get(map.type_id(slot).unwrap()),
        Type::Region(Lifetime::Inference {
            owner: 1041,
            byte: 70
        })
    );
    let Type::Indirect { kind, inner, .. } = dst.get(map.type_id(reference).unwrap()) else {
        panic!("reference preserved");
    };
    assert_eq!(
        kind,
        Indirection::Reference(Lifetime::Inference {
            owner: 1041,
            byte: 70
        })
    );
    assert!(matches!(
        dst.get(inner),
        Type::Decl {
            symbol_id: 1051,
            ..
        }
    ));
    let second = merge_snapshot_into(&snapshot, &dst, &|id| id + 1000).unwrap();
    assert_eq!(map.type_id(reference), second.type_id(reference));
}
use crate::type_checker::core::types::{GenericParamData, Type, TypeArena};

#[test]
fn region_parameters_are_remapped_even_when_first_seen_inside_a_reference() {
    use crate::type_checker::core::types::{GenericParamKind, Indirection, Lifetime, Mutability};
    let src = TypeArena::new();
    let dst = TypeArena::new();
    let p = src.intern_generic(GenericParamData {
        name: "'a".into(),
        kind: GenericParamKind::Lifetime,
        owner_symbol_index: 7,
        bound: None,
    });
    let nominal = src.decl("Doc", 41);
    let reference = src.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Parameter(p)),
        mutability: Mutability::Shared,
        inner: nominal,
    });
    let arg = src.generic_type(p);
    dst.intern_type_parameter("'a".into(), 7, None);
    dst.intern_type_parameter("noise".into(), 0, None);
    let remap = merge_snapshot_into(&src.serialize_snapshot(), &dst, &|id| id + 100).unwrap();
    let Type::Indirect {
        kind: Indirection::Reference(Lifetime::Parameter(mapped)),
        inner,
        ..
    } = dst.get(remap.type_id(reference).unwrap())
    else {
        panic!("reference");
    };
    assert_ne!(mapped, p);
    assert_eq!(
        dst.get(remap.type_id(arg).unwrap()),
        Type::Region(Lifetime::Parameter(mapped))
    );
    assert_eq!(dst.generic_param(mapped).kind, GenericParamKind::Lifetime);
    assert!(matches!(dst.get(inner), Type::Decl { symbol_id: 141, .. }));
}

#[test]
fn indirection_merge_remaps_children_and_declaration_ids() {
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
    let src = TypeArena::new();
    let dst = TypeArena::new();
    dst.class("padding");
    dst.class("more padding");
    let child = src.decl("Same", 42);
    let kind = Indirection::Reference(Lifetime::Static);
    let ty = src.intern(Type::Indirect {
        kind,
        mutability: Mutability::Shared,
        inner: child,
    });
    let remap = merge_snapshot_into(&src.serialize_snapshot(), &dst, &|id| id + 1000).unwrap();
    let inner = remap.type_id(child).unwrap();
    assert_eq!(
        dst.get(remap.type_id(ty).unwrap()),
        Type::Indirect {
            kind,
            mutability: Mutability::Shared,
            inner
        }
    );
    assert!(matches!(
        dst.get(inner),
        Type::Decl {
            symbol_id: 1042,
            ..
        }
    ));
}

#[test]
fn structural_types_remap_and_dedup_into_destination() {
    let src = TypeArena::new();
    let user = src.class("pkg.User");
    let opt = src.intern(Type::Optional(src.intern(Type::Apply {
        base: user,
        args: vec![src.class("T")],
    })));
    let snapshot = src.serialize_snapshot();

    let dst = TypeArena::new();
    // Pre-intern the same class so the merge must dedup onto the existing id.
    let existing = dst.class("pkg.User");
    let remap = merge_snapshot_into(&snapshot, &dst, &|id| id).unwrap();

    assert_eq!(
        remap.type_id(user),
        Some(existing),
        "same structure dedups to one id"
    );
    let merged_opt = remap.type_id(opt).unwrap();
    match dst.get(merged_opt) {
        Type::Optional(inner) => match dst.get(inner) {
            Type::Apply { base, .. } => assert_eq!(base, existing),
            other => panic!("expected Apply under Optional, got {other:?}"),
        },
        other => panic!("expected Optional, got {other:?}"),
    }
}

#[test]
fn decl_symbol_ids_pass_through_the_remapper() {
    let src = TypeArena::new();
    let decl = src.decl("pkg.User", 41);
    let snapshot = src.serialize_snapshot();

    let dst = TypeArena::new();
    let remap = merge_snapshot_into(&snapshot, &dst, &|id| id + 1000).unwrap();

    match dst.get(remap.type_id(decl).unwrap()) {
        Type::Decl {
            symbol_id, qname, ..
        } => {
            assert_eq!(symbol_id, 1041);
            assert_eq!(qname, "pkg.User");
        }
        other => panic!("expected Decl, got {other:?}"),
    }
}

#[test]
fn generic_params_reintern_with_remapped_bounds() {
    let src = TypeArena::new();
    let animal = src.class("Animal");
    let param = src.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 0,
        bound: Some(animal),
    });
    let generic = src.intern(Type::Generic { param });
    let snapshot = src.serialize_snapshot();

    let dst = TypeArena::new();
    let remap = merge_snapshot_into(&snapshot, &dst, &|id| id).unwrap();

    let merged = remap.type_id(generic).unwrap();
    match dst.get(merged) {
        Type::Generic { param } => {
            let data = dst.generic_param(param);
            assert_eq!(data.name, "T");
            let bound = data.bound.expect("bound survives the merge");
            assert_eq!(dst.get(bound), Type::Class("Animal".into()));
        }
        other => panic!("expected Generic, got {other:?}"),
    }
}

#[test]
fn merge_is_idempotent_across_repeated_snapshots() {
    let src = TypeArena::new();
    let ty = src.intern(Type::Union(vec![src.class("A"), src.class("B")]));
    let snapshot = src.serialize_snapshot();

    let dst = TypeArena::new();
    let first = merge_snapshot_into(&snapshot, &dst, &|id| id).unwrap();
    let count_after_first = dst.serialize_snapshot();
    let second = merge_snapshot_into(&snapshot, &dst, &|id| id).unwrap();

    assert_eq!(
        first.type_id(ty),
        second.type_id(ty),
        "re-merge lands on the same ids"
    );
    assert_eq!(
        count_after_first,
        dst.serialize_snapshot(),
        "re-merge adds no types"
    );
}
