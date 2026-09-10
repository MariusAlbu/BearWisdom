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
    if super::lexical::syntax_for(config.strategy_prefix).is_none()
        && super::namespaces::syntax_for(config.strategy_prefix).is_none()
        && !super::callback_lexical::supports(config.strategy_prefix)
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
    let identity = super::flow::identity::capture(
        source,
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

#[cfg(test)]
#[path = "contract_bindings_tests.rs"]
mod tests;
