// =============================================================================
// robot/resolve_tests.rs — resolver-level tests for Robot Framework
//
// Covers:
//   1. Keyword name normalization — case-insensitive, spaces == underscores
//   2. Library imports are external — not resolved against project index
//   3. Resource imports — keywords resolved by normalized name
//   4. Variable references — ${VAR} resolves to Variables section symbols
//   5. Qualified `Library.Keyword` extraction and external classification
// =============================================================================

use super::extract;
use super::resolve::RobotResolver;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex,
};
use crate::types::*;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn make_ref_plain(source_idx: usize, target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 2,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}
fn make_ref_with_module(source_idx: usize, target: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 2,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}
fn make_import(source_idx: usize, target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: Some(target.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}
fn make_file(path: &str, language: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 10,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn build_index(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files
        .iter()
        .map(|f| ParsedFile {
            path: f.path.clone(),
            language: f.language.clone(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols: f.symbols.clone(),
            refs: f.refs.clone(),
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
            flow: crate::types::FlowMeta::default(),
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

fn sym_id(id_map: &HashMap<(String, String), i64>, file: &str, name: &str) -> i64 {
    *id_map
        .get(&(file.to_string(), name.to_string()))
        .unwrap_or_else(|| panic!("symbol not found: {file}::{name}"))
}

fn resolve_first_ref(
    file: &ParsedFile,
    all_files: &[&ParsedFile],
) -> Option<crate::indexer::resolve::engine::Resolution> {
    let (index, _) = build_index(all_files);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(file, None);
    let r = file.refs.first()?;
    let src_sym = &file.symbols[r.source_symbol_index];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: src_sym,
        scope_chain: build_scope_chain(src_sym.scope_path.as_deref()),
    file_package_id: None,
    };
    resolver.resolve(&file_ctx, &ref_ctx, &index)
}

fn infer_ns_first_ref(file: &ParsedFile, all_files: &[&ParsedFile]) -> Option<String> {
    let (index, _) = build_index(all_files);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(file, None);
    let r = file.refs.first()?;
    let src_sym = &file.symbols[r.source_symbol_index];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: src_sym,
        scope_chain: build_scope_chain(src_sym.scope_path.as_deref()),
    file_package_id: None,
    };
    resolver.infer_external_namespace(&file_ctx, &ref_ctx, None)
}

// ---------------------------------------------------------------------------
// 1. Keyword name normalization
// ---------------------------------------------------------------------------

#[test]
fn resolve_same_file_exact_name() {
    // `Click Element` called with exact casing → resolves to same-file Function.
    let file = make_file(
        "tests/login.robot",
        "robot",
        vec![
            make_sym("Login Test", SymbolKind::Test),
            make_sym("Click Element", SymbolKind::Function),
        ],
        vec![make_ref_plain(0, "Click Element")],
    );
    let (index, id_map) = build_index(&[&file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let r = &file.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("should resolve");
    assert_eq!(res.strategy, "robot_same_file");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "tests/login.robot", "Click Element"));
}

#[test]
fn resolve_same_file_case_insensitive() {
    // `click element` (lowercase) should match `Click Element` keyword.
    let file = make_file(
        "tests/login.robot",
        "robot",
        vec![
            make_sym("Login Test", SymbolKind::Test),
            make_sym("Click Element", SymbolKind::Function),
        ],
        vec![make_ref_plain(0, "click element")],
    );
    let (index, id_map) = build_index(&[&file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let r = &file.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("case-insensitive match");
    assert_eq!(res.strategy, "robot_same_file");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "tests/login.robot", "Click Element"));
}

#[test]
fn resolve_same_file_underscore_space_equivalence() {
    // `click_element` (underscore form) should match `Click Element`.
    let file = make_file(
        "tests/login.robot",
        "robot",
        vec![
            make_sym("Login Test", SymbolKind::Test),
            make_sym("Click Element", SymbolKind::Function),
        ],
        vec![make_ref_plain(0, "click_element")],
    );
    let (index, id_map) = build_index(&[&file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let r = &file.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index)
        .expect("underscore/space normalization should match");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "tests/login.robot", "Click Element"));
}

// ---------------------------------------------------------------------------
// 2. Library imports are external — not resolved against project index
// ---------------------------------------------------------------------------

#[test]
fn library_import_keyword_not_resolved() {
    // `SeleniumLibrary` is a Library import — `Open Browser` should not resolve
    // to any project symbol even if a symbol named "Open Browser" exists.
    let resource = make_file(
        "lib/keywords.robot",
        "robot",
        // Simulate a project file that accidentally has the same name.
        vec![make_sym("Open Browser", SymbolKind::Function)],
        vec![],
    );
    let caller = make_file(
        "tests/browser.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "SeleniumLibrary"),
            make_ref_plain(0, "Open Browser"),
        ],
    );
    // For just the first ref (import), resolve returns None.
    // For the second ref (call), it should NOT resolve to the project symbol.
    let (index, _) = build_index(&[&resource, &caller]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&caller, None);

    // Second ref is the `Open Browser` call.
    let r = &caller.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    // `Open Browser` is a known BuiltIn keyword — should return None from resolve.
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        res.is_none(),
        "BuiltIn `Open Browser` should not resolve to project symbol; got: {res:?}"
    );
}

#[test]
fn library_import_classified_external() {
    // `Library  SeleniumLibrary` import should be classified as external namespace.
    let file = make_file(
        "tests/browser.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![make_import(0, "SeleniumLibrary")],
    );
    let ns = infer_ns_first_ref(&file, &[&file]);
    assert_eq!(
        ns.as_deref(),
        Some("robot"),
        "SeleniumLibrary import should be classified as robot external"
    );
}

#[test]
fn resource_import_not_external() {
    // `Resource  common.robot` import should NOT be classified as external.
    let file = make_file(
        "tests/browser.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![make_import(0, "common.robot")],
    );
    let ns = infer_ns_first_ref(&file, &[&file]);
    assert!(
        ns.is_none(),
        "Resource .robot import should not be classified as external; got: {ns:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Resource imports — keywords resolved by normalized name
// ---------------------------------------------------------------------------

#[test]
fn resolve_resource_import_exact() {
    // `common.robot` resource import brings `Setup Database` into scope.
    let common = make_file(
        "common.robot",
        "robot",
        vec![make_sym("Setup Database", SymbolKind::Function)],
        vec![],
    );
    let caller = make_file(
        "tests/suite.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "common.robot"),
            make_ref_plain(0, "Setup Database"),
        ],
    );
    let (index, id_map) = build_index(&[&common, &caller]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&caller, None);
    let r = &caller.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("resource import resolution");
    assert_eq!(res.strategy, "robot_resource_import");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "common.robot", "Setup Database"));
}

