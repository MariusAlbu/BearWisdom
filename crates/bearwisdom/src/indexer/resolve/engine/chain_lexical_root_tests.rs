use super::*;
use crate::indexer::resolve::engine::testkit::{file_ctx, Lookup};

#[test]
fn callable_yield_keeps_a_canonical_return_id_without_nominalizing_it() {
    let arena = TypeArena::new();
    let expected = arena.intern(Type::Optional(arena.decl("SameDisplay", 71)));
    let ty = arena.intern(Type::Function {
        params: vec![],
        return_: expected,
    });
    let local = LocalReference {
        declaration: Some(3),
        kind: SymbolKind::Parameter,
        value_type: Some(ty),
        callable: None,
        type_args: vec![],
    };
    let segment = ChainSegment {
        name: "not-used-for-identity".into(),
        node_kind: String::new(),
        kind: SegmentKind::Identifier,
        declared_type: None,
        type_args: vec![],
        optional_chaining: false,
        byte_offset: 10,
        declared_type_id: None,
        is_call: true,
        call_args: vec![],
        type_arg_ids: vec![],
    };
    let result = resolve(
        local,
        &Lookup::new(),
        &arena,
        &file_ctx(vec![], None),
        &segment,
    )
    .unwrap();
    assert_eq!(result.ty, expected);
}
