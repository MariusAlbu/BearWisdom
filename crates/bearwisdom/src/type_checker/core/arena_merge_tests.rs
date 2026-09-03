use super::merge_snapshot_into;
use crate::type_checker::core::types::{GenericParamData, Type, TypeArena};

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

    assert_eq!(remap.type_id(user), Some(existing), "same structure dedups to one id");
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
        Type::Decl { symbol_id, qname } => {
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

    assert_eq!(first.type_id(ty), second.type_id(ty), "re-merge lands on the same ids");
    assert_eq!(count_after_first, dst.serialize_snapshot(), "re-merge adds no types");
}