#[test]
fn resolve_resource_import_normalized() {
    // `setup_database` (underscore) should match `Setup Database` in resource file.
    let common = make_file(
        "common.robot",
        "robot",
        vec![make_sym("Setup Database", SymbolKind::Function)],
        vec![],
    );
    let caller = make_file(
        "tests/suite.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "common.robot"),
            make_ref_plain(0, "setup_database"),
        ],
    );
    let (index, id_map) = build_index(&[&common, &caller]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&caller, None);
    let r = &caller.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("normalized resource import resolution");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "common.robot", "Setup Database"));
}

// ---------------------------------------------------------------------------
// 4. Variable references — ${VAR} resolves to Variables section symbols
// ---------------------------------------------------------------------------

#[test]
fn resolve_variable_same_file() {
    // `${HOST}` reference should resolve to the `HOST` Variable symbol in the same file.
    let file = make_file(
        "tests/config.robot",
        "robot",
        vec![
            make_sym("HOST", SymbolKind::Variable),
            make_sym("Connect", SymbolKind::Function),
        ],
        vec![make_ref_plain(1, "${HOST}")],
    );
    let (index, id_map) = build_index(&[&file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let r = &file.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &file.symbols[1],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("variable resolution");
    assert_eq!(res.strategy, "robot_variable_same_file");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "tests/config.robot", "HOST"));
}

