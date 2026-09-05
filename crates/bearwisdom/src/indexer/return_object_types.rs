// =============================================================================
// indexer/return_object_types — the object type a function's literal return is
//
// A function that returns an object literal (`return { info, warn }`) has a
// return type the source never names. The flow pass records the literal's
// property names per function; this pass materializes that type as an
// `{fn}$Ret` interface symbol with one property member per name, so a call to
// the function yields a type whose members resolve (`createLogger().info`).
// The members carry no type of their own — their presence is the resolve.
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

/// Append one `{fn}$Ret` interface plus its property members for every
/// `(fn_idx, property_names)` pair. A pair whose function index is out of
/// range is skipped.
pub(super) fn materialize(symbols: &mut Vec<ExtractedSymbol>, return_objects: Vec<(usize, Vec<String>)>) {
    for (fn_idx, members) in return_objects {
        let (ret_qname, ret_name, line) = match symbols.get(fn_idx) {
            Some(s) => (
                format!("{}$Ret", s.qualified_name),
                format!("{}$Ret", s.name),
                s.start_line,
            ),
            None => continue,
        };
        let iface_idx = symbols.len();
        symbols.push(synthetic(&ret_name, &ret_qname, SymbolKind::Interface, None, line));
        for m in &members {
            let m_qname = format!("{ret_qname}.{m}");
            symbols.push(synthetic(m, &m_qname, SymbolKind::Property, Some(iface_idx), line));
        }
    }
}

fn synthetic(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    parent: Option<usize>,
    line: u32,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: parent,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[cfg(test)]
#[path = "return_object_types_tests.rs"]
mod tests;
