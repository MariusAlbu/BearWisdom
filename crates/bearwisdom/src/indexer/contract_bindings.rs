//! Rebuild source-bound declaration metadata after external row filtering.
//! Cached payloads omit flow; filtered rows have different slots. Neither may
//! reuse the old slot-addressed graph, and neither should expose body inference.
use crate::types::ParsedFile;

pub(super) fn restore(pf: &mut ParsedFile) {
    let Some(source) = &pf.content else {
        return;
    };
    let plugin = crate::languages::default_registry().get(&pf.language);
    let Some(config) = plugin.flow_config() else {
        return;
    };
    if plugin.lexical_syntax().is_none()
        && plugin.namespace_forms().is_none()
        && plugin.callback_lexical_adapter().is_none()
    {
        return;
    }
    let Some(grammar) = plugin.grammar(&pf.language) else {
        return;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return;
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return;
    };
    if plugin.requires_correlated_flow_rebuild() {
        // Some plugins retain source-addressed flow products whose ownership
        // cannot survive row filtering. Rebuild them against retained source,
        // correlating only so cold external files never synthesize body
        // symbols absent from the cache.
        pf.flow = super::flow::run_flow_queries_with_tree_and_plugin(
            source,
            config,
            Some(plugin),
            &mut pf.symbols,
            &mut pf.refs,
            &tree,
            super::flow_bindings::BindingSymbols::CorrelateOnly,
        );
    } else {
        let identity = super::flow::identity::capture(
            source,
            Some(plugin),
            config.strategy_prefix,
            &mut pf.symbols,
            &mut pf.refs,
            tree.root_node(),
            super::flow_bindings::BindingSymbols::CorrelateOnly,
        );
        pf.flow.lexical = identity.lexical;
        pf.flow.callback_lexical = identity.callback_lexical;
        pf.flow.namespaces = identity.namespaces;
    }
}

#[cfg(test)]
#[path = "contract_bindings_tests.rs"]
mod tests;