#[test]
fn resolve_variable_case_insensitive() {
    // `${host}` (lowercase) should match `HOST` Variable symbol.
    let file = make_file(
        "tests/config.robot",
        "robot",
        vec![
            make_sym("HOST", SymbolKind::Variable),
            make_sym("Connect", SymbolKind::Function),
        ],
        vec![make_ref_plain(1, "${host}")],
    );
    let (index, id_map) = build_index(&[&file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let r = &file.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &file.symbols[1],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("case-insensitive variable resolution");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "tests/config.robot", "HOST"));
}

#[test]
fn resolve_variable_from_resource() {
    // `${DB_URL}` defined in imported resource file should resolve there.
    let vars_file = make_file(
        "vars/common.robot",
        "robot",
        vec![make_sym("DB_URL", SymbolKind::Variable)],
        vec![],
    );
    let caller = make_file(
        "tests/db.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "vars/common.robot"),
            make_ref_plain(0, "${DB_URL}"),
        ],
    );
    let (index, id_map) = build_index(&[&vars_file, &caller]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&caller, None);
    let r = &caller.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("variable from resource resolution");
    assert_eq!(res.strategy, "robot_variable_resource");
    assert_eq!(res.target_symbol_id, sym_id(&id_map, "vars/common.robot", "DB_URL"));
}

#[test]
fn unresolved_variable_is_classified_external() {
    // `${ENV_VAR}` not in any indexed file → external.
    let file = make_file(
        "tests/env.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![make_ref_plain(0, "${ENV_VAR}")],
    );
    let ns = infer_ns_first_ref(&file, &[&file]);
    assert_eq!(
        ns.as_deref(),
        Some("robot"),
        "unresolved variable should be classified as robot external"
    );
}

// ---------------------------------------------------------------------------
// 5. Qualified `Library.Keyword` — extraction and external classification
// ---------------------------------------------------------------------------

#[test]
fn extractor_splits_qualified_keyword() {
    // `SeleniumLibrary.Click Element` → target_name="Click Element", module="SeleniumLibrary"
    let src = "*** Settings ***\nLibrary    SeleniumLibrary\n*** Test Cases ***\nLogin\n    SeleniumLibrary.Click Element    id=btn\n";
    let result = extract::extract(src);
    let call = result.refs.iter().find(|r| r.kind == EdgeKind::Calls);
    assert!(call.is_some(), "expected a Calls ref; got: {:?}", result.refs);
    let call = call.unwrap();
    assert_eq!(call.target_name, "Click Element", "target should be keyword without library prefix");
    assert_eq!(
        call.module.as_deref(),
        Some("SeleniumLibrary"),
        "module should be the library name"
    );
}

#[test]
fn qualified_library_keyword_not_resolved() {
    // `SeleniumLibrary.Click Element` with split form should not resolve to project symbols.
    let resource = make_file(
        "lib/keywords.robot",
        "robot",
        vec![make_sym("Click Element", SymbolKind::Function)],
        vec![],
    );
    let caller = make_file(
        "tests/browser.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "SeleniumLibrary"),
            make_ref_with_module(0, "Click Element", "SeleniumLibrary"),
        ],
    );
    let (index, _) = build_index(&[&resource, &caller]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&caller, None);
    let r = &caller.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        res.is_none(),
        "SeleniumLibrary.Click Element should not resolve to project symbol; got: {res:?}"
    );
}

#[test]
fn qualified_library_keyword_external_namespace() {
    // `SeleniumLibrary.Click Element` → namespace should be "SeleniumLibrary".
    let caller = make_file(
        "tests/browser.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "SeleniumLibrary"),
            make_ref_with_module(0, "Click Element", "SeleniumLibrary"),
        ],
    );
    let (index, _) = build_index(&[&caller]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&caller, None);
    let r = &caller.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(
        ns.as_deref(),
        Some("SeleniumLibrary"),
        "qualified library keyword should get library name as namespace"
    );
}

#[test]
fn builtin_keyword_classified_external() {
    // `Log` is a BuiltIn keyword — should be classified as external.
    let file = make_file(
        "tests/log.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![make_ref_plain(0, "Log")],
    );
    let ns = infer_ns_first_ref(&file, &[&file]);
    assert_eq!(
        ns.as_deref(),
        Some("robot"),
        "BuiltIn keyword `Log` should be classified as robot external"
    );
}
