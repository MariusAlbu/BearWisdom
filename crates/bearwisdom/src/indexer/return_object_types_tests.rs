use super::materialize;
use crate::types::{ExtractedSymbol, SymbolKind};

fn function(name: &str, line: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("m.{name}"),
        kind: SymbolKind::Function,
        visibility: None,
        start_line: line,
        end_line: line + 3,
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
    }
}

#[test]
fn literal_return_becomes_an_interface_with_property_members() {
    let mut symbols = vec![function("createLogger", 4)];
    materialize(&mut symbols, vec![(0, vec!["info".to_string(), "warn".to_string()])]);

    assert_eq!(symbols.len(), 4);
    let iface = &symbols[1];
    assert_eq!(iface.kind, SymbolKind::Interface);
    assert_eq!(iface.name, "createLogger$Ret");
    assert_eq!(iface.qualified_name, "m.createLogger$Ret");
    assert_eq!(iface.start_line, 4);
    for (i, member) in ["info", "warn"].iter().enumerate() {
        let sym = &symbols[2 + i];
        assert_eq!(sym.kind, SymbolKind::Property);
        assert_eq!(sym.name, *member);
        assert_eq!(sym.qualified_name, format!("m.createLogger$Ret.{member}"));
        assert_eq!(sym.parent_index, Some(1));
    }
}

#[test]
fn out_of_range_function_index_is_skipped() {
    let mut symbols = vec![function("f", 0)];
    materialize(&mut symbols, vec![(7, vec!["x".to_string()])]);
    assert_eq!(symbols.len(), 1);
}
