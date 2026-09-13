// Tests for type_ref_emit.rs — the PHP type-name TypeRef emitter.

use super::type_ref_emit::emit_php_type_ref;
use crate::types::{EdgeKind, ExtractedRef};

fn emit(name: &str) -> Vec<ExtractedRef> {
    let mut refs = Vec::new();
    emit_php_type_ref(name, 7, 42, &mut refs, 3);
    refs
}

#[test]
fn a_qualified_type_name_is_emitted_as_written() {
    let refs = emit("\\Fx\\Factory");
    assert_eq!(refs.len(), 1, "expected one TypeRef, got: {refs:?}");
    assert_eq!(refs[0].target_name, "\\Fx\\Factory");
    assert_eq!(refs[0].kind, EdgeKind::TypeRef);
    assert_eq!(refs[0].line, 7);
    assert_eq!(refs[0].byte_offset, 42);
    assert_eq!(refs[0].source_symbol_index, 3);
}

#[test]
fn a_grammar_owned_type_emits_no_reference() {
    for name in ["string", "mixed", "self", "array"] {
        assert!(emit(name).is_empty(), "{name} should emit no TypeRef");
    }
}

#[test]
fn an_empty_name_emits_no_reference() {
    assert!(emit("").is_empty());
    assert!(emit("\\").is_empty());
}
