//! Correlate physical type declarations with their scoped type binding.
use super::{LexicalBindings, LexicalSyntax};
use crate::types::ExtractedSymbol;

pub(super) fn capture(
    graph: &mut LexicalBindings,
    symbols: &[ExtractedSymbol],
    syntax: &LexicalSyntax,
) {
    let types: std::collections::HashSet<_> = graph.type_entries.values().copied().collect();
    let mut parameters = std::collections::HashMap::new();
    for &(row, col, index) in graph.type_parameters.values() {
        parameters
            .entry((row, col))
            .and_modify(|n: &mut usize| *n = (*n).max(index + 1))
            .or_insert(index + 1);
    }
    for (&slot, &binding) in &graph.symbols {
        let binding = graph.dual_types.get(&binding).copied().unwrap_or(binding);
        if types.contains(&binding) {
            graph
                .type_symbol_slots
                .entry(binding)
                .or_default()
                .push(slot);
        }
    }
    for (&binding, slots) in &mut graph.type_symbol_slots {
        slots.sort_unstable();
        let arity = |symbol: &ExtractedSymbol| {
            parameters
                .get(&(symbol.start_line, symbol.start_col))
                .copied()
                .unwrap_or(0)
        };
        let mut kinds = std::collections::HashMap::new();
        for &slot in slots.iter() {
            *kinds.entry(symbols[slot].kind).or_insert(0) += 1;
        }
        // Cost depends on the number of declaration kinds, not the square of
        // a potentially large interface-augmentation declaration group.
        let compatible = slots
            .iter()
            .all(|&slot| arity(&symbols[slot]) == arity(&symbols[slots[0]]))
            && kinds.iter().all(|(&left, &count)| {
                kinds.keys().all(|&right| {
                    (left == right && count == 1)
                        || syntax
                            .merge_declarations
                            .iter()
                            .any(|&(a, b)| (left == a && right == b) || (left == b && right == a))
                })
            });
        if slots.len() > 1 && compatible {
            graph.mergeable_types.insert(binding);
        }
    }
}

#[cfg(test)]
#[path = "lexical_merges_tests.rs"]
mod tests;
