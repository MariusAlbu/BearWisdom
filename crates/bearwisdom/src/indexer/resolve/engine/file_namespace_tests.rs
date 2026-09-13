use super::declared_namespace;
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

fn blank_parsed_file() -> ParsedFile {
    ParsedFile {
        path: "src/main/java/app/Zqrepo.java".to_string(),
        language: "java".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn symbol(name: &str, qname: &str, kind: SymbolKind, scope_path: Option<&str>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope_path.map(str::to_string),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[test]
fn one_namespace_with_a_symbol_scoped_under_it_is_the_file_namespace() {
    let mut file = blank_parsed_file();
    file.symbols
        .push(symbol("app", "app", SymbolKind::Namespace, None));
    file.symbols.push(symbol(
        "Zqrepo",
        "app.Zqrepo",
        SymbolKind::Class,
        Some("app"),
    ));

    assert_eq!(declared_namespace(&file), Some("app"));
}

#[test]
fn two_top_level_namespaces_declare_nothing() {
    let mut file = blank_parsed_file();
    file.symbols
        .push(symbol("app", "app", SymbolKind::Namespace, None));
    file.symbols
        .push(symbol("other", "other", SymbolKind::Namespace, None));
    file.symbols.push(symbol(
        "Zqrepo",
        "app.Zqrepo",
        SymbolKind::Class,
        Some("app"),
    ));

    assert!(declared_namespace(&file).is_none());
}

#[test]
fn a_namespace_nothing_is_scoped_under_is_only_mentioned() {
    let mut file = blank_parsed_file();
    file.symbols
        .push(symbol("app", "app", SymbolKind::Namespace, None));
    file.symbols.push(symbol(
        "Zqrepo",
        "other.Zqrepo",
        SymbolKind::Class,
        Some("other"),
    ));

    assert!(declared_namespace(&file).is_none());
}

#[test]
fn a_file_with_no_namespace_symbol_declares_nothing() {
    let mut file = blank_parsed_file();
    file.symbols
        .push(symbol("Zqrepo", "Zqrepo", SymbolKind::Class, None));

    assert!(declared_namespace(&file).is_none());
}
