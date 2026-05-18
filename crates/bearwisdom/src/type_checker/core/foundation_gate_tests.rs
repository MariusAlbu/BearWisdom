//! Gate-test for phase 1: the foundation modules must compile, unit-test,
//! and build a SymbolTypeMap from a real ParsedFile. The test exercises the
//! TS extractor end-to-end and feeds its symbols through
//! `SymbolTypeMap::build_from_parsed_files`.

use crate::languages::typescript::extract;
use crate::type_checker::core::{SymbolIdMap, SymbolTypeMap, Type, TypeArena};
use crate::type_checker::profile::language_profile::{DEFAULT_PROFILE, LanguageProfile};
use crate::types::{ParsedFile, SymbolKind};

fn wrap_as_parsed_file(path: &str, source: &str) -> ParsedFile {
    let extraction = extract::extract(source, false);
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: source.len() as u64,
        line_count: source.lines().count() as u32,
        mtime: None,
        package_id: None,
        symbols: extraction.symbols,
        refs: extraction.refs,
        routes: extraction.routes,
        db_sets: extraction.db_sets,
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(source.to_string()),
        has_errors: extraction.has_errors,
        flow: Default::default(),
        demand_contributions: extraction.demand_contributions,
        alias_targets: extraction.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

#[test]
fn build_symbol_type_map_from_typescript_parsed_file() {
    let source = r#"
export class User {
    name: string;
    age: number;
}

export interface Repository {
    save(user: User): Promise<void>;
}

export type Id = string | number;
"#;
    let pf = wrap_as_parsed_file("src/user.ts", source);

    let mut sym_id_map: SymbolIdMap = Default::default();
    for (idx, _) in pf.symbols.iter().enumerate() {
        sym_id_map.insert((pf.path.clone(), idx), idx as i64 + 1);
    }

    let mut arena = TypeArena::new();
    let profile: &LanguageProfile = &DEFAULT_PROFILE;
    let map = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_id_map,
        &mut arena,
        profile,
    );

    let type_defining_count = pf
        .symbols
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Class
                    | SymbolKind::Struct
                    | SymbolKind::Interface
                    | SymbolKind::Enum
                    | SymbolKind::TypeAlias
            )
        })
        .count();
    assert!(
        type_defining_count >= 3,
        "extractor should produce at least User/Repository/Id"
    );
    assert_eq!(
        map.len(),
        type_defining_count,
        "every type-defining symbol gets a SymbolTypeData entry"
    );

    for (idx, sym) in pf.symbols.iter().enumerate() {
        if !matches!(
            sym.kind,
            SymbolKind::Class
                | SymbolKind::Struct
                | SymbolKind::Interface
                | SymbolKind::Enum
                | SymbolKind::TypeAlias
        ) {
            continue;
        }
        let sym_id = sym_id_map[&(pf.path.clone(), idx)];
        let data = map.get(sym_id).expect("type-defining sym should have data");
        let return_id = data
            .return_type
            .expect("type-defining sym yields itself as return_type");
        match arena.get(return_id) {
            Type::Class(q) => assert_eq!(q, sym.qualified_name),
            other => panic!("expected Class type, got {other:?}"),
        }
    }
}

#[test]
fn build_handles_empty_parsed_file_set() {
    let mut arena = TypeArena::new();
    let map = SymbolTypeMap::build_from_parsed_files(
        &[],
        &Default::default(),
        &mut arena,
        &DEFAULT_PROFILE,
    );
    assert!(map.is_empty());
    assert!(arena.is_empty());
}

#[test]
fn build_skips_symbols_missing_from_id_map() {
    let pf = wrap_as_parsed_file("src/foo.ts", "export class Foo {}");
    let mut arena = TypeArena::new();
    let empty_map: SymbolIdMap = Default::default();
    let map = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &empty_map,
        &mut arena,
        &DEFAULT_PROFILE,
    );
    assert!(map.is_empty(), "no symbols recorded when sym_id_map is empty");
}
