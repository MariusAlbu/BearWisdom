//! Declaration modifiers belong to navigation spans, while CST child anchors
//! remain valid source identities for parameter and signature correlation.
use super::{BindingId, LexicalBindings, LexicalSyntax};
use crate::types::{ExtractedSymbol, SymbolKind};
use std::collections::HashMap;
use tree_sitter::{Node, Point};

pub(super) fn start(node: Node, syntax: &LexicalSyntax) -> Point {
    if syntax
        .named_declarations
        .iter()
        .chain(syntax.type_declarations)
        .any(|(kind, _)| *kind == node.kind())
    {
        if let Some(parent) = node
            .parent()
            .filter(|p| syntax.declaration_modifiers.contains(&p.kind()))
        {
            return parent.start_position();
        }
    }
    node.start_position()
}

pub(super) fn normalize(
    graph: &mut LexicalBindings,
    declarations: &[(Node, BindingId, SymbolKind)],
    symbols: &mut [ExtractedSymbol],
    syntax: &LexicalSyntax,
) {
    let mut anchors: HashMap<_, Vec<usize>> = HashMap::new();
    for (slot, symbol) in symbols.iter().enumerate() {
        anchors
            .entry((symbol.start_line, symbol.start_col, symbol.kind))
            .or_default()
            .push(slot);
    }
    let mut owners = HashMap::new();
    for &(node, _, kind) in declarations {
        let old = node.start_position();
        let new = start(node, syntax);
        if old == new {
            continue;
        }
        let old = (old.row as u32, old.column as u32);
        let new = (new.row as u32, new.column as u32);
        let slots = anchors
            .get(&(new.0, new.1, kind))
            .or_else(|| anchors.get(&(old.0, old.1, kind)));
        let Some(slots) = slots else {
            continue;
        };
        let [slot] = slots.as_slice() else {
            graph.declaration_slots.insert(old, None);
            continue;
        };
        let symbol = &mut symbols[*slot];
        symbol.start_line = new.0;
        symbol.start_col = new.1;
        symbol.byte_offset = node
            .parent()
            .expect("attested modifier wrapper")
            .start_byte() as u32;
        graph.declaration_slots.insert(old, Some(*slot));
        owners.insert(old, new);
    }
    for (row, col, _) in graph.type_parameters.values_mut() {
        if let Some(&(new_row, new_col)) = owners.get(&(*row, *col)) {
            *row = new_row;
            *col = new_col;
        }
    }
}

#[cfg(test)]
#[path = "lexical_declaration_sites_tests.rs"]
mod tests;
