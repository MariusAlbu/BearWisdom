use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::core::types::TypeArena;
use crate::types::{FlowMeta, ParsedFile};

fn external_library(path: &str) -> ParsedFile {
    ParsedFile {
        path: path.into(),
        language: "dart".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

#[test]
fn external_pub_libraries_contribute_exact_package_uri_aliases() {
    assert_eq!(
        entry_aliases("ext:dart:matcher/src/expect.dart"),
        ["package:matcher/src/expect.dart"]
    );
    assert!(entry_aliases("ext:ts:matcher/src/expect.dart").is_empty());
    assert!(entry_aliases("ext:dart:matcher").is_empty());
}

#[test]
fn schemeless_exports_resolve_relative_to_the_source_library() {
    assert_eq!(
        relative_entry_key("ext:dart:matcher/src/expect/expect.dart", "../equals.dart").as_deref(),
        Some("package:matcher/src/equals.dart")
    );
    assert_eq!(
        relative_entry_key("ext:dart:matcher/expect.dart", "src/expect.dart").as_deref(),
        Some("package:matcher/src/expect.dart")
    );
    assert!(relative_entry_key("ext:dart:matcher/expect.dart", "package:other/x.dart").is_none());
    assert!(relative_entry_key("src/expect.dart", "nested.dart").is_none());
}

#[test]
fn compilation_consumes_pub_entry_aliases_without_widening() {
    let files = [
        external_library("ext:dart:matcher/expect.dart"),
        external_library("ext:dart:matcher/src/expect/expect.dart"),
        external_library("ext:dart:other/src/expect/expect.dart"),
    ];
    let tree = Compilation::build(&files, &HashMap::new().into(), Arc::new(TypeArena::new()));

    assert_eq!(
        tree.resolve_module_from("ext:dart:test/test.dart", "package:matcher/expect.dart"),
        Some("ext:dart:matcher/expect.dart")
    );
    assert_eq!(
        tree.resolve_module_from("ext:dart:matcher/expect.dart", "src/expect/expect.dart"),
        Some("ext:dart:matcher/src/expect/expect.dart")
    );
    assert_eq!(
        tree.resolve_module_from("ext:dart:matcher/expect.dart", "src/expect/missing.dart"),
        None
    );
}
