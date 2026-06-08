// =============================================================================
// type_checker/core/symbol_view_tests.rs — SymbolView facade unit tests.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::SymbolInfo;
use crate::type_checker::core::symbol_types::{SymbolTypeData, SymbolTypeMap};
use crate::type_checker::core::types::{PrimKind, TypeArena};
use std::sync::Arc;

fn sym(id: i64, name: &str, qname: &str, kind: &str) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from("x.ts"),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

#[test]
fn param_types_none_when_no_record_some_empty_when_recorded_zero() {
    let arena = TypeArena::new();
    let int_ty = arena.primitive(PrimKind::Int);

    let mut types = SymbolTypeMap::new();
    // id 1: recorded with a non-empty return but ZERO params (a no-arg
    // callable) → param_types() must be Some(&[]), not None.
    types.insert(
        1,
        SymbolTypeData {
            return_type: Some(int_ty),
            ..Default::default()
        },
    );
    // id 2: no record at all.

    let has_record = sym(1, "ping", "Svc.ping", "method");
    let no_record = sym(2, "pong", "Svc.pong", "method");

    let v1 = SymbolView::new(&has_record, &types);
    let v2 = SymbolView::new(&no_record, &types);

    assert_eq!(
        v1.param_types(),
        Some(&[][..]),
        "a record with zero params is Some(&[]), not None"
    );
    assert_eq!(
        v2.param_types(),
        None,
        "no SymbolTypeData record is None, distinct from zero params"
    );
}

#[test]
fn param_types_passthrough_when_populated() {
    let arena = TypeArena::new();
    let int_ty = arena.primitive(PrimKind::Int);
    let str_ty = arena.primitive(PrimKind::Str);

    let mut types = SymbolTypeMap::new();
    types.insert(
        5,
        SymbolTypeData {
            param_types: vec![int_ty, str_ty],
            ..Default::default()
        },
    );
    let s = sym(5, "f", "f", "function");
    let v = SymbolView::new(&s, &types);
    assert_eq!(v.param_types(), Some(&[int_ty, str_ty][..]));
}

#[test]
fn return_and_declared_type_passthrough() {
    let arena = TypeArena::new();
    let int_ty = arena.primitive(PrimKind::Int);
    let str_ty = arena.primitive(PrimKind::Str);

    let mut types = SymbolTypeMap::new();
    types.insert(
        3,
        SymbolTypeData {
            return_type: Some(int_ty),
            ..Default::default()
        },
    );
    types.insert(
        4,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let m = sym(3, "f", "f", "function");
    let field = sym(4, "name", "User.name", "field");
    assert_eq!(SymbolView::new(&m, &types).return_type(), Some(int_ty));
    assert_eq!(SymbolView::new(&field, &types).declared_type(), Some(str_ty));

    // Cross-checks: a method with no declared_type, a field with no return_type.
    assert_eq!(SymbolView::new(&m, &types).declared_type(), None);
    assert_eq!(SymbolView::new(&field, &types).return_type(), None);

    // No record at all → both None, generic_params empty.
    let absent = sym(99, "z", "z", "function");
    let va = SymbolView::new(&absent, &types);
    assert_eq!(va.return_type(), None);
    assert_eq!(va.declared_type(), None);
    assert!(va.generic_params().is_empty());
}

#[test]
fn type_data_some_when_recorded_none_when_absent() {
    let arena = TypeArena::new();
    let int_ty = arena.primitive(PrimKind::Int);

    let mut types = SymbolTypeMap::new();
    types.insert(
        7,
        SymbolTypeData {
            declared_type: Some(int_ty),
            ..Default::default()
        },
    );

    let has = sym(7, "name", "User.name", "field");
    let absent = sym(8, "ghost", "User.ghost", "field");
    assert!(
        SymbolView::new(&has, &types).type_data().is_some(),
        "a recorded id exposes its whole record"
    );
    assert_eq!(
        SymbolView::new(&absent, &types).type_data(),
        None,
        "an unrecorded id has no record — the distinction the per-field accessors collapse"
    );
}

