// =============================================================================
// puppet/coverage_tests.rs
//
// Node-kind coverage for PuppetPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; mod.rs returns ExtractionResult::empty() pending wiring.
// Tests document expected behaviour once the grammar is active.
// For now they test the plugin's current contract (empty result).
//
// symbol_node_kinds: class_definition, defined_resource_type,
//                   function_declaration, node_definition, resource_declaration
// ref_node_kinds:    include_statement, require_statement,
//                   function_call, resource_declaration
// =============================================================================

use crate::languages::LanguagePlugin;
use crate::languages::puppet::PuppetPlugin;

// ---------------------------------------------------------------------------
// Current contract: no grammar → empty extraction
// ---------------------------------------------------------------------------

#[test]
fn cov_plugin_returns_empty_without_grammar() {
    // Until tree-sitter-puppet is wired in, extract() returns empty.
    let plugin = PuppetPlugin;
    let r = plugin.extract("class myclass { }", "test.pp", "puppet");
    assert!(
        r.symbols.is_empty() && r.refs.is_empty(),
        "Puppet plugin should return empty until grammar is wired; got symbols={:?} refs={:?}",
        r.symbols,
        r.refs
    );
}

#[test]
fn cov_symbol_node_kinds_declared() {
    let plugin = PuppetPlugin;
    assert!(
        plugin.symbol_node_kinds().contains(&"class_definition"),
        "class_definition should be in symbol_node_kinds"
    );
    assert!(
        plugin.symbol_node_kinds().contains(&"function_declaration"),
        "function_declaration should be in symbol_node_kinds"
    );
}

#[test]
fn cov_ref_node_kinds_declared() {
    let plugin = PuppetPlugin;
    assert!(
        plugin.ref_node_kinds().contains(&"include_statement"),
        "include_statement should be in ref_node_kinds"
    );
    assert!(
        plugin.ref_node_kinds().contains(&"function_call"),
        "function_call should be in ref_node_kinds"
    );
}
