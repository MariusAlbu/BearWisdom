//! Function-local items use the same declaration extractors as module items.
use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::Node;

pub(super) fn extract(
    node: Node,
    source: &str,
    parent: usize,
    symbols: Option<&mut Vec<ExtractedSymbol>>,
    refs: &mut Vec<ExtractedRef>,
) -> bool {
    if !matches!(
        node.kind(),
        "struct_item" | "union_item" | "enum_item" | "type_item" | "function_item" | "impl_item"
    ) {
        return false;
    }
    let Some(symbols) = symbols else {
        return true;
    };
    if node.kind() == "impl_item" {
        let start = symbols.len();
        super::calls::extract_impl(&node, source, symbols, refs, "");
        if let Some(container) = symbols.get_mut(start) {
            container.parent_index = Some(parent);
        }
        return true;
    }
    let symbol = match node.kind() {
        "struct_item" | "union_item" => {
            super::symbols::extract_struct(&node, source, Some(parent), "")
        }
        "enum_item" => super::symbols::extract_enum(&node, source, Some(parent), ""),
        "type_item" => super::symbols::extract_type_alias(&node, source, Some(parent), ""),
        _ => super::symbols::extract_function(&node, source, Some(parent), ""),
    };
    let Some(symbol) = symbol else {
        return true;
    };
    let index = symbols.len();
    symbols.push(symbol);
    match node.kind() {
        "enum_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let name = symbols[index].name.clone();
                super::symbols::extract_enum_variants(
                    &body,
                    source,
                    Some(index),
                    &name,
                    symbols,
                    refs,
                );
            }
        }
        "function_item" => {
            super::symbols::extract_fn_signature_type_refs(&node, source, index, refs);
            if let Some(body) = node.child_by_field_name("body") {
                super::calls::extract_calls_from_body_with_symbols(
                    &body,
                    source,
                    index,
                    refs,
                    Some(symbols),
                );
            }
        }
        _ => {}
    }
    true
}

#[cfg(test)]
#[path = "calls_local_items_tests.rs"]
mod tests;
