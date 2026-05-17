// =============================================================================
// languages/python/resolve_tests.rs — unit tests for resolve.rs
//
// Kept in a sibling file so the production module stays free of synthetic
// fixture literals (dep names, file paths) that look like hardcoded
// production values to a casual reader.
// =============================================================================

use super::resolve::PythonResolver;
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex, SymbolLookup,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use std::collections::HashMap;

fn make_sym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn make_import_ref(
    source_idx: usize,
    target: &str,
    module: &str,
    kind: EdgeKind,
) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line: 1,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        chain: None,
        byte_offset: 0,
    }
}

fn make_py_file(path: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "python".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }
}

fn build_index(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files
        .iter()
        .map(|f| ParsedFile {
            path: f.path.clone(),
            language: f.language.clone(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols: f.symbols.clone(),
            refs: f.refs.clone(),
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
            flow: crate::types::FlowMeta::default(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

/// `from posthog.models import Person` in a consumer file should resolve `Person`
/// to the class defined in `posthog/models/person.py`, even though `Person`'s
/// qualified_name is just "Person" (not "posthog.models.person.Person").
#[test]
fn test_init_reexport_submodule_resolution() {
    // posthog/models/person.py defines Person
    let person_file = make_py_file(
        "posthog/models/person.py",
        vec![make_sym("Person", "Person", SymbolKind::Class)],
        vec![],
    );

    // posthog/api/views.py imports Person from posthog.models
    let consumer_sym = make_sym("get_person", "get_person", SymbolKind::Function);
    let import_ref = make_import_ref(0, "Person", "posthog.models", EdgeKind::Imports);
    let call_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Person".to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        module: None,
        chain: None,
        byte_offset: 0,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
};
    let consumer_file = make_py_file(
        "posthog/api/views.py",
        vec![consumer_sym],
        vec![import_ref, call_ref],
    );

    let (index, id_map) = build_index(&[&person_file, &consumer_file]);
    let resolver = PythonResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    // The Calls ref to "Person" should resolve via python_from_import_prefix
    // (import says module=posthog.models, symbol lives under posthog/models/).
    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[1], // the Calls ref
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(None),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "Person should resolve via __init__.py re-export path"
    );
    let res = result.unwrap();
    let expected_id = *id_map
        .get(&("posthog/models/person.py".to_string(), "Person".to_string()))
        .unwrap();
    assert_eq!(res.target_symbol_id, expected_id);
    assert_eq!(res.strategy, "python_from_import_prefix");
    assert!(res.confidence >= 0.95);
}

/// `from myapp.models import Team` where Team lives in `myapp/models/team.py`
/// on Windows-style paths (backslash separators).
#[test]
fn test_init_reexport_windows_path() {
    let team_file = make_py_file(
        "myapp\\models\\team.py",
        vec![make_sym("Team", "Team", SymbolKind::Class)],
        vec![],
    );

    let consumer_sym = make_sym("handler", "handler", SymbolKind::Function);
    let import_ref = make_import_ref(0, "Team", "myapp.models", EdgeKind::Imports);
    let call_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Team".to_string(),
        kind: EdgeKind::TypeRef,
        line: 3,
        module: None,
        chain: None,
        byte_offset: 0,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
};
    let consumer_file = make_py_file(
        "myapp\\api\\views.py",
        vec![consumer_sym],
        vec![import_ref, call_ref],
    );

    let (index, _) = build_index(&[&team_file, &consumer_file]);
    let resolver = PythonResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[1],
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(None),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "Team should resolve on Windows backslash paths"
    );
}

/// A `class Foo` symbol must have `return_type = "Foo"` in the SymbolIndex so
/// that `x = Foo(); x.method()` can be chain-walked. Without this, the chain
/// walker receives `None` from `return_type_name("Foo")` and cannot propagate
/// the type of `x`.
#[test]
fn class_symbol_has_return_type_equal_to_own_qname() {
    let foo_file = make_py_file(
        "myapp/models.py",
        vec![make_sym("Foo", "Foo", SymbolKind::Class)],
        vec![],
    );

    let (index, _) = build_index(&[&foo_file]);

    assert_eq!(
        index.return_type_name("Foo"),
        Some("Foo"),
        "Class symbol must carry return_type = its own qualified_name"
    );
}

/// A nested class (`module.Foo`) must also expose `return_type` equal to its
/// fully-qualified name so chain walking works across module boundaries.
#[test]
fn nested_class_symbol_has_qualified_return_type() {
    let foo_file = make_py_file(
        "myapp/models.py",
        vec![make_sym("Foo", "myapp.models.Foo", SymbolKind::Class)],
        vec![],
    );

    let (index, _) = build_index(&[&foo_file]);

    assert_eq!(
        index.return_type_name("myapp.models.Foo"),
        Some("myapp.models.Foo"),
        "Qualified class symbol must carry return_type = its own qualified_name"
    );
}
