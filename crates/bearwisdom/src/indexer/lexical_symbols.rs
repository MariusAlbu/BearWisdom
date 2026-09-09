//! Source-coordinate bridge to extractor rows. Never match declarations by
//! spelling, and never reorder slots already used by extracted references.
use super::{BindingId, LexicalBindings};
use crate::indexer::flow_bindings::BindingSymbols;
use crate::types::{ExtractedSymbol, SymbolKind};
use std::collections::HashMap;
use tree_sitter::Node;

#[cfg(test)]
#[path = "lexical_symbols_tests.rs"]
mod tests;

pub(crate) fn reconcile(
    graph: &mut LexicalBindings,
    declarations: &[(Node, BindingId, SymbolKind)],
    source: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    policy: BindingSymbols,
) {
    let mut anchors: HashMap<_, Vec<usize>> = HashMap::new();
    for (slot, symbol) in symbols.iter().enumerate() {
        anchors
            .entry((symbol.start_line, symbol.start_col, symbol.kind))
            .or_default()
            .push(slot);
    }
    for (&(row, col), &slot) in &graph.declaration_slots {
        if let Some(slot) = slot {
            anchors
                .entry((row, col, symbols[slot].kind))
                .or_default()
                .push(slot);
        }
    }
    // Sorted intervals make parent correlation independent of symbol order
    // and same-line ties, without scanning all symbols for every binding.
    let mut containers: Vec<_> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            !matches!(
                s.kind,
                SymbolKind::Parameter
                    | SymbolKind::Variable
                    | SymbolKind::Field
                    | SymbolKind::Property
            )
        })
        .map(|(slot, s)| ((s.start_line, s.start_col), (s.end_line, s.end_col), slot))
        .collect();
    containers.sort_unstable_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end)));
    let mut active = Vec::new();
    let mut next = 0;
    // Pattern traversal can visit parameters before nested declaration nodes.
    let mut ordered: Vec<_> = declarations.iter().collect();
    ordered.sort_unstable_by_key(|(node, _, _)| node.start_byte());
    for &(node, binding, kind) in ordered {
        let p = node.start_position();
        let point = (p.row as u32, p.column as u32);
        while next < containers.len() && containers[next].0 <= point {
            let entry = containers[next];
            while active
                .last()
                .is_some_and(|&i: &usize| containers[i].1 <= entry.0)
            {
                active.pop();
            }
            active.push(next);
            next += 1;
        }
        while active
            .last()
            .is_some_and(|&i: &usize| containers[i].1 <= point)
        {
            active.pop();
        }
        if let Some(slots) = anchors.get(&(point.0, point.1, kind)) {
            for &slot in slots {
                graph.attach_symbol(slot, binding);
            }
            continue;
        }
        if policy == BindingSymbols::CorrelateOnly
            || !matches!(kind, SymbolKind::Parameter | SymbolKind::Variable)
        {
            continue;
        }
        // Preserve the existing no-file-scope-synthesis guard: symbol-less
        // files still use source slot zero and need a dedicated module row.
        let Some(&container) = active.last() else {
            continue;
        };
        let parent_index = containers[container].2;
        let Ok(name) = node.utf8_text(source) else {
            continue;
        };
        let parent = &symbols[parent_index];
        let end = node.end_position();
        let slot = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.to_owned(),
            qualified_name: format!("{}.{name}", parent.qualified_name),
            kind,
            visibility: None,
            start_line: point.0,
            start_col: point.1,
            end_line: end.row as u32,
            end_col: end.column as u32,
            byte_offset: node.start_byte() as u32,
            signature: None,
            doc_comment: None,
            scope_path: Some(parent.qualified_name.clone()),
            parent_index: Some(parent_index),
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
        anchors.insert((point.0, point.1, kind), vec![slot]);
        graph.attach_symbol(slot, binding);
    }
}
