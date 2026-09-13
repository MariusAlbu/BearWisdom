// Tests for field_init_sources.rs — which ref initializes which field.

use super::field_initializers;
use crate::indexer::lexical::LexicalBindings;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

fn sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("Host.{name}"),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 1,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn reference(target: &str, kind: EdgeKind, source_symbol_index: usize, byte: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index,
        target_name: target.to_string(),
        kind,
        line: 1,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: byte,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
        is_include: false,
    }
}

fn parsed(symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: "src/Host.php".to_string(),
        language: "php".to_string(),
        content_hash: "h".to_string(),
        size: 256,
        line_count: 20,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

#[test]
fn a_member_initializer_overrides_the_leftmost_call_attribution() {
    let mut pf = parsed(
        vec![sym("factory", SymbolKind::Property), sym("setUp", SymbolKind::Method)],
        vec![
            reference("legacy", EdgeKind::Calls, 0, 5),
            reference("Factory", EdgeKind::Instantiates, 1, 40),
        ],
    );
    pf.flow.flow_member_init.insert(1, 0);

    let found = field_initializers(&pf);
    assert_eq!(found.len(), 1);
    assert_eq!(found[&0].target_name, "Factory");
}

#[test]
fn the_lexical_slot_path_is_unchanged_for_a_lexical_file() {
    let mut pf = parsed(
        vec![sym("a", SymbolKind::Property), sym("b", SymbolKind::Property)],
        vec![
            reference("make", EdgeKind::Calls, 0, 10),
            reference("other", EdgeKind::Calls, 1, 20),
        ],
    );
    let mut graph = LexicalBindings::default();
    graph.types.call_initializers.insert(0, (10, 10));
    pf.flow.lexical = Some(graph);

    let found = field_initializers(&pf);
    assert_eq!(found.len(), 1, "only the declaration slot attributes");
    assert_eq!(found[&0].target_name, "make");
}

#[test]
fn a_member_initializer_whose_ref_is_not_a_call_is_dropped() {
    let mut pf = parsed(
        vec![sym("factory", SymbolKind::Property)],
        vec![reference("Factory", EdgeKind::TypeRef, 0, 5)],
    );
    pf.flow.flow_member_init.insert(0, 0);

    assert!(field_initializers(&pf).is_empty());
}
