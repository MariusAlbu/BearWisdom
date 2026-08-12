use super::*;
use crate::types::Visibility;

fn symbol(name: &str, qualified_name: &str, kind: SymbolKind, parent_index: Option<usize>) -> ExtractedSymbol {
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
fn direct_children_of_unit_root_get_unit_prefixed() {
    let mut symbols = vec![
        symbol("MyUnit", "MyUnit", SymbolKind::Namespace, None),
        symbol("DoThing", "DoThing", SymbolKind::Function, Some(0)),
        symbol("TFoo", "TFoo", SymbolKind::Class, Some(0)),
    ];
    qualify_top_level_qnames(&mut symbols);

    assert_eq!(symbols[1].qualified_name, "MyUnit.DoThing");
    assert_eq!(symbols[2].qualified_name, "MyUnit.TFoo");
}

#[test]
fn nested_class_member_is_untouched() {
    let mut symbols = vec![
        symbol("MyUnit", "MyUnit", SymbolKind::Namespace, None),
        symbol("TFoo", "TFoo", SymbolKind::Class, Some(0)),
        // Field inside TFoo — parent_index points at TFoo (index 1), not the unit.
        symbol("Bar", "Bar", SymbolKind::Method, Some(1)),
    ];
    qualify_top_level_qnames(&mut symbols);

    assert_eq!(symbols[2].qualified_name, "Bar");
}

#[test]
fn already_dotted_qname_is_not_double_prefixed() {
    let mut symbols = vec![
        symbol("MyUnit", "MyUnit", SymbolKind::Namespace, None),
        symbol("TFoo.Bar", "TFoo.Bar", SymbolKind::Function, Some(0)),
    ];
    qualify_top_level_qnames(&mut symbols);

    assert_eq!(symbols[1].qualified_name, "TFoo.Bar");
}

#[test]
fn uses_block_namespace_symbol_is_skipped() {
    let mut symbols = vec![
        symbol("MyUnit", "MyUnit", SymbolKind::Namespace, None),
        symbol("uses", "uses", SymbolKind::Namespace, Some(0)),
    ];
    qualify_top_level_qnames(&mut symbols);

    assert_eq!(symbols[1].qualified_name, "uses");
}

#[test]
fn fragment_with_no_namespace_root_is_a_no_op() {
    // `.inc` fragments have no `unit`/`program` header — index 0 is whatever
    // declaration parses first, never a `Namespace` symbol.
    let mut symbols = vec![symbol(
        "FragmentHelper",
        "FragmentHelper",
        SymbolKind::Function,
        None,
    )];
    qualify_top_level_qnames(&mut symbols);

    assert_eq!(symbols[0].qualified_name, "FragmentHelper");
}

#[test]
fn empty_symbol_list_does_not_panic() {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    qualify_top_level_qnames(&mut symbols);
    assert!(symbols.is_empty());
}
