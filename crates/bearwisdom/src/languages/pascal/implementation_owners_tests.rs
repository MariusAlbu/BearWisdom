use super::*;
use crate::types::Visibility;

fn symbol(
    name: &str,
    qualified_name: &str,
    kind: SymbolKind,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[test]
fn qualified_implementation_is_owned_by_its_declaring_type() {
    let mut symbols = vec![
        symbol("Widgets", "Widgets", SymbolKind::Namespace, None),
        symbol("TWidget", "Widgets.TWidget", SymbolKind::Class, Some(0)),
        symbol(
            "TWidget.Paint",
            "TWidget.Paint",
            SymbolKind::Function,
            Some(0),
        ),
    ];

    attach(&mut symbols, &mut []);

    assert_eq!(symbols[2].parent_index, Some(1));
    assert_eq!(symbols[2].scope_path.as_deref(), Some("Widgets.TWidget"));
    assert_eq!(symbols[2].qualified_name, "TWidget.Paint");
}

#[test]
fn qualified_free_routine_without_a_type_owner_stays_at_unit_scope() {
    let mut symbols = vec![
        symbol("Widgets", "Widgets", SymbolKind::Namespace, None),
        symbol("Other.Paint", "Other.Paint", SymbolKind::Function, Some(0)),
    ];

    attach(&mut symbols, &mut []);

    assert_eq!(symbols[1].parent_index, Some(0));
    assert_eq!(symbols[1].scope_path, None);
}

#[test]
fn a_type_from_another_structural_scope_cannot_claim_the_body() {
    let mut symbols = vec![
        symbol("First", "First", SymbolKind::Namespace, None),
        symbol("Second", "Second", SymbolKind::Namespace, None),
        symbol("TWidget", "First.TWidget", SymbolKind::Class, Some(0)),
        symbol(
            "TWidget.Paint",
            "TWidget.Paint",
            SymbolKind::Function,
            Some(1),
        ),
    ];

    attach(&mut symbols, &mut []);

    assert_eq!(symbols[3].parent_index, Some(1));
    assert_eq!(symbols[3].scope_path, None);
}

#[test]
fn standalone_fragment_promotes_the_qualified_body_until_include_splicing() {
    let mut symbols = vec![
        symbol("unknown", "unknown", SymbolKind::Function, None),
        symbol(
            "TWidget.Paint",
            "TWidget.Paint",
            SymbolKind::Function,
            Some(0),
        ),
    ];
    let mut refs = vec![crate::types::ExtractedRef {
        source_symbol_index: 0,
        target_name: "AddField".into(),
        kind: crate::types::EdgeKind::Calls,
        line: 2,
        col: 2,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
        is_include: false,
    }];

    attach(&mut symbols, &mut refs);

    assert_eq!(symbols[1].parent_index, None);
    assert_eq!(symbols[1].scope_path, None);
    assert_eq!(refs[0].source_symbol_index, 1);

    prepare_include_splice(&mut symbols);
    assert_eq!(symbols[1].scope_path.as_deref(), Some("TWidget"));
}
