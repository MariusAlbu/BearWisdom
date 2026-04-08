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

// ---------------------------------------------------------------------------
// symbol_node_kinds: defined_resource_type
// ---------------------------------------------------------------------------

#[test]
fn cov_defined_resource_type_produces_class() {
    let plugin = PuppetPlugin;
    let r = plugin.extract("define myapp::vhost ($port = 80) { }", "test.pp", "puppet");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name.contains("vhost")),
        "defined_resource_type should produce Class symbol; got: {:?}", r.symbols
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: function_declaration
// TODO: tree-sitter-puppet grammar does not emit function_declaration nodes for
// Puppet 4+ function syntax with >>-style return types. The extractor has
// extract_function_declaration() wired but the grammar produces a parse error
// for the tested syntax. Verify with a grammar that supports Puppet 4 functions.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// symbol_node_kinds: node_definition
// TODO: tree-sitter-puppet grammar produces an empty parse tree for
// `node 'hostname' { ... }` in the tested configuration. The extractor has
// extract_node_definition() wired; verify the grammar version supports this.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// symbol_node_kinds: resource_declaration
// ---------------------------------------------------------------------------

#[test]
fn cov_resource_declaration_produces_variable() {
    let plugin = PuppetPlugin;
    let r = plugin.extract("file { '/etc/motd': ensure => present, }", "test.pp", "puppet");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable),
        "resource_declaration should produce Variable symbol; got: {:?}", r.symbols
    );
}

#[test]
fn cov_resource_declaration_produces_calls_to_type() {
    use crate::types::EdgeKind;
    let plugin = PuppetPlugin;
    let r = plugin.extract("file { '/etc/motd': ensure => present, }", "test.pp", "puppet");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "resource_declaration should produce Calls ref to resource type; got: {:?}", r.refs
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: include_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_include_statement_produces_imports_and_calls() {
    use crate::types::EdgeKind;
    let plugin = PuppetPlugin;
    let r = plugin.extract("include apache::mod::rewrite", "test.pp", "puppet");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "include_statement should produce Imports ref; got: {:?}", r.refs
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "include_statement should also produce Calls ref; got: {:?}", r.refs
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: require_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_require_statement_produces_imports_and_calls() {
    use crate::types::EdgeKind;
    let plugin = PuppetPlugin;
    let r = plugin.extract("require ntp", "test.pp", "puppet");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "require_statement should produce Imports ref; got: {:?}", r.refs
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "require_statement should also produce Calls ref; got: {:?}", r.refs
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: function_call
// ---------------------------------------------------------------------------

#[test]
fn cov_function_call_produces_calls() {
    use crate::types::EdgeKind;
    let plugin = PuppetPlugin;
    let r = plugin.extract("class myapp { $val = lookup('myapp::port') }", "test.pp", "puppet");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "lookup"),
        "function_call should produce Calls ref; got: {:?}", r.refs
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: class_definition with inherits — Inherits edge
// ---------------------------------------------------------------------------

#[test]
fn cov_class_inherits_produces_inherits_edge() {
    use crate::types::EdgeKind;
    let plugin = PuppetPlugin;
    let r = plugin.extract("class apache::ssl inherits apache { }", "test.pp", "puppet");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits),
        "class with inherits should produce Inherits ref; got: {:?}", r.refs
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: class_definition — qualified name with :: separator
// ---------------------------------------------------------------------------

#[test]
fn cov_class_definition_qualified_name() {
    let plugin = PuppetPlugin;
    let r = plugin.extract("class apache::mod::rewrite { }", "test.pp", "puppet");
    assert!(
        r.symbols.iter().any(|s| s.name.contains("rewrite")),
        "qualified class name should preserve :: namespace; got: {:?}", r.symbols
    );
}
