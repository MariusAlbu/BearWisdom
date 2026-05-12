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
        component_selectors: Vec::new(),
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
            component_selectors: Vec::new(),
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
fn library_import_keyword_not_resolved_to_project_symbol() {
    // `SeleniumLibrary` is a Library import — `Open Browser` should not resolve
    // to a project-internal symbol even if one exists with the same name.
    // In production, `Open Browser` resolves to the robot-seleniumlibrary synthetic.
    // In this unit test, no synthetics are in the index, so we only check the
    // non-resolution against the project-internal symbol.
    let resource = make_file(
        "lib/keywords.robot",
        "robot",
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
    let (index, id_map) = build_index(&[&resource, &caller]);
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
    // The qualified-library guard at step 1 fires (SeleniumLibrary is a library import)
    // and returns None without reaching the project-symbol lookup.
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    let project_sym_id = sym_id(&id_map, "lib/keywords.robot", "Open Browser");
    let resolves_to_project = res.as_ref().map_or(false, |r| r.target_symbol_id == project_sym_id);
    assert!(
        !resolves_to_project,
        "Open Browser must not resolve to the project-internal symbol; got: {res:?}"
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
fn builtin_keyword_resolves_to_none_without_synthetics() {
    // In production, `Log` resolves to the robot-builtin synthetic symbol.
    // In this unit test, the SymbolIndex has no synthetic files, so resolve
    // returns None and infer_external_namespace also returns None (no
    // is_robot_builtin shortcut any more — that was the predicate-stuffing
    // smell being fixed). The correct external classification happens via the
    // resolved synthetic symbol's `origin='external'` in the indexer.
    let file = make_file(
        "tests/log.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![make_ref_plain(0, "Log")],
    );
    let res = resolve_first_ref(&file, &[&file]);
    // Without synthetics in the test index, the resolver correctly returns None.
    // infer_external_namespace also returns None — the external tag comes from
    // the synthetic symbol's origin in production.
    assert!(
        res.is_none(),
        "without synthetics in the test index, Log returns None from resolve"
    );
}

// ---------------------------------------------------------------------------
// 6. Robot dynamic libraries — KEYWORDS dict + get_keyword_names list
// ---------------------------------------------------------------------------

fn make_project_ctx_with_dynamic_libs(
    robot_path: &str,
    py_path: &str,
    library_name: &str,
    keywords: Vec<super::dynamic_keywords::RobotDynamicKeyword>,
) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    let mut library_map = super::library_map::RobotLibraryMap::default();
    library_map.insert(
        robot_path.to_string(),
        vec![super::library_map::RobotPythonLibrary {
            library_name: library_name.to_string(),
            py_file_path: py_path.to_string(),
        }],
    );
    let mut dynamic_keywords = super::dynamic_keywords::RobotDynamicKeywordMap::default();
    dynamic_keywords.insert(py_path.to_string(), keywords);
    ctx.plugin_state.set(super::RobotProjectState {
        library_map,
        resource_basenames: Default::default(),
        dynamic_keywords,
    });
    ctx
}

#[test]
fn dynamic_keyword_resolves_to_owning_class() {
    // `Async Keyword` is exposed by class AsyncDynamicLibrary's
    // `get_keyword_names` list. The resolver should find the class symbol
    // in async.py via the dynamic-keyword ImportEntry and target its id.
    let robot_file = make_file(
        "tests/async_test.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "AsyncDynamicLibrary"),
            make_ref_plain(0, "Async Keyword"),
        ],
    );
    let py_file = make_file(
        "lib/async.py",
        "python",
        vec![make_sym("AsyncDynamicLibrary", SymbolKind::Class)],
        vec![],
    );
    let ctx = make_project_ctx_with_dynamic_libs(
        "tests/async_test.robot",
        "lib/async.py",
        "AsyncDynamicLibrary",
        vec![super::dynamic_keywords::RobotDynamicKeyword {
            normalized_name: "asynckeyword".to_string(),
            class_name: Some("AsyncDynamicLibrary".to_string()),
            method_name: None,
        }],
    );
    let (index, id_map) = build_index(&[&robot_file, &py_file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&robot_file, Some(&ctx));
    let r = &robot_file.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &robot_file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("dynamic keyword should resolve to AsyncDynamicLibrary class");
    assert_eq!(res.strategy, "robot_dynamic_library");
    assert_eq!(
        res.target_symbol_id,
        sym_id(&id_map, "lib/async.py", "AsyncDynamicLibrary")
    );
}

#[test]
fn module_level_keywords_dict_falls_back_to_first_class() {
    // `One Arg` is a module-level KEYWORDS dict key with no owning class.
    // The resolver picks the file's first class symbol as the closest
    // legal target (the dispatch class that uses `dict(KEYWORDS, ...)`).
    let robot_file = make_file(
        "tests/dyn_test.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "DynamicWithoutKwargs"),
            make_ref_plain(0, "One Arg"),
        ],
    );
    let py_file = make_file(
        "lib/dyn.py",
        "python",
        vec![make_sym("DynamicWithoutKwargs", SymbolKind::Class)],
        vec![],
    );
    let ctx = make_project_ctx_with_dynamic_libs(
        "tests/dyn_test.robot",
        "lib/dyn.py",
        "DynamicWithoutKwargs",
        vec![super::dynamic_keywords::RobotDynamicKeyword {
            normalized_name: "onearg".to_string(),
            class_name: None, // module-level KEYWORDS dict
            method_name: None,
        }],
    );
    let (index, id_map) = build_index(&[&robot_file, &py_file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&robot_file, Some(&ctx));
    let r = &robot_file.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &robot_file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("module-level dynamic keyword should fall back to first class");
    assert_eq!(res.strategy, "robot_dynamic_library_fallback");
    assert_eq!(
        res.target_symbol_id,
        sym_id(&id_map, "lib/dyn.py", "DynamicWithoutKwargs")
    );
}

#[test]
fn keyword_decorator_alias_resolves_to_specific_method() {
    // `@keyword("Custom Name")` ⇒ Robot looks up "Custom Name" but the
    // resolution target is the actual Python method `add_copies_to_cart`,
    // not just the enclosing class.
    let robot_file = make_file(
        "tests/cart.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "Lib"),
            make_ref_plain(0, "Add ${count} copies of ${item} to cart"),
        ],
    );
    let class_sym = make_sym("Lib", SymbolKind::Class);
    let mut method_sym = make_sym("add_copies_to_cart", SymbolKind::Function);
    method_sym.scope_path = Some("Lib".to_string());
    let py_file = make_file(
        "lib/cart_lib.py",
        "python",
        vec![class_sym, method_sym],
        vec![],
    );
    let ctx = make_project_ctx_with_dynamic_libs(
        "tests/cart.robot",
        "lib/cart_lib.py",
        "Lib",
        vec![super::dynamic_keywords::RobotDynamicKeyword {
            normalized_name: super::predicates::_test_normalize_robot_name(
                "Add ${count} copies of ${item} to cart",
            ),
            class_name: Some("Lib".to_string()),
            method_name: Some("add_copies_to_cart".to_string()),
        }],
    );
    let (index, id_map) = build_index(&[&robot_file, &py_file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&robot_file, Some(&ctx));
    let r = &robot_file.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &robot_file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("decorator alias should resolve to its method");
    assert_eq!(res.strategy, "robot_dynamic_library_method");
    assert_eq!(
        res.target_symbol_id,
        sym_id(&id_map, "lib/cart_lib.py", "add_copies_to_cart")
    );
}

#[test]
fn dynamic_keyword_normalization_matches_call_site() {
    // The call site uses Robot's title-case + spaces form ("Get Keyword
    // That Passes"), while the dynamic_keywords map stores the normalised
    // snake form ("get_keyword_that_passes"). The resolver normalises the
    // call before comparing — so the title-case form should still hit
    // the entry.
    let robot_file = make_file(
        "tests/get.robot",
        "robot",
        vec![make_sym("My Test", SymbolKind::Test)],
        vec![
            make_import(0, "GetKeywordNamesLibrary"),
            make_ref_plain(0, "Get Keyword That Passes"),
        ],
    );
    let py_file = make_file(
        "lib/g.py",
        "python",
        vec![make_sym("GetKeywordNamesLibrary", SymbolKind::Class)],
        vec![],
    );
    let ctx = make_project_ctx_with_dynamic_libs(
        "tests/get.robot",
        "lib/g.py",
        "GetKeywordNamesLibrary",
        vec![super::dynamic_keywords::RobotDynamicKeyword {
            normalized_name: "getkeywordthatpasses".to_string(),
            class_name: Some("GetKeywordNamesLibrary".to_string()),
            method_name: None,
        }],
    );
    let (index, _) = build_index(&[&robot_file, &py_file]);
    let resolver = RobotResolver;
    let file_ctx = resolver.build_file_context(&robot_file, Some(&ctx));
    let r = &robot_file.refs[1];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &robot_file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("normalised keyword should match call-site spaces form");
    assert_eq!(res.strategy, "robot_dynamic_library");
}

#[test]
fn strip_bdd_prefix_handles_multibyte_names() {
    use super::predicates::_test_strip_bdd_prefix;

    // Pre-fix this would panic: "Straße" has 'ß' as a 2-byte UTF-8 char at
    // bytes 4..6, so the old `lower[..5]` slice for the "When " prefix
    // landed mid-character.
    let name = "Straße";
    assert_eq!(_test_strip_bdd_prefix(name), "Straße");

    // Multibyte with a real BDD prefix: should still strip cleanly.
    let with_when = "When Straße ist leer";
    assert_eq!(_test_strip_bdd_prefix(with_when), "Straße ist leer");
}
