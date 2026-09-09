//! Syntax-profile decoding shared by symbol and call ingestion.
use crate::types::SymbolKind;
use tree_sitter::Node;

pub(super) fn private_kind(node: Node) -> Option<SymbolKind> {
    node.child_by_field_name("name")?;
    super::flow::TS_LEXICAL_SYNTAX
        .named_expressions
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
        .map(|&(_, kind)| kind)
}

pub(super) fn declaration_kind(node: Node) -> &'static str {
    match private_kind(node) {
        Some(SymbolKind::Class) => "class_declaration",
        Some(SymbolKind::Function) => "function_declaration",
        _ => node.kind(),
    }
}

#[cfg(test)]
#[path = "expressions_tests.rs"]
mod tests;
