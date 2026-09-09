use super::*;
use crate::indexer::resolve::engine::testkit::{
    call_ref, file_ctx, ref_ctx, source_symbol, Lookup,
};

#[test]
fn active_binding_type_precedes_the_extractors_flat_annotation() {
    let arena = TypeArena::new();
    let lookup = Lookup::new().with_local_type("value", "Alpha");
    let reference = call_ref("save");
    let source = source_symbol("caller");
    let context = ref_ctx(&reference, &source, vec![]);
    let segment = crate::types::ChainSegment {
        name: "value".into(),
        kind: SegmentKind::Identifier,
        node_kind: String::new(),
        declared_type: Some("Beta".into()),
        declared_type_id: None,
        type_args: vec![],
        type_arg_ids: vec![],
        optional_chaining: false,
        byte_offset: 0,
        is_call: false,
        call_args: vec![],
    };
    let receiver =
        resolve_root_impl(&context, &file_ctx(vec![], None), &lookup, &arena, &segment).unwrap();
    assert_eq!(receiver.ty, arena.class("Alpha"));
}
