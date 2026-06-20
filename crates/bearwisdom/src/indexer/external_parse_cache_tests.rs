use super::*;
use crate::type_checker::core::types::TypeId;
use std::num::NonZeroU32;

fn sample_symbol() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "Open".to_string(),
        qualified_name: "db.Open".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 10,
        end_line: 12,
        start_col: 0,
        end_col: 4,
        byte_offset: 100,
        signature: Some("fn Open() -> Result<Conn>".to_string()),
        doc_comment: Some("doc".to_string()),
        scope_path: Some("db".to_string()),
        parent_index: Some(3),
        // These TypeIds must be dropped by the cache (arena-specific).
        declared_type: Some(TypeId(NonZeroU32::new(7).unwrap())),
        return_type: Some(TypeId(NonZeroU32::new(8).unwrap())),
        param_types: vec![TypeId(NonZeroU32::new(9).unwrap())],
        generic_params: Vec::new(),
    }
}

fn sample_ref() -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 2,
        target_name: "Conn".to_string(),
        kind: EdgeKind::TypeRef,
        line: 11,
        col: 9,
        byte_offset: 140,
        module: Some("db".to_string()),
        namespace_segments: vec!["inner".to_string()],
        chain: None,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: true,
    }
}

#[test]
fn cached_sym_roundtrip_preserves_fields_drops_typeids() {
    let s = sample_symbol();
    let back = CachedSym::from_extracted(&s).into_extracted();
    assert_eq!(back.name, s.name);
    assert_eq!(back.qualified_name, s.qualified_name);
    assert_eq!(back.kind, s.kind);
    assert_eq!(back.visibility, s.visibility);
    assert_eq!(back.signature, s.signature);
    assert_eq!(back.scope_path, s.scope_path);
    assert_eq!(back.parent_index, s.parent_index);
    assert_eq!(back.byte_offset, s.byte_offset);
    // TypeIds are arena-specific and not cached.
    assert!(back.declared_type.is_none());
    assert!(back.return_type.is_none());
    assert!(back.param_types.is_empty());
}

#[test]
fn cached_ref_roundtrip_preserves_fields_drops_chain() {
    let r = sample_ref();
    let back = CachedRef::from_extracted(&r).into_extracted();
    assert_eq!(back.source_symbol_index, r.source_symbol_index);
    assert_eq!(back.target_name, r.target_name);
    assert_eq!(back.kind, r.kind);
    assert_eq!(back.module, r.module);
    assert_eq!(back.namespace_segments, r.namespace_segments);
    assert_eq!(back.is_reexport, r.is_reexport);
    // Chain / call-args are not cached (externals are never resolution sources).
    assert!(back.chain.is_none());
    assert!(back.call_args.is_empty());
}

#[test]
fn cached_parse_serde_roundtrip() {
    let cp = CachedParse {
        language: "rust".to_string(),
        package_id: Some(4),
        symbols: vec![CachedSym::from_extracted(&sample_symbol())],
        refs: vec![CachedRef::from_extracted(&sample_ref())],
        alias_targets: vec![(
            "Foo".to_string(),
            crate::types::AliasTarget::Intersection(vec!["Bar".to_string()]),
        )],
    };
    let json = serde_json::to_string(&cp).expect("serialize");
    let back: CachedParse = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.language, "rust");
    assert_eq!(back.package_id, Some(4));
    assert_eq!(back.symbols.len(), 1);
    assert_eq!(back.symbols[0].qualified_name, "db.Open");
    assert_eq!(back.refs.len(), 1);
    assert_eq!(back.refs[0].target_name, "Conn");
    assert_eq!(
        back.alias_targets,
        vec![(
            "Foo".to_string(),
            crate::types::AliasTarget::Intersection(vec!["Bar".to_string()])
        )],
        "alias_targets must survive the cache round-trip"
    );
}

#[test]
fn content_hash_is_stable_and_distinct() {
    assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
    assert_ne!(content_hash(b"hello"), content_hash(b"world"));
    // sha-256 hex is 64 chars.
    assert_eq!(content_hash(b"x").len(), 64);
}
