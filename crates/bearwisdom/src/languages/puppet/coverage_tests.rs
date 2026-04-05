// =============================================================================
// puppet/coverage_tests.rs
//
// Node-kind coverage for PuppetPlugin::symbol_node_kinds() and ref_node_kinds().
//
// symbol_node_kinds: class_definition, defined_resource_type,
//                   function_declaration, node_definition, resource_declaration
// ref_node_kinds:    include_statement, require_statement,
//                   function_call, resource_declaration
// =============================================================================

use crate::languages::LanguagePlugin;
use crate::languages::puppet::PuppetPlugin;
use crate::types::SymbolKind;

// ---------------------------------------------------------------------------
// Grammar is wired — extraction should produce symbols
// ---------------------------------------------------------------------------

#[test]
fn cov_class_definition_produces_class_symbol() {
    let plugin = PuppetPlugin;
    let r = plugin.extract("class myclass { }", "test.pp", "puppet");
    assert!(
        r.symbols.iter().any(|s| s.name == "myclass" && s.kind == SymbolKind::Class),
        "Puppet plugin should extract Class myclass; got symbols={:?}",
        r.symbols
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
