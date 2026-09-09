use super::super::{compilation::Compilation, contract::TypeInfo};
use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;
use crate::indexer::symbol_ids::SymbolIds;
use crate::type_checker::core::types::GenericParamData;
use std::sync::Arc;

#[test]
fn anonymous_sites_and_omitted_argument_positions_are_distinct_and_do_not_shift_types() {
    let arena = Arc::new(TypeArena::new());
    let mut lookup = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    let parameter = |kind| {
        arena.intern_generic(GenericParamData {
            name: "'_".into(),
            kind,
            owner_symbol_index: 0,
            bound: None,
        })
    };
    let a = parameter(GenericParamKind::Lifetime);
    let b = parameter(GenericParamKind::Lifetime);
    let t = parameter(GenericParamKind::Type);
    let x = parameter(GenericParamKind::Lifetime);
    let y = parameter(GenericParamKind::Lifetime);
    let z = parameter(GenericParamKind::Lifetime);
    lookup.type_info_by_id.insert(
        10,
        TypeInfo {
            generic_param_ids: vec![a, b, t],
            ..Default::default()
        },
    );
    lookup.type_info_by_id.insert(
        20,
        TypeInfo {
            elided_input_params: vec![(17, 0, x), (17, 1, y)],
            ..Default::default()
        },
    );
    lookup.type_info_by_id.insert(
        21,
        TypeInfo {
            elided_input_params: vec![(17, 0, z)],
            ..Default::default()
        },
    );
    assert_ne!(
        region(&lookup, &arena, 20, 17, 0),
        region(&lookup, &arena, 20, 17, 1)
    );
    assert_ne!(
        region(&lookup, &arena, 20, 17, 0),
        region(&lookup, &arena, 21, 17, 0)
    );
    let base = arena.decl("Same", 10);
    let actual = arena.decl("Same", 11);
    let result = application(&lookup, &arena, 20, 17, base, vec![actual]);
    assert_eq!(
        arena.get(result),
        Type::Apply {
            base,
            args: vec![arena.generic_type(x), arena.generic_type(y), actual]
        }
    );
    assert!(
        lookup.type_info_by_id[&20].generic_param_ids.is_empty(),
        "implicit binders are not explicit argument slots"
    );
    let explicit = vec![arena.generic_type(z), arena.generic_type(z), actual];
    assert_eq!(
        arena.get(application(&lookup, &arena, 20, 17, base, explicit.clone())),
        Type::Apply {
            base,
            args: explicit
        }
    );
    assert_eq!(
        arena.get(application(
            &lookup,
            &arena,
            20,
            17,
            base,
            vec![actual, actual]
        )),
        Type::Unknown
    );
    let restored: TypeInfo =
        serde_json::from_str(&serde_json::to_string(&lookup.type_info_by_id[&20]).unwrap())
            .unwrap();
    assert_eq!(restored.elided_input_params, [(17, 0, x), (17, 1, y)]);
    let mut old = serde_json::to_value(&restored).unwrap();
    old.as_object_mut().unwrap().remove("elided_input_params");
    assert!(serde_json::from_value::<TypeInfo>(old)
        .unwrap()
        .elided_input_params
        .is_empty());
}

#[test]
fn missing_anonymous_owner_is_unknown_not_a_shared_synthetic_identity() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    assert_eq!(
        arena.get(region(&lookup, &arena, 7, 31, 0)),
        Type::Region(Lifetime::Unknown)
    );
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(
        application(&lookup, &arena, 7, 31, unknown, vec![]),
        unknown
    );
}
