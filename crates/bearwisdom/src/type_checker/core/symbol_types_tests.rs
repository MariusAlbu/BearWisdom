use super::*;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena};

#[test]
fn empty_map_returns_none_for_missing_id() {
    let map = SymbolTypeMap::new();
    assert!(map.get(42).is_none());
    assert_eq!(map.len(), 0);
}

#[test]
fn insert_then_get_round_trips() {
    let mut arena = TypeArena::new();
    let int_id = arena.intern(Type::Primitive(PrimKind::Int));
    let mut map = SymbolTypeMap::new();
    map.insert(
        7,
        SymbolTypeData {
            declared_type: Some(int_id),
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        },
    );
    let data = map.get(7).expect("expected entry");
    assert_eq!(data.declared_type, Some(int_id));
    assert!(data.return_type.is_none());
}

#[test]
fn insert_empty_bundle_is_treated_as_removal() {
    let mut map = SymbolTypeMap::new();
    map.insert(7, SymbolTypeData::default());
    assert!(map.get(7).is_none());
    assert_eq!(map.len(), 0);
}

#[test]
fn entry_creates_on_first_access_and_mutates_in_place() {
    let mut arena = TypeArena::new();
    let bool_id = arena.primitive(PrimKind::Bool);
    let mut map = SymbolTypeMap::new();
    map.entry(11).return_type = Some(bool_id);
    map.entry(11).param_types.push(bool_id);
    let data = map.get(11).expect("entry should exist");
    assert_eq!(data.return_type, Some(bool_id));
    assert_eq!(data.param_types, vec![bool_id]);
}

#[test]
fn iter_yields_all_inserted_pairs() {
    let mut arena = TypeArena::new();
    let int_id = arena.primitive(PrimKind::Int);
    let mut map = SymbolTypeMap::new();
    map.entry(1).declared_type = Some(int_id);
    map.entry(2).declared_type = Some(int_id);
    let collected: std::collections::HashSet<i64> = map.iter().map(|(k, _)| k).collect();
    let mut expected = std::collections::HashSet::new();
    expected.insert(1);
    expected.insert(2);
    assert_eq!(collected, expected);
}

#[test]
fn symbol_type_data_is_empty_detects_default_state() {
    assert!(SymbolTypeData::default().is_empty());
    let mut arena = TypeArena::new();
    let t = arena.primitive(PrimKind::Str);
    let populated = SymbolTypeData {
        declared_type: Some(t),
        ..Default::default()
    };
    assert!(!populated.is_empty());
}
