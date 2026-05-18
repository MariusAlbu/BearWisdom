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
fn build_propagates_extractor_populated_fields() {
    // Simulate a Phase 5+ extractor that emits TypeId-bearing fields on
    // ExtractedSymbol directly. The builder must carry those values into
    // SymbolTypeData rather than silently dropping them.
    use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

    let mut arena = TypeArena::new();
    let int_id = arena.primitive(PrimKind::Int);
    let str_id = arena.primitive(PrimKind::Str);
    let t_param = arena.intern_generic(crate::type_checker::core::types::GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });

    let method_sym = ExtractedSymbol {
        name: "find".to_string(),
        qualified_name: "Repo.find".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: Some("Repo".to_string()),
        parent_index: None,
        declared_type: None,
        return_type: Some(int_id),
        param_types: vec![str_id],
        generic_params: vec![t_param],
    };

    let pf = ParsedFile {
        path: "x.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![method_sym],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("x.ts".to_string(), 0), 42);

    let map = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );

    let data = map.get(42).expect("method's TypeData propagated");
    assert_eq!(
        data.return_type,
        Some(int_id),
        "extractor's return_type carries through"
    );
    assert_eq!(
        data.param_types,
        vec![str_id],
        "param_types carries through"
    );
    assert_eq!(
        data.generic_params,
        vec![t_param],
        "generic_params carries through"
    );
    assert_eq!(data.declared_type, None);
}

#[test]
fn build_extractor_return_type_overrides_self_yield_for_type_defining_kinds() {
    // A type-defining symbol whose extractor explicitly set return_type
    // must not have its choice clobbered by the self-yield default.
    use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

    let mut arena = TypeArena::new();
    let meta_id = arena.class("MetaUser");

    let sym = ExtractedSymbol {
        name: "User".to_string(),
        qualified_name: "User".to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: Some(meta_id),
        param_types: Vec::new(),
        generic_params: Vec::new(),
    };

    let pf = ParsedFile {
        path: "x.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![sym],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("x.ts".to_string(), 0), 7);

    let map = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );

    let data = map.get(7).expect("class entry");
    assert_eq!(
        data.return_type,
        Some(meta_id),
        "extractor-provided return_type must win over self-yield default"
    );
}

#[test]
fn build_records_self_yielding_reverse_index_for_type_defining_kinds() {
    use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

    let mut arena = TypeArena::new();
    let class_sym = ExtractedSymbol {
        name: "User".to_string(),
        qualified_name: "User".to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    };
    let pf = ParsedFile {
        path: "x.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![class_sym],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };
    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("x.ts".to_string(), 0), 99);

    let map = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );

    let user_ty = arena.class("User");
    let data = map
        .data_for_class(user_ty)
        .expect("reverse index resolves class TypeId → SymbolTypeData");
    assert_eq!(data.return_type, Some(user_ty));
    assert_eq!(map.sym_id_for_class(user_ty), Some(99));
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
