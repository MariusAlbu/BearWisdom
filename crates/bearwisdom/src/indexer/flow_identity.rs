//! Source identity capture is independent of optional flow-query budgets.
use super::BindingSymbols;
use crate::types::{ExtractedRef, ExtractedSymbol, FlowMeta};

pub(in crate::indexer) fn capture(
    source: &str,
    prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut [ExtractedRef],
    root: tree_sitter::Node,
    bindings: BindingSymbols,
) -> FlowMeta {
    let src = source.as_bytes();
    let mut meta = FlowMeta::default();
    meta.ref_byte_offsets = refs.iter().map(|r| r.byte_offset).collect();
    crate::indexer::namespaces::stamp_calls(root, src, prefix, refs);
    meta.lexical = crate::indexer::lexical::capture(root, src, prefix, symbols, refs, bindings);
    meta.callback_lexical = crate::indexer::callback_lexical::capture(root, src, prefix, refs);
    meta.namespaces = crate::indexer::namespaces::capture(root, src, prefix, symbols, refs);
    if let Some(data) = &mut meta.namespaces {
        meta.lexical = Some(crate::indexer::namespaces::capture_locals(
            data, root, src, prefix, symbols, refs, bindings,
        ));
    }
    meta
}

#[cfg(test)]
#[path = "flow_identity_tests.rs"]
mod tests;
