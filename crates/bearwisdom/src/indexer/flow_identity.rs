//! Source identity capture is independent of optional flow-query budgets.
use super::BindingSymbols;
use crate::types::{ExtractedRef, ExtractedSymbol, FlowMeta};

pub(in crate::indexer) fn capture(
    source: &str,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    _prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut [ExtractedRef],
    root: tree_sitter::Node,
    bindings: BindingSymbols,
) -> FlowMeta {
    let src = source.as_bytes();
    let mut meta = FlowMeta::default();
    meta.ref_byte_offsets = refs.iter().map(|r| r.byte_offset).collect();
    let namespace_forms = plugin.and_then(|plugin| plugin.namespace_forms());
    let lexical_syntax = plugin.and_then(|plugin| plugin.lexical_syntax());
    crate::indexer::namespaces::stamp_calls(root, src, namespace_forms, refs);
    meta.lexical = crate::indexer::lexical::capture_with_cfg(
        root,
        src,
        lexical_syntax,
        symbols,
        refs,
        bindings,
        plugin.and_then(|plugin| plugin.flow_cfg_node_kinds()),
    );
    meta.callback_lexical = crate::indexer::callback_lexical::capture(
        root,
        src,
        plugin.and_then(|plugin| plugin.callback_lexical_adapter()),
        refs,
    );
    meta.namespaces =
        crate::indexer::namespaces::capture(root, src, namespace_forms, symbols, refs);
    if let Some(data) = &mut meta.namespaces {
        meta.lexical = Some(crate::indexer::namespaces::capture_locals(
            data,
            root,
            src,
            namespace_forms,
            symbols,
            refs,
            bindings,
        ));
    }
    meta
}

#[cfg(test)]
#[path = "flow_identity_tests.rs"]
mod tests;
