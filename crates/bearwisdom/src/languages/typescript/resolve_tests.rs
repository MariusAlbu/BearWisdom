use super::hooks::*;
use crate::indexer::resolve::engine::{RefContext};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{build_scope_chain, SymbolIndex};
use crate::types::*;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test helpers (same pattern as csharp_tests.rs)
// ---------------------------------------------------------------------------

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    vis: Visibility,
    scope: Option<&str>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(vis),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope.map(|s| s.to_string()),
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
            col: 0,
}
}
/// Make an import binding ref — the TS extractor emits these as TypeRef with module set.
fn make_import_ref(source_idx: usize, target: &str, module: &str, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::TypeRef,
        line,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
            col: 0,
}
}
fn make_ts_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
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
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }
}

/// Build index from files, assigning sequential IDs.
fn build_test_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
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
            demand_contributions: Vec::new(),
            alias_targets: f.alias_targets.clone(),
            component_selectors: Vec::new(),

            plugin_flow_emissions: Vec::new(),
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

/// Build a minimal ProjectContext with react and @tanstack/react-query as packages.
fn make_ts_project_ctx() -> ProjectContext {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut npm = ManifestData::default();
    npm.dependencies.insert("react".to_string());
    npm.dependencies.insert("react-dom".to_string());
    npm.dependencies.insert("@tanstack/react-query".to_string());
    npm.dependencies.insert("@tanstack".to_string());
    npm.dependencies.insert("express".to_string());
    npm.dependencies.insert("lodash".to_string());
    // Node.js built-ins (subset)
    for builtin in &["fs", "path", "http", "https", "crypto", "os", "events", "stream"] {
        npm.dependencies.insert(builtin.to_string());
    }
    npm.dependencies.insert("node".to_string());
    ctx.manifests.insert(ManifestKind::Npm, npm);
    ctx
}

// ---------------------------------------------------------------------------
// Resolution tests
// ---------------------------------------------------------------------------

#[test]
fn test_same_file_resolution() {
    // A call to a top-level function in the same file resolves via same-file lookup.
    let file = make_ts_file(
        "src/app.ts",
        vec![
            make_symbol("App", "App", SymbolKind::Class, Visibility::Public, None),
            make_symbol(
                "render",
                "App.render",
                SymbolKind::Method,
                Visibility::Public,
                Some("App"),
            ),
            make_symbol(
                "helper",
                "helper",
                SymbolKind::Function,
                Visibility::Public,
                None,
            ),
        ],
        // A Calls ref from render → helper, no module (not an import binding).
        vec![make_ref(1, "helper", EdgeKind::Calls, 5)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[1],
        scope_chain: build_scope_chain(file.symbols[1].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "helper should resolve via same-file");
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    // May resolve via scope_chain ("App.helper" won't exist, so falls through to same_file)
    assert!(
        res.strategy == "ts_same_file" || res.strategy == "ts_scope_chain",
        "unexpected strategy: {}",
        res.strategy
    );
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("src/app.ts".to_string(), "helper".to_string()))
            .unwrap()
    );
}

#[test]
fn test_scope_chain_resolution() {
    // Method call to sibling method within the same class resolves via scope chain.
    let file = make_ts_file(
        "src/service.ts",
        vec![
            make_symbol("Service", "Service", SymbolKind::Class, Visibility::Public, None),
            make_symbol(
                "process",
                "Service.process",
                SymbolKind::Method,
                Visibility::Public,
                Some("Service"),
            ),
            make_symbol(
                "validate",
                "Service.validate",
                SymbolKind::Method,
                Visibility::Public,
                Some("Service"),
            ),
        ],
        vec![make_ref(1, "validate", EdgeKind::Calls, 8)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[1],
        // scope_path = "Service" → scope chain = ["Service"]
        scope_chain: build_scope_chain(file.symbols[1].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "validate should resolve via scope chain");
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "ts_scope_chain");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("src/service.ts".to_string(), "Service.validate".to_string()))
            .unwrap()
    );
}

#[test]
fn test_import_resolution_relative_by_in_file_lookup() {
    // `import { formatDate } from './utils'` — the import binding ref carries
    // module="./utils". We look up by simple name in the target file.
    let utils_file = make_ts_file(
        "./utils",
        vec![make_symbol(
            "formatDate",
            "formatDate",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    // In app.ts: the import binding is represented as a TypeRef ref with module set.
    let app_file = make_ts_file(
        "src/app.ts",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        // The import binding ref: target="formatDate", module="./utils"
        vec![make_import_ref(0, "formatDate", "./utils", 1)],
    );

    let (index, id_map) = build_test_env(&[&utils_file, &app_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&app_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &app_file.refs[0],
        source_symbol: &app_file.symbols[0],
        scope_chain: build_scope_chain(app_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "formatDate should resolve via in-file lookup of ./utils"
    );
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "ts_import_file");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("./utils".to_string(), "formatDate".to_string()))
            .unwrap()
    );
}

#[test]
fn test_import_resolution_by_qualified_name() {
    // The parser emits a qualified name `{module}.{symbol}` — resolved via ts_import.
    // Import module uses the relative specifier form (starts with "./").
    let component_file = make_ts_file(
        "./component.ts",
        vec![make_symbol(
            "Component",
            "./component.ts.Component",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    // Import binding ref: target="Component", module="./component.ts" (relative specifier)
    let app_file = make_ts_file(
        "src/app.ts",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "Component", "./component.ts", 1)],
    );

    let (index, id_map) = build_test_env(&[&component_file, &app_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&app_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &app_file.refs[0],
        source_symbol: &app_file.symbols[0],
        scope_chain: build_scope_chain(app_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "Component should resolve via qualified name or in-file lookup"
    );
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "./component.ts".to_string(),
                "./component.ts.Component".to_string()
            ))
            .unwrap()
    );
}

#[test]
fn test_external_import_not_resolved() {
    // `import { useState } from 'react'` — bare specifier, not in the index.
    // The resolver returns None (falls back to heuristic).
    let app_file = make_ts_file(
        "src/app.tsx",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        // Import binding ref: target="useState", module="react"
        vec![make_import_ref(0, "useState", "react", 1)],
    );

    let (index, _) = build_test_env(&[&app_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&app_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &app_file.refs[0],
        source_symbol: &app_file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_none(), "External package import should not resolve");
}

#[test]
fn test_deep_import_strips_to_package_qname() {
    // `import { tap } from 'rxjs/operators'` — externals index rxjs's operators
    // under the bare `rxjs.` prefix, so `rxjs/operators.tap` misses but
    // `rxjs.tap` (after stripping the deep path) hits.
    let rxjs_file = make_ts_file(
        "ext:ts:rxjs/dist/types/./internal/operators/tap.d.ts",
        vec![make_symbol(
            "tap",
            "rxjs.tap",
            SymbolKind::Function,
            Visibility::Public,
            Some("rxjs"),
        )],
        vec![],
    );

    let app_file = make_ts_file(
        "src/auth.service.ts",
        vec![make_symbol(
            "AuthService",
            "AuthService",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        // Import binding ref: target="tap", module="rxjs/operators"
        vec![make_import_ref(0, "tap", "rxjs/operators", 3)],
    );

    let (index, id_map) = build_test_env(&[&rxjs_file, &app_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&app_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &app_file.refs[0],
        source_symbol: &app_file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    let res = result.expect("rxjs/operators.tap should strip to rxjs.tap");
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "ts_import_deep");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "ext:ts:rxjs/dist/types/./internal/operators/tap.d.ts".to_string(),
                "rxjs.tap".to_string(),
            ))
            .unwrap()
    );
}

#[test]
fn test_deep_import_stops_at_scope_boundary() {
    // `import { Injectable } from '@angular/core/testing'` — when the index
    // only holds `@angular/core.Injectable`, the stripper must descend to
    // `@angular/core` but never past it: a bare `@angular.Injectable` would
    // never be a valid package qname, and stopping there prevents a
    // false-positive resolution if some unrelated `@angular.Injectable`
    // somehow existed.
    let angular_file = make_ts_file(
        "ext:ts:@angular/core/types/core.d.ts",
        vec![make_symbol(
            "Injectable",
            "@angular/core.Injectable",
            SymbolKind::Variable,
            Visibility::Public,
            Some("@angular/core"),
        )],
        vec![],
    );

    let app_file = make_ts_file(
        "src/app.ts",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "Injectable", "@angular/core/testing", 1)],
    );

    let (index, id_map) = build_test_env(&[&angular_file, &app_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&app_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &app_file.refs[0],
        source_symbol: &app_file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    let res = result.expect("@angular/core/testing should strip once to @angular/core");
    assert_eq!(res.confidence, 1.0);
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "ext:ts:@angular/core/types/core.d.ts".to_string(),
                "@angular/core.Injectable".to_string(),
            ))
            .unwrap()
    );
}

#[test]
fn test_deep_import_no_match_returns_none() {
    // When the package prefix doesn't appear in the index at all, stripping
    // shouldn't manufacture a match — the resolver still returns None so
    // Tier 1.5 classifies the ref as external.
    let app_file = make_ts_file(
        "src/app.ts",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "format", "date-fns/utcToZonedTime", 1)],
    );

    let (index, _) = build_test_env(&[&app_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&app_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &app_file.refs[0],
        source_symbol: &app_file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "unknown package deep import should not resolve"
    );
}

#[test]
fn test_qualified_name_resolution() {
    // A dotted reference resolves directly.
    let file1 = make_ts_file(
        "src/types.ts",
        vec![make_symbol(
            "UserRole",
            "types.UserRole",
            SymbolKind::Enum,
            Visibility::Public,
            Some("types"),
        )],
        vec![],
    );

    let file2 = make_ts_file(
        "src/auth.ts",
        vec![make_symbol(
            "Auth",
            "Auth",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_ref(0, "types.UserRole", EdgeKind::TypeRef, 5)],
    );

    let (index, _) = build_test_env(&[&file1, &file2]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Dotted name should resolve directly");
    assert_eq!(result.unwrap().strategy, "ts_qualified_name");
}

#[test]
fn test_falls_back_for_unknown() {
    // A ref to a name not in the index returns None (falls back to heuristic).
    let file = make_ts_file(
        "src/app.ts",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_ref(0, "NonExistentThing", EdgeKind::Calls, 5)],
    );

    let (index, _) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Unknown ref should fall back"
    );
}

// ---------------------------------------------------------------------------
// External namespace inference tests
// ---------------------------------------------------------------------------

#[test]
fn test_infer_external_react_import() {
    // An import binding ref with module="react" is classified as external.
    let ctx = make_ts_project_ctx();
    let file = make_ts_file(
        "src/component.tsx",
        vec![make_symbol(
            "MyComponent",
            "MyComponent",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        // Import binding ref: module carries the bare specifier.
        vec![make_import_ref(0, "useState", "react", 1)],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(ns.is_some(), "useState from 'react' should be inferred as external");
    assert_eq!(ns.unwrap(), "react");
}

#[test]
fn test_infer_external_scoped_package() {
    // `import { useQuery } from '@tanstack/react-query'`
    let ctx = make_ts_project_ctx();
    let file = make_ts_file(
        "src/data.ts",
        vec![make_symbol(
            "DataFetcher",
            "DataFetcher",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "useQuery", "@tanstack/react-query", 1)],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(ns.is_some(), "useQuery should be inferred as external");
    assert_eq!(ns.unwrap(), "@tanstack/react-query");
}

#[test]
fn test_infer_external_node_builtin() {
    // `import { readFile } from 'fs'`
    let ctx = make_ts_project_ctx();
    let file = make_ts_file(
        "src/io.ts",
        vec![make_symbol(
            "FileReader",
            "FileReader",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "readFile", "fs", 1)],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(ns.is_some(), "readFile from 'fs' should be inferred as external");
    assert_eq!(ns.unwrap(), "fs");
}

#[test]
fn test_infer_external_node_protocol() {
    // `import { readFile } from 'node:fs'` — node: protocol always external.
    let ctx = make_ts_project_ctx();
    let file = make_ts_file(
        "src/io.ts",
        vec![make_symbol(
            "FileReader",
            "FileReader",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "readFile", "node:fs", 1)],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(ns.is_some(), "readFile from 'node:fs' should be external");
    assert_eq!(ns.unwrap(), "node:fs");
}

#[test]
fn test_no_external_inference_for_relative_import() {
    // `import { helper } from './utils'` — relative import, NOT external.
    let ctx = make_ts_project_ctx();
    let file = make_ts_file(
        "src/app.ts",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "helper", "./utils", 1)],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(ns.is_none(), "Relative import should not be inferred as external");
}

#[test]
fn test_infer_external_without_project_context() {
    // Without a ProjectContext, any bare specifier is assumed external.
    let file = make_ts_file(
        "src/app.ts",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "someFunc", "some-package", 1)],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, None, &empty_lookup,
        )
    };
    assert!(
        ns.is_some(),
        "Bare specifier should be external without project context"
    );
    assert_eq!(ns.unwrap(), "some-package");
}

#[test]
fn test_infer_external_via_file_ctx_imports() {
    // Non-import ref (no module on the ref itself) — falls back to checking file_ctx.imports.
    // `useState` is imported from 'react', then used in a Calls ref without module.
    let ctx = make_ts_project_ctx();
    let file = make_ts_file(
        "src/component.tsx",
        vec![make_symbol(
            "MyComponent",
            "MyComponent",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![
            // Import binding (has module)
            make_import_ref(0, "useState", "react", 1),
            // Usage ref (no module) — the Calls ref from within the component body
            make_ref(0, "useState", EdgeKind::Calls, 10),
        ],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));

    let usage_ref_ctx = RefContext {
        extracted_ref: &file.refs[1], // Calls ref, no module
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &usage_ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(
        ns.is_some(),
        "useState usage should be inferred external via file_ctx imports"
    );
    assert_eq!(ns.unwrap(), "react");
}

// ---------------------------------------------------------------------------
// Bare specifier helper tests
// ---------------------------------------------------------------------------

#[test]
fn test_is_bare_specifier() {
    assert!(is_bare_specifier("react"));
    assert!(is_bare_specifier("@tanstack/react-query"));
    assert!(is_bare_specifier("node:fs"));
    assert!(is_bare_specifier("lodash/fp"));
    assert!(is_bare_specifier("some-package"));

    assert!(!is_bare_specifier("./utils"));
    assert!(!is_bare_specifier("../shared/types"));
    assert!(!is_bare_specifier("/absolute/path"));
}

// ---------------------------------------------------------------------------
// ProjectContext ts_packages tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_package_json_deps() {
    use crate::indexer::project_context::parse_package_json_deps;

    let package_json = r#"{
        "name": "my-app",
        "dependencies": {
            "react": "^18.0.0",
            "react-dom": "^18.0.0",
            "@tanstack/react-query": "^5.0.0",
            "express": "^4.18.0"
        },
        "devDependencies": {
            "typescript": "^5.0.0",
            "@types/react": "^18.0.0",
            "vite": "^5.0.0"
        }
    }"#;

    let deps = parse_package_json_deps(package_json);
    assert!(deps.contains(&"react".to_string()));
    assert!(deps.contains(&"@tanstack/react-query".to_string()));
    assert!(deps.contains(&"typescript".to_string()));
    assert!(deps.contains(&"@types/react".to_string()));
    assert!(!deps.contains(&"my-app".to_string()));
}

#[test]
fn test_project_context_external_package_lookup() {
    let ctx = make_ts_project_ctx();

    assert!(super::hooks::is_manifest_ts_package(&ctx, None,"react"));
    assert!(super::hooks::is_manifest_ts_package(&ctx, None,"@tanstack/react-query"));
    assert!(super::hooks::is_manifest_ts_package(&ctx, None,"@tanstack"));
    assert!(super::hooks::is_manifest_ts_package(&ctx, None,"fs"));
    assert!(super::hooks::is_manifest_ts_package(&ctx, None,"path"));
    assert!(super::hooks::is_manifest_ts_package(&ctx, None,"node:fs")); // node: protocol always external

    assert!(!super::hooks::is_manifest_ts_package(&ctx, None,"./utils"));
    assert!(!super::hooks::is_manifest_ts_package(&ctx, None,"../shared"));
    assert!(!super::hooks::is_manifest_ts_package(&ctx, None,"MyInternalService"));
}

#[test]
fn test_parse_package_json_invalid() {
    use crate::indexer::project_context::parse_package_json_deps;

    // Invalid JSON should return empty vec, not panic.
    let result = parse_package_json_deps("not json at all {{{");
    assert!(result.is_empty());

    // Empty object is valid.
    let result = parse_package_json_deps("{}");
    assert!(result.is_empty());

    // Missing dependency sections is fine.
    let result = parse_package_json_deps(r#"{"name": "my-app", "version": "1.0.0"}"#);
    assert!(result.is_empty());
}

#[test]
fn test_namespace_import_binding_not_external() {
    // `import * as React from 'react'` — the import binding ref (target="React",
    // module="react") carries a bare specifier. infer_external_namespace returns the
    // package name for it. The Imports edge kind is skipped.
    let ctx = make_ts_project_ctx();
    let file = make_ts_file(
        "src/app.tsx",
        vec![make_symbol(
            "App",
            "App",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![
            // Namespace import binding (TypeRef with module="react")
            make_import_ref(0, "React", "react", 1),
        ],
    );

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
    file_package_id: None,
    };

    // The import binding itself is classified as external.
    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(ns.is_some(), "React namespace import should be classified as external");
    assert_eq!(ns.unwrap(), "react");

    // Resolve returns None (bare specifier, not in index).
    let resolution = resolver.resolve(&file_ctx, &ref_ctx, &index_empty());
    assert!(resolution.is_none());
}

fn index_empty() -> SymbolIndex {
    SymbolIndex::build(&[], &HashMap::new())
}

// ---------------------------------------------------------------------------
// Re-export chain following tests
// ---------------------------------------------------------------------------

/// Build a re-export ref: `export { name } from 'module'`
/// These are emitted by the TS extractor as EdgeKind::Imports with module set.
fn make_reexport_ref(source_idx: usize, exported_name: &str, from_module: &str, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: exported_name.to_string(),
        kind: EdgeKind::Imports,
        line,
        module: Some(from_module.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
            col: 0,
}
}
#[test]
fn test_barrel_named_reexport() {
    // consumer.ts imports UserService from './services' (the barrel).
    // The barrel re-exports UserService from './user.service'.
    // UserService is defined in the source file.
    //
    // NOTE: file paths are set equal to the module specifier strings used in
    // import/re-export refs.  This mirrors the convention in the existing
    // `test_import_resolution_relative_by_in_file_lookup` test — the engine
    // tier does exact-string in_file() lookups, so paths must match specifiers.

    // Definition file: path matches the module string the barrel re-exports from.
    let user_service_file = make_ts_file(
        "./user.service",
        vec![make_symbol(
            "UserService",
            "UserService",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    // Barrel file: path matches the module string in the consumer's import.
    // Its re-export ref points to "./user.service" (the definition file path).
    let barrel_file = make_ts_file(
        "./services",
        vec![],
        vec![make_reexport_ref(0, "UserService", "./user.service", 1)],
    );

    // Consumer: imports UserService from the barrel module path.
    let consumer_file = make_ts_file(
        "src/consumer.ts",
        vec![make_symbol(
            "Consumer",
            "Consumer",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "UserService", "./services", 2)],
    );

    let (index, id_map) = build_test_env(&[&user_service_file, &barrel_file, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[0],
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(consumer_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "UserService should resolve through barrel file");
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "ts_reexport_chain");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("./user.service".to_string(), "UserService".to_string()))
            .unwrap()
    );
}

#[test]
fn test_barrel_aliased_reexport() {
    // export { AuthService as Auth } from './auth.service'
    // The consumer imports as `AuthService` (original name), but the barrel uses alias `Auth`.
    // The extractor stores the original name (`AuthService`) — so it still resolves.

    let auth_file = make_ts_file(
        "./auth.service",
        vec![make_symbol(
            "AuthService",
            "AuthService",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    // Barrel: export { AuthService as Auth } from './auth.service'
    // Extractor emits target_name = "AuthService" (the original, pre-alias name).
    let barrel_file = make_ts_file(
        "./services",
        vec![],
        vec![make_reexport_ref(0, "AuthService", "./auth.service", 1)],
    );

    let consumer_file = make_ts_file(
        "src/consumer.ts",
        vec![make_symbol(
            "Consumer",
            "Consumer",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "AuthService", "./services", 2)],
    );

    let (index, id_map) = build_test_env(&[&auth_file, &barrel_file, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[0],
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(consumer_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "AuthService should resolve through aliased barrel re-export");
    let res = result.unwrap();
    assert_eq!(res.strategy, "ts_reexport_chain");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("./auth.service".to_string(), "AuthService".to_string()))
            .unwrap()
    );
}

#[test]
fn test_barrel_wildcard_reexport() {
    // export * from './utils'
    // Consumer imports `formatDate` from the barrel.

    let utils_file = make_ts_file(
        "./utils",
        vec![make_symbol(
            "formatDate",
            "formatDate",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    // Barrel: export * from './utils'
    let barrel_file = make_ts_file(
        "./index",
        vec![],
        vec![make_reexport_ref(0, "*", "./utils", 1)],
    );

    let consumer_file = make_ts_file(
        "src/consumer.ts",
        vec![make_symbol(
            "Consumer",
            "Consumer",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "formatDate", "./index", 2)],
    );

    let (index, id_map) = build_test_env(&[&utils_file, &barrel_file, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[0],
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(consumer_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "formatDate should resolve through export-star barrel");
    let res = result.unwrap();
    // Wildcard resolution uses 0.95 confidence.
    assert_eq!(res.confidence, 0.95);
    assert_eq!(res.strategy, "ts_reexport_star");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("./utils".to_string(), "formatDate".to_string()))
            .unwrap()
    );
}

#[test]
fn test_barrel_deep_chain() {
    // Two-hop chain:
    //   consumer → barrel/index.ts → services/index.ts → user.service.ts

    // All file paths match the corresponding module specifier strings.
    let definition_file = make_ts_file(
        "./user.service",
        vec![make_symbol(
            "UserService",
            "UserService",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    // First barrel: re-exports from "./user.service"
    let services_barrel = make_ts_file(
        "./services",
        vec![],
        vec![make_reexport_ref(0, "UserService", "./user.service", 1)],
    );

    // Second barrel: re-exports from "./services"
    let root_barrel = make_ts_file(
        "./barrel",
        vec![],
        vec![make_reexport_ref(0, "UserService", "./services", 1)],
    );

    let consumer_file = make_ts_file(
        "src/consumer.ts",
        vec![make_symbol(
            "Consumer",
            "Consumer",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "UserService", "./barrel", 2)],
    );

    let (index, id_map) = build_test_env(&[
        &definition_file,
        &services_barrel,
        &root_barrel,
        &consumer_file,
    ]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[0],
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(consumer_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "UserService should resolve through 2-hop barrel chain");
    assert_eq!(
        result.unwrap().target_symbol_id,
        *id_map
            .get(&("./user.service".to_string(), "UserService".to_string()))
            .unwrap()
    );
}

#[test]
fn test_barrel_depth_limit() {
    // Circular re-export chain: a → b → c → a
    // Should return None without panicking.

    // Circular barrel files — paths match the module specifier strings.
    let barrel_a = make_ts_file(
        "./a",
        vec![],
        vec![make_reexport_ref(0, "Foo", "./b", 1)],
    );
    let barrel_b = make_ts_file(
        "./b",
        vec![],
        vec![make_reexport_ref(0, "Foo", "./c", 1)],
    );
    let barrel_c = make_ts_file(
        "./c",
        vec![],
        vec![make_reexport_ref(0, "Foo", "./a", 1)],
    );

    let consumer_file = make_ts_file(
        "src/consumer.ts",
        vec![make_symbol(
            "Consumer",
            "Consumer",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "Foo", "./a", 2)],
    );

    let (index, _) = build_test_env(&[&barrel_a, &barrel_b, &barrel_c, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[0],
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(consumer_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    // Should not panic and should return None (Foo never defined).
    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_none(), "Circular barrel chain should return None, not panic");
}

// ---------------------------------------------------------------------------
// Workspace package resolution (A3)
// ---------------------------------------------------------------------------

fn make_ts_file_in_pkg(
    path: &str,
    pkg_id: Option<i64>,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    let mut pf = make_ts_file(path, symbols, refs);
    pf.package_id = pkg_id;
    pf
}

#[test]
fn workspace_package_exact_import_resolves_at_confidence_1() {
    // Producer package "@myorg/utils" exports `formatDate` from src/index.ts.
    let producer = make_ts_file_in_pkg(
        "packages/utils/src/index.ts",
        Some(7),
        vec![make_symbol(
            "formatDate",
            "formatDate",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    // Consumer package imports it via the declared_name.
    let consumer = make_ts_file_in_pkg(
        "packages/app/src/main.ts",
        Some(9),
        vec![make_symbol(
            "main",
            "main",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "formatDate", "@myorg/utils", 1)],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&producer, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let mut ctx = ProjectContext::default();
    ctx.workspace_pkg_by_declared_name
        .insert("@myorg/utils".to_string(), 7);

    let parsed = vec![producer, consumer];
    let index = SymbolIndex::build_with_context(&parsed, &id_map, Some(&ctx));
    let consumer_ref = &parsed[1];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[0],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: Some(9),
    };

    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("workspace import should resolve");
    assert_eq!(res.strategy, "ts_workspace_pkg");
    assert_eq!(res.confidence, 1.0);
}

#[test]
fn workspace_package_deep_import_prefers_matching_file() {
    // Producer declares `@myorg/utils` with two files that both export `foo`.
    // Consumer's `@myorg/utils/sub/mod` import must prefer the file whose
    // path contains `sub/mod`.
    let producer_root = make_ts_file_in_pkg(
        "packages/utils/src/index.ts",
        Some(7),
        vec![make_symbol("foo", "foo", SymbolKind::Function, Visibility::Public, None)],
        vec![],
    );
    let producer_sub = make_ts_file_in_pkg(
        "packages/utils/src/sub/mod.ts",
        Some(7),
        vec![make_symbol(
            "foo",
            "sub.mod.foo",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );
    let consumer = make_ts_file_in_pkg(
        "packages/app/src/main.ts",
        Some(9),
        vec![make_symbol(
            "main",
            "main",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "foo", "@myorg/utils/sub/mod", 1)],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&producer_root, &producer_sub, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let mut ctx = ProjectContext::default();
    ctx.workspace_pkg_by_declared_name
        .insert("@myorg/utils".to_string(), 7);
    let parsed = vec![producer_root, producer_sub, consumer];
    let index = SymbolIndex::build_with_context(&parsed, &id_map, Some(&ctx));
    let consumer_ref = &parsed[2];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[0],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: Some(9),
    };

    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("deep import should resolve");
    assert_eq!(res.strategy, "ts_workspace_pkg");
    assert_eq!(res.confidence, 1.0);
    let expected_id = id_map[&(
        "packages/utils/src/sub/mod.ts".to_string(),
        "sub.mod.foo".to_string(),
    )];
    assert_eq!(res.target_symbol_id, expected_id);
}

#[test]
fn workspace_package_import_not_classified_as_external() {
    // Import that references a workspace package must not surface as
    // external even if the resolver's main path didn't land a match.
    let consumer = make_ts_file_in_pkg(
        "packages/app/src/main.ts",
        Some(9),
        vec![make_symbol(
            "main",
            "main",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "missing", "@myorg/utils", 1)],
    );

    let mut ctx = ProjectContext::default();
    ctx.workspace_pkg_by_declared_name
        .insert("@myorg/utils".to_string(), 7);

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer.refs[0],
        source_symbol: &consumer.symbols[0],
        scope_chain: vec![],
        file_package_id: Some(9),
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(
        ns.is_none(),
        "workspace package must not be classified as external, got {ns:?}"
    );
}

#[test]
fn symbol_lookup_symbols_in_package_groups_by_pkg_id() {
    let pf_a = make_ts_file_in_pkg(
        "packages/a/src/a.ts",
        Some(1),
        vec![make_symbol("A", "A", SymbolKind::Class, Visibility::Public, None)],
        vec![],
    );
    let pf_b = make_ts_file_in_pkg(
        "packages/b/src/b.ts",
        Some(2),
        vec![make_symbol("B", "B", SymbolKind::Class, Visibility::Public, None)],
        vec![],
    );
    let pf_root = make_ts_file_in_pkg(
        "tools/script.ts",
        None,
        vec![make_symbol(
            "R",
            "R",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&pf_a, &pf_b, &pf_root] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }

    let parsed = vec![pf_a, pf_b, pf_root];
    let index = SymbolIndex::build(&parsed, &id_map);

    use crate::indexer::resolve::engine::SymbolLookup;
    assert_eq!(index.symbols_in_package(1).len(), 1);
    assert_eq!(index.symbols_in_package(1)[0].qualified_name, "A");
    assert_eq!(index.symbols_in_package(2).len(), 1);
    assert_eq!(index.symbols_in_package(2)[0].qualified_name, "B");
    // Root-scoped symbols (no package_id) do not surface via this index.
    assert!(index.symbols_in_package(99).is_empty());
}

#[test]
fn tsconfig_alias_resolves_bare_specifier() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};

    // Producer at src/utils/format.ts exports `formatDate`.
    let producer = make_ts_file_in_pkg(
        "src/utils/format.ts",
        None,
        vec![make_symbol(
            "formatDate",
            "formatDate",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );
    // Consumer imports `@/utils/format` relying on a `@/* -> src/*` alias.
    let consumer = make_ts_file_in_pkg(
        "src/app/main.ts",
        None,
        vec![make_symbol(
            "main",
            "main",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "formatDate", "@/utils/format", 1)],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&producer, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }

    let mut ctx = ProjectContext::default();
    let mut npm = ManifestData::default();
    npm.tsconfig_paths.push(("@/".to_string(), "src/".to_string()));
    ctx.manifests.insert(ManifestKind::Npm, npm);

    let parsed = vec![producer, consumer];
    let index = SymbolIndex::build_with_context(&parsed, &id_map, Some(&ctx));
    let consumer_ref = &parsed[1];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[0],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };

    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("alias-rewritten import should resolve");
    assert_eq!(res.strategy, "ts_tsconfig_alias");
    assert_eq!(res.confidence, 1.0);
}

#[test]
fn tsconfig_alias_prepends_package_path_in_monorepo() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};

    // Monorepo layout: `apps/landing/tsconfig.json` declares `@/* -> src/*`.
    // The producer lives at `apps/landing/src/components/Button.tsx`. The
    // consumer's `@/components/Button` import must resolve to that file,
    // not the workspace-relative `src/components/Button.tsx` (which doesn't
    // exist).
    let producer = make_ts_file_in_pkg(
        "apps/landing/src/components/Button.tsx",
        Some(7),
        vec![make_symbol(
            "Button",
            "Button",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![],
    );
    let consumer = make_ts_file_in_pkg(
        "apps/landing/src/app/page.tsx",
        Some(7),
        vec![make_symbol(
            "Page",
            "Page",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "Button", "@/components/Button", 1)],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&producer, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }

    let mut ctx = ProjectContext::default();
    let mut landing_npm = ManifestData::default();
    landing_npm
        .tsconfig_paths
        .push(("@/".to_string(), "src/".to_string()));
    let mut by_pkg = std::collections::HashMap::new();
    by_pkg.insert(ManifestKind::Npm, landing_npm);
    ctx.by_package.insert(7, by_pkg);
    ctx.workspace_pkg_paths
        .insert(7, "apps/landing".to_string());

    let parsed = vec![producer, consumer];
    let index = SymbolIndex::build_with_context(&parsed, &id_map, Some(&ctx));
    let consumer_ref = &parsed[1];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[0],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: Some(7),
    };

    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("per-package alias should resolve after path prepend");
    assert_eq!(res.strategy, "ts_tsconfig_alias");
    assert_eq!(res.confidence, 1.0);
    let expected = id_map[&(
        "apps/landing/src/components/Button.tsx".to_string(),
        "Button".to_string(),
    )];
    assert_eq!(res.target_symbol_id, expected);
}

#[test]
fn tsconfig_alias_follows_barrel_reexport() {
    // Pattern: `import { QuickCreateButton } from "@/features/quick-create"`
    // where `@/features/quick-create/index.ts` is a barrel that forwards
    // the name from a neighbour file. The alias rewrite lands in the
    // index.ts — but that file has no own symbols, only re-exports.
    // `resolve_via_alias` must walk the barrel chain.
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};

    let producer = make_ts_file_in_pkg(
        "apps/web/src/features/quick-create/quick-create-button.tsx",
        Some(7),
        vec![make_symbol(
            "QuickCreateButton",
            "QuickCreateButton",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );
    // Barrel: `export { QuickCreateButton } from "./quick-create-button"`
    // The TS extractor emits this as an Imports ref with module set. We
    // build the file with one such ref and no own symbols.
    let barrel_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "QuickCreateButton".to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        col: 0,
        module: Some("./quick-create-button".to_string()),
        chain: None,
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};
    let barrel = make_ts_file_in_pkg(
        "apps/web/src/features/quick-create/index.ts",
        Some(7),
        vec![],
        vec![barrel_ref],
    );
    let consumer = make_ts_file_in_pkg(
        "apps/web/src/app/layout.tsx",
        Some(7),
        vec![make_symbol(
            "Layout",
            "Layout",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "QuickCreateButton", "@/features/quick-create", 1)],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&producer, &barrel, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }

    let mut ctx = ProjectContext::default();
    let mut npm = ManifestData::default();
    npm.tsconfig_paths
        .push(("@/".to_string(), "src/".to_string()));
    let mut by_pkg = std::collections::HashMap::new();
    by_pkg.insert(ManifestKind::Npm, npm);
    ctx.by_package.insert(7, by_pkg);
    ctx.workspace_pkg_paths
        .insert(7, "apps/web".to_string());

    let parsed = vec![producer, barrel, consumer];
    let index = SymbolIndex::build_with_context(&parsed, &id_map, Some(&ctx));
    let consumer_ref = &parsed[2];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[0],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: Some(7),
    };

    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("alias + barrel chain should resolve");
    // Either tsconfig_alias (if landed directly) or reexport_chain (if
    // the barrel walk surfaced the result).
    assert!(
        res.strategy == "ts_tsconfig_alias" || res.strategy == "ts_reexport_chain",
        "got unexpected strategy: {}",
        res.strategy
    );
    let expected = id_map[&(
        "apps/web/src/features/quick-create/quick-create-button.tsx".to_string(),
        "QuickCreateButton".to_string(),
    )];
    assert_eq!(res.target_symbol_id, expected);
}

#[test]
fn tsconfig_alias_longest_prefix_wins() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};

    // Two aliases: @/ → src/ and @/components/ → packages/ui/src/.
    // An import of `@/components/Button` must use the longer mapping.
    let producer = make_ts_file_in_pkg(
        "packages/ui/src/Button.ts",
        None,
        vec![make_symbol(
            "Button",
            "Button",
            SymbolKind::Class,
            Visibility::Public,
            None,
        )],
        vec![],
    );
    let consumer = make_ts_file_in_pkg(
        "src/app/main.ts",
        None,
        vec![make_symbol(
            "main",
            "main",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "Button", "@/components/Button", 1)],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&producer, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }

    let mut ctx = ProjectContext::default();
    let mut npm = ManifestData::default();
    npm.tsconfig_paths.push(("@/".to_string(), "src/".to_string()));
    npm.tsconfig_paths.push((
        "@/components/".to_string(),
        "packages/ui/src/".to_string(),
    ));
    ctx.manifests.insert(ManifestKind::Npm, npm);

    let parsed = vec![producer, consumer];
    let index = SymbolIndex::build_with_context(&parsed, &id_map, Some(&ctx));
    let consumer_ref = &parsed[1];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[0],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };

    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("longer alias prefix should win");
    assert_eq!(res.strategy, "ts_tsconfig_alias");
    let expected_id = id_map[&(
        "packages/ui/src/Button.ts".to_string(),
        "Button".to_string(),
    )];
    assert_eq!(res.target_symbol_id, expected_id);
}

#[test]
fn relative_import_jsx_usage_resolves_via_module_to_file() {
    // `import { cn } from "./lib/utils"` produces two refs: an import
    // binding (TypeRef with module set) and JSX/call usages (module=None,
    // just the target name). The non-module resolver path must handle
    // the relative-import case — without it, usages fall through to the
    // heuristic.
    use crate::indexer::resolve::engine::SymbolIndex;

    let producer = make_ts_file_in_pkg(
        "packages/ui/src/lib/utils.ts",
        None,
        vec![make_symbol(
            "cn",
            "cn",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![],
    );
    let consumer = make_ts_file_in_pkg(
        "packages/ui/src/button.tsx",
        None,
        vec![make_symbol(
            "Button",
            "Button",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![
            // Import binding ref — has module set.
            make_import_ref(0, "cn", "./lib/utils", 1),
            // JSX usage ref — module=None, just the bare target.
            make_ref(0, "cn", EdgeKind::Calls, 5),
        ],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&producer, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }

    let parsed = vec![producer, consumer];
    let index = SymbolIndex::build(&parsed, &id_map);
    let consumer_ref = &parsed[1];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, None);
    // Resolve the JSX usage ref (index 1 — the Calls ref without module).
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[1],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };

    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("JSX usage of relative-imported symbol should resolve");
    assert_eq!(res.strategy, "ts_relative_import");
    assert_eq!(res.confidence, 1.0);
    let expected = id_map[&("packages/ui/src/lib/utils.ts".to_string(), "cn".to_string())];
    assert_eq!(res.target_symbol_id, expected);
}

#[test]
fn passthrough_alias_barrel_classifies_as_external() {
    // Pattern: `import { Trans } from "@/i18n/client/trans"` where
    // `apps/landing/src/i18n/client/trans.tsx` is exactly:
    //   export { Trans } from "react-i18next";
    // The consumer ref must classify as external `react-i18next`, NOT
    // fall through to the heuristic which would pick a wrong same-named
    // symbol elsewhere in the project.
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    use crate::indexer::resolve::engine::SymbolIndex;

    // Barrel: zero own symbols, one re-export ref pointing at a bare spec.
    let barrel_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Trans".to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        col: 0,
        module: Some("react-i18next".to_string()),
        chain: None,
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};
    let barrel = make_ts_file_in_pkg(
        "apps/landing/src/i18n/client/trans.tsx",
        Some(7),
        vec![],
        vec![barrel_ref],
    );
    let consumer = make_ts_file_in_pkg(
        "apps/landing/src/footer.tsx",
        Some(7),
        vec![make_symbol(
            "Footer",
            "Footer",
            SymbolKind::Function,
            Visibility::Public,
            None,
        )],
        vec![make_import_ref(0, "Trans", "@/i18n/client/trans", 1)],
    );

    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in [&barrel, &consumer] {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }

    let mut ctx = ProjectContext::default();
    let mut npm = ManifestData::default();
    npm.tsconfig_paths
        .push(("@/".to_string(), "src/".to_string()));
    let mut by_pkg = std::collections::HashMap::new();
    by_pkg.insert(ManifestKind::Npm, npm);
    ctx.by_package.insert(7, by_pkg);
    ctx.workspace_pkg_paths
        .insert(7, "apps/landing".to_string());

    let parsed = vec![barrel, consumer];
    let index = SymbolIndex::build_with_context(&parsed, &id_map, Some(&ctx));
    let consumer_ref = &parsed[1];

    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(consumer_ref, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &consumer_ref.refs[0],
        source_symbol: &consumer_ref.symbols[0],
        scope_chain: vec![],
        file_package_id: Some(7),
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        crate::languages::typescript::hooks::TypeScriptHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &index,
        )
    };
    assert_eq!(
        ns.as_deref(),
        Some("react-i18next"),
        "passthrough barrel should classify as the external bare spec"
    );
}

#[test]
fn tsconfig_alias_parser_handles_realworld_landing_shape() {
    // Exact shape from ts-rallly's apps/landing/tsconfig.json — the wild
    // case we missed. Has `extends`, `baseUrl`, mixed-type compilerOptions,
    // and trailing commas may or may not appear.
    use crate::ecosystem::manifest::npm::parse_tsconfig_paths;
    let content = r##"{
        "extends": "@rallly/tsconfig/next.json",
        "compilerOptions": {
            "baseUrl": ".",
            "paths": {
                "@/*": ["src/*"],
                "~/*": ["public/*"]
            },
            "checkJs": false,
            "strictNullChecks": true,
            "target": "ES2017"
        },
        "include": ["**/*.ts", "**/*.tsx"],
        "exclude": ["node_modules", ".next"]
    }"##;
    let aliases = parse_tsconfig_paths(content);
    assert_eq!(aliases.len(), 2, "expected 2 aliases, got {aliases:?}");
    assert!(aliases.contains(&("@/".to_string(), "src/".to_string())));
    assert!(aliases.contains(&("~/".to_string(), "public/".to_string())));
}

#[test]
fn tsconfig_alias_parser_extracts_wildcard_mappings() {
    use crate::ecosystem::manifest::npm::parse_tsconfig_paths;

    let tsconfig = r##"{
        // top-of-file comment
        "compilerOptions": {
            "paths": {
                "@/*": ["src/*"],
                "@components/*": ["src/components/*"],
                /* block */ "#no_wildcard": ["src/ignored"],
                "@utils": ["src/utils/index"]
            }
        }
    }"##;
    let aliases = parse_tsconfig_paths(tsconfig);
    assert!(aliases.contains(&("@/".to_string(), "src/".to_string())));
    assert!(aliases.contains(&(
        "@components/".to_string(),
        "src/components/".to_string()
    )));
    // Non-wildcard keys are currently skipped — document that.
    assert!(!aliases.iter().any(|(k, _)| k == "@utils"));
}

// ---------------------------------------------------------------------------
// Call-root chain tests — expect(x).toBe(y) / vitest.expect
// ---------------------------------------------------------------------------

/// `import { expect } from 'chai'; expect(x).toBe(y)` — the chain root
/// `expect` is a bare-specifier import. Phase 1 resolves it by looking up
/// `chai.expect` → `return_type_name` → `chai.Assertion`.
/// Phase 2 walks `chai.Assertion.toBe`, Phase 3 resolves the final segment.
#[test]
fn call_root_chain_expect_from_chai_resolves_to_be() {
    use crate::type_checker::chain::external_type_qname;

    // Build a minimal index manually — the chai chain-type synthetic
    // that used to supply this shape has been deleted, but the chain
    // walker's behaviour is still specified here independently.
    let chai_assertion_sym = make_symbol(
        "Assertion", "chai.Assertion", SymbolKind::Interface, Visibility::Public, Some("chai"),
    );
    let chai_expect_sym = ExtractedSymbol {
        name: "expect".to_string(),
        qualified_name: "chai.expect".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("expect(val: any): chai.Assertion".to_string()),
        doc_comment: None,
        scope_path: Some("chai".to_string()),
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let chai_tobe_sym = ExtractedSymbol {
        name: "toBe".to_string(),
        qualified_name: "chai.Assertion.toBe".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("toBe(...): chai.Assertion".to_string()),
        doc_comment: None,
        scope_path: Some("chai.Assertion".to_string()),
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    // Return-type refs: chai.expect → chai.Assertion, chai.Assertion.toBe → chai.Assertion
    let expect_rt_ref = ExtractedRef {
        source_symbol_index: 1,
        target_name: "chai.Assertion".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};
    let tobe_rt_ref = ExtractedRef {
        source_symbol_index: 2,
        target_name: "chai.Assertion".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};
    let chai_file = ParsedFile {
        path: "ext:ts:chai/__bw_synthetic__.d.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: "synthetic".to_string(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![chai_assertion_sym, chai_expect_sym, chai_tobe_sym],
        refs: vec![expect_rt_ref, tobe_rt_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None, None, None],
        ref_origin_languages: vec![None, None],
        symbol_from_snippet: vec![false, false, false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    // The consumer file: `import { expect } from 'chai'` + the chain ref.
    let chain_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "toBe".to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "expect".to_string(),
                    node_kind: "identifier".to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "toBe".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};
    let import_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "expect".to_string(),
        kind: EdgeKind::TypeRef,
        line: 1,
        col: 0,
        module: Some("chai".to_string()),
        chain: None,
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};
    let test_sym = make_symbol("myTest", "myTest", SymbolKind::Function, Visibility::Public, None);
    let consumer_file = ParsedFile {
        path: "src/app.test.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![test_sym],
        refs: vec![import_ref, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None],
        ref_origin_languages: vec![None, None],
        symbol_from_snippet: vec![false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&chai_file, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[1], // the chain ref for toBe
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "expect(x).toBe(y) chain must resolve via call-root import fallback"
    );
    let res = result.unwrap();
    let tobe_id = id_map
        .get(&(
            "ext:ts:chai/__bw_synthetic__.d.ts".to_string(),
            "chai.Assertion.toBe".to_string(),
        ))
        .expect("chai.Assertion.toBe must be indexed");
    assert_eq!(
        res.target_symbol_id, *tobe_id,
        "chain must resolve to chai.Assertion.toBe"
    );
}

/// `expect(spy).toHaveBeenCalledOnce()` with NO `expect` import (vitest globals mode).
/// Phase 1 must fall through to Pass 3 of `resolve_call_root_type` and find
/// `__npm_globals__.expect` → return_type → `chai.Assertion`. Phase 3 then
/// resolves `chai.Assertion.toHaveBeenCalledOnce` which lives in chai synthetic.
#[test]
fn call_root_chain_expect_global_vitest_resolves_spy_matcher() {
    // Minimal chai synthetic: Assertion interface + toHaveBeenCalledOnce method.
    let chai_assertion_sym = ExtractedSymbol {
        name: "Assertion".to_string(),
        qualified_name: "chai.Assertion".to_string(),
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None,
        scope_path: Some("chai".to_string()),
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let chai_matcher_sym = ExtractedSymbol {
        name: "toHaveBeenCalledOnce".to_string(),
        qualified_name: "chai.Assertion.toHaveBeenCalledOnce".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: Some("toHaveBeenCalledOnce(): void".to_string()),
        doc_comment: None,
        scope_path: Some("chai.Assertion".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    // __npm_globals__.expect → return_type = "chai.Assertion"
    let npm_globals_expect_sym = ExtractedSymbol {
        name: "expect".to_string(),
        qualified_name: "__npm_globals__.expect".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: Some("expect(val: any): chai.Assertion".to_string()),
        doc_comment: None,
        scope_path: Some("__npm_globals__".to_string()),
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    // TypeRef: __npm_globals__.expect → chai.Assertion
    let globals_expect_ref = ExtractedRef {
        source_symbol_index: 2, // npm_globals_expect_sym is index 2
        target_name: "chai.Assertion".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0, module: None, chain: None, byte_offset: 1,
        col: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};

    let synth_file = ParsedFile {
        path: "ext:ts:vitest/__bw_synthetic__.d.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: "synthetic".to_string(),
        size: 0, line_count: 0, mtime: None, package_id: None,
        content: None, has_errors: false,
        symbols: vec![chai_assertion_sym, chai_matcher_sym, npm_globals_expect_sym],
        refs: vec![globals_expect_ref],
        routes: vec![], db_sets: vec![],
        symbol_origin_languages: vec![None, None, None],
        ref_origin_languages: vec![None],
        symbol_from_snippet: vec![false, false, false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    // Consumer file: NO import for `expect` — globals mode.
    let chain_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "toHaveBeenCalledOnce".to_string(),
        kind: EdgeKind::Calls,
        line: 10,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "expect".to_string(),
                    node_kind: "identifier".to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "toHaveBeenCalledOnce".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
};
    let test_sym = make_symbol("myTest", "myTest", SymbolKind::Function, Visibility::Public, None);
    let consumer_file = ParsedFile {
        path: "compat/test/browser/PureComponent.test.jsx".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0, line_count: 0, mtime: None, package_id: None,
        content: None, has_errors: false,
        // No import ref for expect — globals mode.
        symbols: vec![test_sym],
        refs: vec![chain_ref],
        routes: vec![], db_sets: vec![],
        symbol_origin_languages: vec![None],
        ref_origin_languages: vec![None],
        symbol_from_snippet: vec![false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&synth_file, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[0],
        source_symbol: &consumer_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "expect(spy).toHaveBeenCalledOnce() globals-mode chain must resolve via __npm_globals__"
    );
    let res = result.unwrap();
    let matcher_id = id_map
        .get(&(
            "ext:ts:vitest/__bw_synthetic__.d.ts".to_string(),
            "chai.Assertion.toHaveBeenCalledOnce".to_string(),
        ))
        .expect("chai.Assertion.toHaveBeenCalledOnce must be indexed");
    assert_eq!(
        res.target_symbol_id, *matcher_id,
        "chain must resolve to chai.Assertion.toHaveBeenCalledOnce"
    );
}

#[test]
fn project_context_workspace_package_id_handles_deep_imports() {
    let mut ctx = ProjectContext::default();
    ctx.workspace_pkg_by_declared_name
        .insert("@myorg/utils".to_string(), 7);

    assert_eq!(ctx.workspace_package_id("@myorg/utils"), Some(7));
    assert_eq!(ctx.workspace_package_id("@myorg/utils/sub"), Some(7));
    assert_eq!(ctx.workspace_package_id("@myorg/utils/sub/mod"), Some(7));
    assert_eq!(ctx.workspace_package_id("@myorg/other"), None);
    assert_eq!(ctx.workspace_package_id("react"), None);
}

// ---------------------------------------------------------------------------
// PR 9: Type-alias expansion in chain walking
// ---------------------------------------------------------------------------

/// `type UserMap = Map<string, User>; class M { users: UserMap; do() { this.users.get(k) } }`.
/// The chain walker hits `current_type = "UserMap"` after Phase 2's field
/// lookup. Without alias expansion, Phase 3's `Map.get` lookup would fail
/// because `UserMap.get` is not indexed. With expansion, `current_type` is
/// rewritten to `Map` before the leaf lookup and `Map.get` resolves.
#[test]
fn alias_expansion_dereferences_type_alias_through_chain() {
    // Synthetic Map: signature carries the generic params so the engine
    // populates generic_params(["K", "V"]) for alias-arg substitution.
    let map_iface = ExtractedSymbol {
        name: "Map".to_string(),
        qualified_name: "Map".to_string(),
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("interface Map<K, V>".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let map_get = ExtractedSymbol {
        name: "get".to_string(),
        qualified_name: "Map.get".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("get(key: K): V | undefined".to_string()),
        doc_comment: None,
        scope_path: Some("Map".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let synth_file = ParsedFile {
        path: "ext:ts:lib/__bw_synthetic__.d.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: "synthetic".to_string(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![map_iface, map_get],
        refs: vec![],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None, None],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![false, false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    // Consumer file: User class, UserMap alias, UserManager class with the chain ref.
    let user_sym = make_symbol("User", "User", SymbolKind::Class, Visibility::Public, None);
    let user_map_alias = make_symbol(
        "UserMap",
        "UserMap",
        SymbolKind::TypeAlias,
        Visibility::Public,
        None,
    );
    let user_manager = make_symbol(
        "UserManager",
        "UserManager",
        SymbolKind::Class,
        Visibility::Public,
        None,
    );
    let users_field = ExtractedSymbol {
        name: "users".to_string(),
        qualified_name: "UserManager.users".to_string(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("users: UserMap".to_string()),
        doc_comment: None,
        scope_path: Some("UserManager".to_string()),
        parent_index: Some(2),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let do_method = ExtractedSymbol {
        name: "do".to_string(),
        qualified_name: "UserManager.do".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("do(): void".to_string()),
        doc_comment: None,
        scope_path: Some("UserManager".to_string()),
        parent_index: Some(2),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    // TypeRef from UserManager.users → "UserMap" — the engine reads this
    // into field_type["UserManager.users"] = "UserMap".
    let users_typeref = ExtractedRef {
        source_symbol_index: 3, // users_field
        target_name: "UserMap".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    // The chain ref: this.users.get(k) emitted from `do`.
    let chain_ref = ExtractedRef {
        source_symbol_index: 4, // do_method
        target_name: "get".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "this".to_string(),
                    node_kind: "this".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "users".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "get".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    // Consumer ParsedFile, including the explicit alias_targets payload that
    // the TS extractor would normally produce from `type UserMap = Map<string, User>`.
    let consumer_file = ParsedFile {
        path: "src/manager.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![
            user_sym,
            user_map_alias,
            user_manager,
            users_field,
            do_method,
        ],
        refs: vec![users_typeref, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None, None, None, None, None],
        ref_origin_languages: vec![None, None],
        symbol_from_snippet: vec![false, false, false, false, false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: vec![(
            "UserMap".to_string(),
            AliasTarget::Application {
                root: "Map".to_string(),
                args: vec!["string".to_string(), "User".to_string()],
            },
        )],
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&synth_file, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[1], // chain_ref
        source_symbol: &consumer_file.symbols[4], // do_method
        scope_chain: build_scope_chain(consumer_file.symbols[4].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "this.users.get(k) must resolve via UserMap → Map alias expansion"
    );
    let map_get_id = id_map
        .get(&(
            "ext:ts:lib/__bw_synthetic__.d.ts".to_string(),
            "Map.get".to_string(),
        ))
        .expect("Map.get must be indexed");
    assert_eq!(
        result.unwrap().target_symbol_id,
        *map_get_id,
        "chain must resolve to Map.get, not UserMap.get"
    );
}

/// `type Numbers = number[]; class C { ns: Numbers; do() { this.ns.map(f) } }`.
/// `array_type` aliases are stored as `Application{root: "Array", args: [elem]}`,
/// so `Numbers` expands to `Array` and `Array.map` resolves through the same
/// alias path as the explicit-generic form.
#[test]
fn alias_expansion_handles_array_type_form() {
    let array_iface = ExtractedSymbol {
        name: "Array".to_string(),
        qualified_name: "Array".to_string(),
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("interface Array<T>".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let array_map = ExtractedSymbol {
        name: "map".to_string(),
        qualified_name: "Array.map".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("map<U>(fn: (x: T) => U): Array<U>".to_string()),
        doc_comment: None,
        scope_path: Some("Array".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let synth_file = ParsedFile {
        path: "ext:ts:lib/__bw_synthetic_arr__.d.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: "synthetic".to_string(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![array_iface, array_map],
        refs: vec![],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None, None],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![false, false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let numbers_alias = make_symbol(
        "Numbers",
        "Numbers",
        SymbolKind::TypeAlias,
        Visibility::Public,
        None,
    );
    let c_class = make_symbol("C", "C", SymbolKind::Class, Visibility::Public, None);
    let ns_field = ExtractedSymbol {
        name: "ns".to_string(),
        qualified_name: "C.ns".to_string(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("ns: Numbers".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(1),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let do_method = ExtractedSymbol {
        name: "do".to_string(),
        qualified_name: "C.do".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("do(): void".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(1),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    let ns_typeref = ExtractedRef {
        source_symbol_index: 2, // ns_field
        target_name: "Numbers".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let chain_ref = ExtractedRef {
        source_symbol_index: 3, // do_method
        target_name: "map".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "this".to_string(),
                    node_kind: "this".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "ns".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "map".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let consumer_file = ParsedFile {
        path: "src/arr.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![numbers_alias, c_class, ns_field, do_method],
        refs: vec![ns_typeref, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None, None, None, None],
        ref_origin_languages: vec![None, None],
        symbol_from_snippet: vec![false, false, false, false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: vec![(
            "Numbers".to_string(),
            AliasTarget::Application {
                root: "Array".to_string(),
                args: vec!["number".to_string()],
            },
        )],
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&synth_file, &consumer_file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&consumer_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &consumer_file.refs[1],
        source_symbol: &consumer_file.symbols[3],
        scope_chain: build_scope_chain(consumer_file.symbols[3].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "this.ns.map(f) must resolve via Numbers (array_type alias) → Array"
    );
    let array_map_id = id_map
        .get(&(
            "ext:ts:lib/__bw_synthetic_arr__.d.ts".to_string(),
            "Array.map".to_string(),
        ))
        .expect("Array.map must be indexed");
    assert_eq!(result.unwrap().target_symbol_id, *array_map_id);
}

/// `type Status = "open" | "closed"; class C { s: Status; do() { this.s.foo() } }`.
/// Union aliases are NOT expanded — chain must miss (Phase-3 records a
/// chain miss and returns None) rather than incorrectly walking into the
/// first branch's members.
#[test]
fn alias_expansion_refuses_union_aliases() {
    let status_alias = make_symbol(
        "Status",
        "Status",
        SymbolKind::TypeAlias,
        Visibility::Public,
        None,
    );
    let c_class = make_symbol("C", "C", SymbolKind::Class, Visibility::Public, None);
    let s_field = ExtractedSymbol {
        name: "s".to_string(),
        qualified_name: "C.s".to_string(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("s: Status".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(1),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let do_method = ExtractedSymbol {
        name: "do".to_string(),
        qualified_name: "C.do".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("do(): void".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(1),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    let s_typeref = ExtractedRef {
        source_symbol_index: 2,
        target_name: "Status".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let chain_ref = ExtractedRef {
        source_symbol_index: 3,
        target_name: "foo".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "this".to_string(),
                    node_kind: "this".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "s".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "foo".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let file = ParsedFile {
        path: "src/u.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![status_alias, c_class, s_field, do_method],
        refs: vec![s_typeref, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None, None, None, None],
        ref_origin_languages: vec![None, None],
        symbol_from_snippet: vec![false, false, false, false],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: vec![(
            "Status".to_string(),
            AliasTarget::Union(vec!["\"open\"".to_string(), "\"closed\"".to_string()]),
        )],
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, _) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[1],
        source_symbol: &file.symbols[3],
        scope_chain: build_scope_chain(file.symbols[3].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_none(),
        "Union aliases must NOT expand — chain must miss, not pick a branch"
    );
}

// ---------------------------------------------------------------------------
// PR 10: typeof
// ---------------------------------------------------------------------------

/// `const api: User; type ApiType = typeof api; class C { a: ApiType; do() { this.a.greet() } }`.
/// `ApiType` is a `Typeof("api")` alias. The walker should dereference
/// it to `api`'s field_type ("User") and then resolve `User.greet`.
#[test]
fn typeof_alias_dereferences_to_value_type() {
    let user_class = make_symbol("User", "User", SymbolKind::Class, Visibility::Public, None);
    let user_greet = ExtractedSymbol {
        name: "greet".to_string(),
        qualified_name: "User.greet".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("greet(): void".to_string()),
        doc_comment: None,
        scope_path: Some("User".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    // The value `api: User`. Variable kind so the engine reads the
    // first TypeRef into `field_type["api"] = "User"`.
    let api_value = ExtractedSymbol {
        name: "api".to_string(),
        qualified_name: "api".to_string(),
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("const api: User".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let api_typeref = ExtractedRef {
        source_symbol_index: 2, // api_value
        target_name: "User".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let api_type_alias = make_symbol(
        "ApiType",
        "ApiType",
        SymbolKind::TypeAlias,
        Visibility::Public,
        None,
    );
    let c_class = make_symbol("C", "C", SymbolKind::Class, Visibility::Public, None);
    let a_field = ExtractedSymbol {
        name: "a".to_string(),
        qualified_name: "C.a".to_string(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("a: ApiType".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(4),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let do_method = ExtractedSymbol {
        name: "do".to_string(),
        qualified_name: "C.do".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("do(): void".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(4),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    let a_typeref = ExtractedRef {
        source_symbol_index: 5, // a_field
        target_name: "ApiType".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let chain_ref = ExtractedRef {
        source_symbol_index: 6, // do_method
        target_name: "greet".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "this".to_string(),
                    node_kind: "this".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "a".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "greet".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let file = ParsedFile {
        path: "src/typeof.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![
            user_class,
            user_greet,
            api_value,
            api_type_alias,
            c_class,
            a_field,
            do_method,
        ],
        refs: vec![api_typeref, a_typeref, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None; 7],
        ref_origin_languages: vec![None; 3],
        symbol_from_snippet: vec![false; 7],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: vec![(
            "ApiType".to_string(),
            AliasTarget::Typeof("api".to_string()),
        )],
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[2],
        source_symbol: &file.symbols[6],
        scope_chain: build_scope_chain(file.symbols[6].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "this.a.greet() must resolve via ApiType (typeof api) → User"
    );
    let user_greet_id = id_map
        .get(&("src/typeof.ts".to_string(), "User.greet".to_string()))
        .expect("User.greet must be indexed");
    assert_eq!(result.unwrap().target_symbol_id, *user_greet_id);
}

// ---------------------------------------------------------------------------
// PR 15: transparent mapped type expansion
// ---------------------------------------------------------------------------

/// `interface User { name: string; greet(): void }`
/// `type Partial<T> = { [K in keyof T]?: T[K] }`
/// `class C { p: Partial<User>; do() { this.p.greet() } }`
/// — `Partial<User>` is a transparent mapped type; member access
/// should fall through to `User`.
#[test]
fn transparent_mapped_partial_resolves_through_source() {
    let user_class = ExtractedSymbol {
        name: "User".to_string(),
        qualified_name: "User".to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("class User".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let user_greet = ExtractedSymbol {
        name: "greet".to_string(),
        qualified_name: "User.greet".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("greet(): void".to_string()),
        doc_comment: None,
        scope_path: Some("User".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    // Partial<T> alias — generic param T captured via signature.
    let partial_alias = ExtractedSymbol {
        name: "Partial".to_string(),
        qualified_name: "Partial".to_string(),
        kind: SymbolKind::TypeAlias,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("type Partial<T>".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let c_class = make_symbol("C", "C", SymbolKind::Class, Visibility::Public, None);
    // `p: Partial<User>` — engine sees TypeRef(Partial) followed by
    // TypeRef(User), reads field_type[C.p] = "Partial",
    // field_type_args[C.p] = ["User"].
    let p_field = ExtractedSymbol {
        name: "p".to_string(),
        qualified_name: "C.p".to_string(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("p: Partial<User>".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(3),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let do_method = ExtractedSymbol {
        name: "do".to_string(),
        qualified_name: "C.do".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("do(): void".to_string()),
        doc_comment: None,
        scope_path: Some("C".to_string()),
        parent_index: Some(3),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    let p_typeref_partial = ExtractedRef {
        source_symbol_index: 4,
        target_name: "Partial".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let p_typeref_user = ExtractedRef {
        source_symbol_index: 4,
        target_name: "User".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let chain_ref = ExtractedRef {
        source_symbol_index: 5,
        target_name: "greet".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "this".to_string(),
                    node_kind: "this".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "p".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "greet".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let file = ParsedFile {
        path: "src/mapped.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![
            user_class,
            user_greet,
            partial_alias,
            c_class,
            p_field,
            do_method,
        ],
        refs: vec![p_typeref_partial, p_typeref_user, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None; 6],
        ref_origin_languages: vec![None; 3],
        symbol_from_snippet: vec![false; 6],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: vec![(
            "Partial".to_string(),
            AliasTarget::Mapped {
                source: "T".to_string(),
                value_template: "T[K]".to_string(),
            },
        )],
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[2],
        source_symbol: &file.symbols[5],
        scope_chain: build_scope_chain(file.symbols[5].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "this.p.greet() on Partial<User> must collapse through to User.greet"
    );
    let user_greet_id = id_map
        .get(&("src/mapped.ts".to_string(), "User.greet".to_string()))
        .expect("User.greet must be indexed");
    assert_eq!(result.unwrap().target_symbol_id, *user_greet_id);
}

// ---------------------------------------------------------------------------
// PR 19: Phase 2 inheritance walking — intermediate field/method lookups
// climb the parent_class_qname chain so `this.injectedField.method()`
// resolves when injectedField is declared on a base class.
// ---------------------------------------------------------------------------

#[test]
fn phase2_inheritance_resolves_inherited_field() {
    let repo = make_symbol("Repo", "Repo", SymbolKind::Class, Visibility::Public, None);
    let repo_find = ExtractedSymbol {
        name: "find".to_string(),
        qualified_name: "Repo.find".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("find(): User".to_string()),
        doc_comment: None,
        scope_path: Some("Repo".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let base = make_symbol("Base", "Base", SymbolKind::Class, Visibility::Public, None);
    let base_db = ExtractedSymbol {
        name: "db".to_string(),
        qualified_name: "Base.db".to_string(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("db: Repo".to_string()),
        doc_comment: None,
        scope_path: Some("Base".to_string()),
        parent_index: Some(2),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let child = make_symbol("Child", "Child", SymbolKind::Class, Visibility::Public, None);
    let do_method = ExtractedSymbol {
        name: "do".to_string(),
        qualified_name: "Child.do".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("do(): void".to_string()),
        doc_comment: None,
        scope_path: Some("Child".to_string()),
        parent_index: Some(4),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    let base_db_typeref = ExtractedRef {
        source_symbol_index: 3,
        target_name: "Repo".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let child_inherits_base = ExtractedRef {
        source_symbol_index: 4,
        target_name: "Base".to_string(),
        kind: EdgeKind::Inherits,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let chain_ref = ExtractedRef {
        source_symbol_index: 5,
        target_name: "find".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "this".to_string(),
                    node_kind: "this".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "db".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "find".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let file = ParsedFile {
        path: "src/inherit.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![repo, repo_find, base, base_db, child, do_method],
        refs: vec![base_db_typeref, child_inherits_base, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None; 6],
        ref_origin_languages: vec![None; 3],
        symbol_from_snippet: vec![false; 6],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[2],
        source_symbol: &file.symbols[5],
        scope_chain: build_scope_chain(file.symbols[5].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "this.db.find() must climb Child → Base inheritance to find the inherited db field"
    );
    let repo_find_id = id_map
        .get(&("src/inherit.ts".to_string(), "Repo.find".to_string()))
        .expect("Repo.find must be indexed");
    assert_eq!(result.unwrap().target_symbol_id, *repo_find_id);
}

#[test]
fn phase2_inheritance_resolves_through_two_hops() {
    let svc = make_symbol("Svc", "Svc", SymbolKind::Class, Visibility::Public, None);
    let svc_run = ExtractedSymbol {
        name: "run".to_string(),
        qualified_name: "Svc.run".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("run(): void".to_string()),
        doc_comment: None,
        scope_path: Some("Svc".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let grand = make_symbol("Grand", "Grand", SymbolKind::Class, Visibility::Public, None);
    let grand_svc = ExtractedSymbol {
        name: "svc".to_string(),
        qualified_name: "Grand.svc".to_string(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("svc: Svc".to_string()),
        doc_comment: None,
        scope_path: Some("Grand".to_string()),
        parent_index: Some(2),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let mid = make_symbol("Mid", "Mid", SymbolKind::Class, Visibility::Public, None);
    let leaf = make_symbol("Leaf", "Leaf", SymbolKind::Class, Visibility::Public, None);
    let do_method = ExtractedSymbol {
        name: "do".to_string(),
        qualified_name: "Leaf.do".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("do(): void".to_string()),
        doc_comment: None,
        scope_path: Some("Leaf".to_string()),
        parent_index: Some(5),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    let grand_svc_typeref = ExtractedRef {
        source_symbol_index: 3,
        target_name: "Svc".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let mid_inherits = ExtractedRef {
        source_symbol_index: 4,
        target_name: "Grand".to_string(),
        kind: EdgeKind::Inherits,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let leaf_inherits = ExtractedRef {
        source_symbol_index: 5,
        target_name: "Mid".to_string(),
        kind: EdgeKind::Inherits,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let chain_ref = ExtractedRef {
        source_symbol_index: 6,
        target_name: "run".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "this".to_string(),
                    node_kind: "this".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "svc".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "run".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let file = ParsedFile {
        path: "src/two_hops.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![svc, svc_run, grand, grand_svc, mid, leaf, do_method],
        refs: vec![grand_svc_typeref, mid_inherits, leaf_inherits, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None; 7],
        ref_origin_languages: vec![None; 4],
        symbol_from_snippet: vec![false; 7],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[3],
        source_symbol: &file.symbols[6],
        scope_chain: build_scope_chain(file.symbols[6].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "this.svc.run() must climb Leaf → Mid → Grand to find inherited field"
    );
    let svc_run_id = id_map
        .get(&("src/two_hops.ts".to_string(), "Svc.run".to_string()))
        .expect("Svc.run must be indexed");
    assert_eq!(result.unwrap().target_symbol_id, *svc_run_id);
}

// ---------------------------------------------------------------------------
// PR 24: `: this` polymorphic-self return — fluent-API chains keep their
// receiver type. NestJS DocumentBuilder, query builders, etc.
// ---------------------------------------------------------------------------

#[test]
fn this_return_keeps_receiver_through_fluent_chain() {
    // Mirror DocumentBuilder shape: two methods that both return `this`.
    // Chain: `new Builder().setA().setB()` — without the `: this` hop,
    // current_type advances to literal "this" after setA() and setB
    // can't be found on a class named "this".
    let builder = make_symbol("Builder", "Builder", SymbolKind::Class, Visibility::Public, None);
    let set_a = ExtractedSymbol {
        name: "setA".to_string(),
        qualified_name: "Builder.setA".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("setA(x: string): this".to_string()),
        doc_comment: None,
        scope_path: Some("Builder".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let set_b = ExtractedSymbol {
        name: "setB".to_string(),
        qualified_name: "Builder.setB".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("setB(y: number): this".to_string()),
        doc_comment: None,
        scope_path: Some("Builder".to_string()),
        parent_index: Some(0),
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let caller = ExtractedSymbol {
        name: "build".to_string(),
        qualified_name: "build".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some("build(): void".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};

    // Each method's signature carries the `: this` return — the type
    // checker's signature parser populates return_type from this.
    let chain_ref = ExtractedRef {
        source_symbol_index: 3,
        target_name: "setB".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                // The chain builder peels `new Builder()` into the bare
                // identifier `Builder`, so the root segment is Identifier
                // (not Construction) — same shape produced for plain class
                // references like `Builder.staticMethod()`.
                ChainSegment {
                    name: "Builder".to_string(),
                    node_kind: "identifier".to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "setA".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
                ChainSegment {
                    name: "setB".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                                    byte_offset: 0,
    declared_type_id: None,
    type_arg_ids: Vec::new(),
},
            ],
        }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };

    let file = ParsedFile {
        path: "src/fluent.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![builder, set_a, set_b, caller],
        refs: vec![chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![None; 4],
        ref_origin_languages: vec![None; 1],
        symbol_from_snippet: vec![false; 4],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    };

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = TypeScriptResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[3],
        scope_chain: build_scope_chain(file.symbols[3].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "new Builder().setA().setB() must keep the Builder binding through `: this` returns"
    );
    let set_b_id = id_map
        .get(&("src/fluent.ts".to_string(), "Builder.setB".to_string()))
        .expect("Builder.setB must be indexed");
    assert_eq!(result.unwrap().target_symbol_id, *set_b_id);
}

// ---------------------------------------------------------------------------
// detect_chain_flow_emission tests
// ---------------------------------------------------------------------------

fn make_chain_segs(segments: &[(&str, crate::types::SegmentKind)]) -> crate::types::MemberChain {
    crate::types::MemberChain {
        segments: segments
            .iter()
            .map(|(name, kind)| crate::types::ChainSegment {
                name: name.to_string(),
                node_kind: String::new(),
                kind: *kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
})
            .collect(),
    }
}

fn make_ctx_with_import(
    import_name: &str,
    from_module: &str,
) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/app.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: import_name.to_string(),
            module_path: Some(from_module.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

#[test]
fn http_call_axios_get_emits_with_method() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("axios", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let file_ctx = make_ctx_with_import("axios", "axios");
    let args = vec![crate::types::CallArg::StringLit("/api/users".to_string())];
    let result = detect_chain_flow_emission(&chain, &args, &file_ctx);
    assert!(result.is_some(), "axios.get should emit a flow edge");
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, method, role, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(role, ChannelRole::Producer);
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn http_call_axios_post_emits_post_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("axios", SegmentKind::Identifier),
        ("post", SegmentKind::Property),
    ]);
    let file_ctx = make_ctx_with_import("axios", "axios");
    let args = vec![crate::types::CallArg::StringLit("/api/users".to_string())];
    let result = detect_chain_flow_emission(&chain, &args, &file_ctx);
    assert!(result.is_some());
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(method, Some(HttpMethod::Post));
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn http_call_nestjs_axios_http_service_recognised() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    // `import { HttpService } from '@nestjs/axios'; this.httpService.get(...)` —
    // the imported root is `HttpService` from `@nestjs/axios`. The detector
    // keys on the import source package; the chain shape is the same as axios.
    let chain = make_chain_segs(&[
        ("HttpService", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let file_ctx = make_ctx_with_import("HttpService", "@nestjs/axios");
    let args = vec![crate::types::CallArg::StringLit("/api/users".to_string())];
    let result = detect_chain_flow_emission(&chain, &args, &file_ctx);
    assert!(result.is_some(), "@nestjs/axios HttpService.get should emit");
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn http_call_openapi_typescript_fetch_recognised() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("Fetcher", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let file_ctx = make_ctx_with_import("Fetcher", "openapi-typescript-fetch");
    let args = vec![crate::types::CallArg::StringLit("/api/users".to_string())];
    let result = detect_chain_flow_emission(&chain, &args, &file_ctx);
    assert!(result.is_some());
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn http_call_global_fetch_emits_without_import() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[("fetch", SegmentKind::Identifier)]);
    let file_ctx = crate::indexer::resolve::engine::FileContext {
        file_path: "src/app.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let args = vec![crate::types::CallArg::StringLit("/api/users".to_string())];
    let result = detect_chain_flow_emission(&chain, &args, &file_ctx);
    assert!(result.is_some(), "global fetch with URL should emit without import");
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn websocket_emit_producer_on_emit() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("socket", SegmentKind::Identifier),
        ("emit", SegmentKind::Property),
    ]);
    let file_ctx = make_ctx_with_import("socket", "socket.io-client");
    let result = detect_chain_flow_emission(&chain, &[], &file_ctx);
    assert!(result.is_some());
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, .. } => {
            assert_eq!(kind, NamedChannelKind::WebSocket);
            assert_eq!(role, ChannelRole::Producer);
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn websocket_on_handler_emits_consumer_role() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("socket", SegmentKind::Identifier),
        ("on", SegmentKind::Property),
    ]);
    let file_ctx = make_ctx_with_import("socket", "socket.io-client");
    let result = detect_chain_flow_emission(&chain, &[], &file_ctx);
    assert!(result.is_some());
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, .. } => {
            assert_eq!(kind, NamedChannelKind::WebSocket);
            assert_eq!(role, ChannelRole::Consumer);
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn ipc_call_tauri_invoke_emits_ipc_call() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[("invoke", SegmentKind::Identifier)]);
    let file_ctx = make_ctx_with_import("invoke", "@tauri-apps/api/tauri");
    let result = detect_chain_flow_emission(&chain, &[], &file_ctx);
    assert!(result.is_some());
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, .. } => {
            assert_eq!(kind, NamedChannelKind::IpcCall);
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn non_http_package_does_not_emit() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("_", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let file_ctx = make_ctx_with_import("_", "lodash");
    let result = detect_chain_flow_emission(&chain, &[], &file_ctx);
    assert!(result.is_none(), "lodash.get should NOT emit a flow edge");
}

// ---------------------------------------------------------------------------
// parse_gql_operation
// ---------------------------------------------------------------------------

#[test]
fn gql_named_query_parses_operation_name() {
    use super::hooks::parse_gql_operation;
    let body = "\n  query GetUsers {\n    users { id name }\n  }\n";
    let result = parse_gql_operation(body);
    assert_eq!(result, Some("query:GetUsers".to_string()));
}

#[test]
fn gql_named_mutation_parses_operation_name() {
    use super::hooks::parse_gql_operation;
    let body = "mutation CreatePost($input: PostInput!) { createPost(input: $input) { id } }";
    let result = parse_gql_operation(body);
    assert_eq!(result, Some("mutation:CreatePost".to_string()));
}

#[test]
fn gql_named_subscription_parses_operation_name() {
    use super::hooks::parse_gql_operation;
    let body = "subscription OnMessageAdded { messageAdded { id body } }";
    let result = parse_gql_operation(body);
    assert_eq!(result, Some("subscription:OnMessageAdded".to_string()));
}

#[test]
fn gql_anonymous_query_returns_anon_sentinel() {
    use super::hooks::parse_gql_operation;
    let body = "query { users { id } }";
    let result = parse_gql_operation(body);
    assert_eq!(result, Some("query:__anon__".to_string()));
}

#[test]
fn gql_underscore_prefixed_name_parses() {
    use super::hooks::parse_gql_operation;
    let body = "query _InternalFetch { nodes { id } }";
    let result = parse_gql_operation(body);
    assert_eq!(result, Some("query:_InternalFetch".to_string()));
}

#[test]
fn gql_empty_body_returns_none() {
    use super::hooks::parse_gql_operation;
    let result = parse_gql_operation("");
    assert_eq!(result, None);
}

#[test]
fn gql_non_graphql_body_returns_none() {
    use super::hooks::parse_gql_operation;
    let result = parse_gql_operation("SELECT * FROM users");
    assert_eq!(result, None);
}

// ---------------------------------------------------------------------------
// detect_chain_flow_emission — URL captured from call_args
// ---------------------------------------------------------------------------

#[test]
fn http_call_axios_captures_url_from_string_arg() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("axios", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("/api/users".to_string())];
    let file_ctx = make_ctx_with_import("axios", "axios");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    assert!(result.is_some());
    match result.unwrap() {
        FlowEmission::NamedChannel { name, .. } => {
            assert_eq!(name, "/api/users");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn http_call_global_fetch_captures_url() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[("fetch", SegmentKind::Identifier)]);
    let call_args = vec![CallArg::StringLit("/graphql".to_string())];
    let file_ctx = crate::indexer::resolve::engine::FileContext {
        file_path: "src/api.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    match result.unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/graphql"),
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn ipc_call_tauri_invoke_captures_command_name() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("invoke", SegmentKind::Identifier),
    ]);
    let call_args = vec![CallArg::StringLit("get_config".to_string())];
    let file_ctx = make_ctx_with_import("invoke", "@tauri-apps/api/tauri");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    assert!(result.is_some());
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert_eq!(kind, NamedChannelKind::IpcCall);
            assert_eq!(name, "get_config");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_chain_flow_emission — GQL tagged template
// ---------------------------------------------------------------------------

#[test]
fn gql_tagged_template_emits_graphql_op() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[("gql", SegmentKind::Identifier)]);
    let body = "query GetPosts { posts { id title } }".to_string();
    let call_args = vec![CallArg::TaggedTemplate {
        tag: "gql".to_string(),
        body,
    }];
    let file_ctx = make_ctx_with_import("gql", "@apollo/client");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    assert!(result.is_some(), "gql tagged template should emit graphql_op");
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert_eq!(kind, NamedChannelKind::GraphQLOp);
            assert_eq!(name, "query:GetPosts");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_chain_flow_emission — migration calls
// ---------------------------------------------------------------------------

#[test]
fn knex_schema_create_table_emits_migration_target() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("knex", SegmentKind::Identifier),
        ("schema", SegmentKind::Property),
        ("createTable", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("users".to_string())];
    let file_ctx = make_ctx_with_import("knex", "knex");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    assert!(result.is_some(), "knex.schema.createTable should emit MigrationTarget");
    match result.unwrap() {
        FlowEmission::MigrationTarget { table_name, .. } => {
            assert_eq!(table_name, "users");
        }
        other => panic!("Expected MigrationTarget, got {other:?}"),
    }
}

#[test]
fn sequelize_query_interface_create_table_emits_migration_target() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("queryInterface", SegmentKind::Identifier),
        ("createTable", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("orders".to_string())];
    // Sequelize queryInterface is recognized by root name — no import check.
    let file_ctx = make_ctx_with_import("queryInterface", "sequelize");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    assert!(result.is_some(), "queryInterface.createTable should emit MigrationTarget");
    match result.unwrap() {
        FlowEmission::MigrationTarget { table_name, .. } => {
            assert_eq!(table_name, "orders");
        }
        other => panic!("Expected MigrationTarget, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_chain_flow_emission — scheduled job
// ---------------------------------------------------------------------------

#[test]
fn node_cron_schedule_emits_scheduled_job() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("cron", SegmentKind::Identifier),
        ("schedule", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("0 * * * *".to_string())];
    let file_ctx = make_ctx_with_import("cron", "node-cron");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    assert!(result.is_some(), "cron.schedule should emit ScheduledJob");
    match result.unwrap() {
        FlowEmission::ScheduledJob { schedule } => {
            assert_eq!(schedule, "0 * * * *");
        }
        other => panic!("Expected ScheduledJob, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_chain_flow_emission — CLI command
// ---------------------------------------------------------------------------

#[test]
fn commander_command_emits_cli_command() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("program", SegmentKind::Identifier),
        ("command", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("build".to_string())];
    let file_ctx = make_ctx_with_import("program", "commander");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    assert!(result.is_some(), "program.command should emit CliCommand");
    match result.unwrap() {
        FlowEmission::CliCommand { command_name, framework } => {
            assert_eq!(command_name, "build");
            assert_eq!(framework.as_deref(), Some("commander"));
        }
        other => panic!("Expected CliCommand, got {other:?}"),
    }
}

#[test]
fn yargs_command_emits_cli_command_yargs_framework() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("yargs", SegmentKind::Identifier),
        ("command", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("deploy".to_string())];
    let file_ctx = make_ctx_with_import("yargs", "yargs");
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    match result.unwrap() {
        FlowEmission::CliCommand { command_name, framework } => {
            assert_eq!(command_name, "deploy");
            assert_eq!(framework.as_deref(), Some("yargs"));
        }
        other => panic!("Expected CliCommand, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_decorator_flow_emission — direct function tests
// ---------------------------------------------------------------------------

#[test]
fn decorator_entity_with_table_name_emits_db_entity() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Entity", Some("users"), None);
    assert!(result.is_some(), "Entity decorator should emit DbEntity");
    match result.unwrap() {
        FlowEmission::DbEntity { table_name_hint, base_name_hint, .. } => {
            assert_eq!(table_name_hint.as_deref(), Some("users"));
            assert_eq!(base_name_hint, "Entity");
        }
        other => panic!("Expected DbEntity, got {other:?}"),
    }
}

#[test]
fn decorator_entity_without_table_name_emits_db_entity_no_hint() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Entity", None, None);
    match result.unwrap() {
        FlowEmission::DbEntity { table_name_hint, .. } => {
            assert!(table_name_hint.is_none());
        }
        other => panic!("Expected DbEntity, got {other:?}"),
    }
}

#[test]
fn decorator_table_emits_db_entity_with_model_base() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Table", Some("products"), None);
    match result.unwrap() {
        FlowEmission::DbEntity { base_name_hint, table_name_hint, .. } => {
            assert_eq!(base_name_hint, "Model");
            assert_eq!(table_name_hint.as_deref(), Some("products"));
        }
        other => panic!("Expected DbEntity, got {other:?}"),
    }
}

#[test]
fn decorator_schema_emits_db_entity() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Schema", Some("post"), None);
    match result.unwrap() {
        FlowEmission::DbEntity { base_name_hint, .. } => {
            assert_eq!(base_name_hint, "Schema");
        }
        other => panic!("Expected DbEntity, got {other:?}"),
    }
}

#[test]
fn decorator_roles_emits_auth_guard_role_kind() {
    use crate::indexer::resolve::flow_emit::{AuthGuardKind, FlowEmission};
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Roles", Some("admin"), None);
    assert!(result.is_some(), "Roles decorator should emit AuthGuard");
    match result.unwrap() {
        FlowEmission::AuthGuard { requirement, kind } => {
            assert_eq!(requirement, "admin");
            assert_eq!(kind, AuthGuardKind::Role);
        }
        other => panic!("Expected AuthGuard, got {other:?}"),
    }
}

#[test]
fn decorator_use_guards_emits_auth_guard_custom_kind() {
    use crate::indexer::resolve::flow_emit::{AuthGuardKind, FlowEmission};
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("UseGuards", Some("JwtAuthGuard"), None);
    match result.unwrap() {
        FlowEmission::AuthGuard { requirement, kind } => {
            assert_eq!(requirement, "JwtAuthGuard");
            assert_eq!(kind, AuthGuardKind::Custom);
        }
        other => panic!("Expected AuthGuard, got {other:?}"),
    }
}

#[test]
fn decorator_permissions_emits_auth_guard_permission_kind() {
    use crate::indexer::resolve::flow_emit::{AuthGuardKind, FlowEmission};
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Permissions", Some("read:users"), None);
    match result.unwrap() {
        FlowEmission::AuthGuard { kind, .. } => assert_eq!(kind, AuthGuardKind::Permission),
        other => panic!("Expected AuthGuard, got {other:?}"),
    }
}

#[test]
fn decorator_jwt_auth_guard_emits_token_kind() {
    use crate::indexer::resolve::flow_emit::{AuthGuardKind, FlowEmission};
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("JwtAuthGuard", None, None);
    match result.unwrap() {
        FlowEmission::AuthGuard { requirement, kind } => {
            assert_eq!(kind, AuthGuardKind::Token);
            // When no first arg, falls back to the decorator name itself.
            assert_eq!(requirement, "JwtAuthGuard");
        }
        other => panic!("Expected AuthGuard, got {other:?}"),
    }
}

#[test]
fn decorator_policy_emits_auth_guard_policy_kind() {
    use crate::indexer::resolve::flow_emit::{AuthGuardKind, FlowEmission};
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Policy", Some("IsOwner"), None);
    match result.unwrap() {
        FlowEmission::AuthGuard { requirement, kind } => {
            assert_eq!(requirement, "IsOwner");
            assert_eq!(kind, AuthGuardKind::Policy);
        }
        other => panic!("Expected AuthGuard, got {other:?}"),
    }
}

#[test]
fn unknown_decorator_does_not_emit() {
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Injectable", None, None);
    assert!(result.is_none(), "Injectable is not a flow-relevant decorator");
}

#[test]
fn decorator_input_does_not_emit() {
    use super::hooks::detect_decorator_flow_emission;
    assert!(detect_decorator_flow_emission("Input", Some("name"), None).is_none());
}

#[test]
fn decorator_controller_does_not_emit() {
    use super::hooks::detect_decorator_flow_emission;
    assert!(detect_decorator_flow_emission("Controller", Some("/api"), None).is_none());
}

// ---------------------------------------------------------------------------
// detect_decorator_flow_emission — class-context fallback for DbEntity
// ---------------------------------------------------------------------------

#[test]
fn decorator_entity_without_arg_falls_back_to_class_context() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Entity", None, Some("User"));
    match result.unwrap() {
        FlowEmission::DbEntity { table_name_hint, base_name_hint, .. } => {
            assert_eq!(table_name_hint.as_deref(), Some("User"));
            assert_eq!(base_name_hint, "Entity");
        }
        other => panic!("Expected DbEntity, got {other:?}"),
    }
}

#[test]
fn decorator_schema_without_arg_falls_back_to_class_context() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Schema", None, Some("Post"));
    match result.unwrap() {
        FlowEmission::DbEntity { table_name_hint, .. } => {
            assert_eq!(table_name_hint.as_deref(), Some("Post"));
        }
        other => panic!("Expected DbEntity, got {other:?}"),
    }
}

#[test]
fn decorator_entity_explicit_arg_takes_precedence_over_class_context() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("Entity", Some("users"), Some("User"));
    match result.unwrap() {
        FlowEmission::DbEntity { table_name_hint, .. } => {
            assert_eq!(table_name_hint.as_deref(), Some("users"));
        }
        other => panic!("Expected DbEntity, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_db_query_emission — Prisma 3-segment chains
// ---------------------------------------------------------------------------

#[test]
fn test_db_query_prisma_find_unique() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("prisma", SegmentKind::Identifier),
        ("user", SegmentKind::Property),
        ("findUnique", SegmentKind::Property),
    ]);
    let result = detect_db_query_emission(&chain);
    assert!(result.is_some(), "prisma.user.findUnique should emit DbQuery");
    match result.unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_prisma_create_many_op_classified_as_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("db", SegmentKind::Identifier),
        ("post", SegmentKind::Property),
        ("createMany", SegmentKind::Property),
    ]);
    let result = detect_db_query_emission(&chain);
    match result.unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "Post");
            assert_eq!(operation, DbQueryOp::Insert);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_prisma_upsert_op() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("prisma", SegmentKind::Identifier),
        ("session", SegmentKind::Property),
        ("upsert", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "Session");
            assert_eq!(operation, DbQueryOp::Upsert);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_prisma_pascal_model_rejected() {
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    // PascalCase second segment is NOT a Prisma model accessor — Prisma
    // models hang off the client as camelCase properties.
    let chain = make_chain_segs(&[
        ("svc", SegmentKind::Identifier),
        ("User", SegmentKind::Property),
        ("findUnique", SegmentKind::Property),
    ]);
    let result = detect_db_query_emission(&chain);
    assert!(result.is_none(), "PascalCase second segment must not be treated as a Prisma model");
}

// ---------------------------------------------------------------------------
// detect_db_query_emission — TypeORM repositories
// ---------------------------------------------------------------------------

fn make_chain_with_typed_root(
    root_name: &str,
    root_type: &str,
    root_type_args: &[&str],
    leaf_name: &str,
) -> crate::types::MemberChain {
    use crate::types::{ChainSegment, MemberChain, SegmentKind};
    MemberChain {
        segments: vec![
            ChainSegment {
                name: root_name.to_string(),
                node_kind: String::new(),
                kind: SegmentKind::Identifier,
                declared_type: Some(root_type.to_string()),
                type_args: root_type_args.iter().map(|s| s.to_string()).collect(),
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
},
            ChainSegment {
                name: leaf_name.to_string(),
                node_kind: String::new(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
},
        ],
    }
}

#[test]
fn test_db_query_typeorm_repository_declared_type() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;

    let chain = make_chain_with_typed_root("userRepo", "Repository", &["User"], "findOne");
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_typeorm_tree_repository_declared_type() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;

    let chain = make_chain_with_typed_root("categoryTree", "TreeRepository", &["Category"], "save");
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "Category");
            assert_eq!(operation, DbQueryOp::Insert);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_typeorm_repository_name_suffix() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("userRepository", SegmentKind::Identifier),
        ("delete", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "User");
            assert_eq!(operation, DbQueryOp::Delete);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_typeorm_repo_short_suffix() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("postRepo", SegmentKind::Identifier),
        ("save", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, .. } => {
            assert_eq!(entity_name, "Post");
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_db_query_emission — Mongoose static methods
// ---------------------------------------------------------------------------

#[test]
fn test_db_query_mongoose_find_one() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("User", SegmentKind::Identifier),
        ("findOne", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_mongoose_find_by_id_and_update() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("Post", SegmentKind::Identifier),
        ("findByIdAndUpdate", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "Post");
            assert_eq!(operation, DbQueryOp::Update);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_mongoose_chain_with_populate_still_emits() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    // User.find().populate('author') — chain has trailing populate segments
    // but the underlying query is still on User.
    let chain = make_chain_segs(&[
        ("Comment", SegmentKind::Identifier),
        ("find", SegmentKind::Property),
        ("populate", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, .. } => {
            assert_eq!(entity_name, "Comment");
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_db_query_emission — Sequelize static methods
// ---------------------------------------------------------------------------

#[test]
fn test_db_query_sequelize_find_all() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("User", SegmentKind::Identifier),
        ("findAll", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_sequelize_find_by_pk() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("Product", SegmentKind::Identifier),
        ("findByPk", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "Product");
            assert_eq!(operation, DbQueryOp::Select);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_sequelize_bulk_create_classified_as_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("Order", SegmentKind::Identifier),
        ("bulkCreate", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => {
            assert_eq!(operation, DbQueryOp::Insert);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

#[test]
fn test_db_query_sequelize_destroy_classified_as_delete() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("Session", SegmentKind::Identifier),
        ("destroy", SegmentKind::Property),
    ]);
    match detect_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => {
            assert_eq!(operation, DbQueryOp::Delete);
        }
        other => panic!("Expected DbQuery, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// detect_db_query_emission — false-positive guards
// ---------------------------------------------------------------------------

#[test]
fn test_db_query_lowercase_root_rejected_for_mongoose_shape() {
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    // `users.find(...)` — root is lowercase, must NOT match the
    // Mongoose/Sequelize PascalCase model shape.
    let chain = make_chain_segs(&[
        ("users", SegmentKind::Identifier),
        ("find", SegmentKind::Property),
    ]);
    assert!(detect_db_query_emission(&chain).is_none());
}

#[test]
fn test_db_query_unknown_method_does_not_emit() {
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    // `User.greet(...)` — PascalCase root but `greet` isn't in any ORM
    // method set, so no emission.
    let chain = make_chain_segs(&[
        ("User", SegmentKind::Identifier),
        ("greet", SegmentKind::Property),
    ]);
    assert!(detect_db_query_emission(&chain).is_none());
}

#[test]
fn test_db_query_object_keys_does_not_emit() {
    use super::hooks::detect_db_query_emission;
    use crate::types::SegmentKind;

    // `Object.keys(...)` — `keys` isn't in any ORM method set.
    let chain = make_chain_segs(&[
        ("Object", SegmentKind::Identifier),
        ("keys", SegmentKind::Property),
    ]);
    assert!(detect_db_query_emission(&chain).is_none());
}

// ---------------------------------------------------------------------------
// detect_chain_flow_emission — DbQuery integration (fallthrough path)
// ---------------------------------------------------------------------------

#[test]
fn test_db_query_chain_emission_dispatch_prisma() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("prisma", SegmentKind::Identifier),
        ("user", SegmentKind::Property),
        ("findMany", SegmentKind::Property),
    ]);
    // No HTTP/IPC/etc. import — must fall through to DbQuery branch.
    let file_ctx = crate::indexer::resolve::engine::FileContext {
        file_path: "src/users.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    match detect_chain_flow_emission(&chain, &[], &file_ctx).unwrap() {
        FlowEmission::DbQuery { entity_name, .. } => assert_eq!(entity_name, "User"),
        other => panic!("Expected DbQuery via chain dispatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// NestJS HTTP route decorators — Consumer-role HttpCall
// ---------------------------------------------------------------------------

/// FileContext carrying a single synthetic `__ts_controller_prefix__:<qname>`
/// entry, mimicking what `build_file_context` populates during the
/// `@Controller(...)` pre-pass.
fn make_ctx_with_controller_prefix(
    class_qname: &str,
    prefix: &str,
) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/users.controller.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![
            // Controller-prefix lookup (synthetic key produced by the
            // class-decorator pre-pass).
            crate::indexer::resolve::engine::ImportEntry {
                imported_name: format!("__ts_controller_prefix__:{}", class_qname),
                module_path: Some(prefix.to_string()),
                alias: None,
                is_wildcard: false,
            },
            // Real @nestjs/common import — required for the route-decorator
            // detector to fire. Production controllers always have this.
            crate::indexer::resolve::engine::ImportEntry {
                imported_name: "Controller".to_string(),
                module_path: Some("@nestjs/common".to_string()),
                alias: None,
                is_wildcard: false,
            },
        ],
        file_namespace: None,
    }
}

#[test]
fn test_nestjs_http_consumer_get_emits_consumer_with_get_method() {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("UsersController", "users");
    let result = detect_route_decorator_flow_emission(
        "Get",
        Some(":id"),
        "UsersController.findOne",
        &file_ctx,
    );
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, method, name, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/users/{}");
        }
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_post_emits_post_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("UsersController", "users");
    let result = detect_route_decorator_flow_emission(
        "Post",
        None,
        "UsersController.create",
        &file_ctx,
    );
    match result.unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Post));
            assert_eq!(name, "/users");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_put_emits_put_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("AlbumsController", "/albums");
    match detect_route_decorator_flow_emission(
        "Put",
        Some(":id/assets"),
        "AlbumsController.addAssets",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Put));
            assert_eq!(name, "/albums/{}/assets");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_patch_emits_patch_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("UsersController", "users");
    match detect_route_decorator_flow_emission(
        "Patch",
        Some(":id"),
        "UsersController.update",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Patch));
            assert_eq!(name, "/users/{}");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_delete_emits_delete_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("UsersController", "users");
    match detect_route_decorator_flow_emission(
        "Delete",
        Some(":id"),
        "UsersController.remove",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Delete));
            assert_eq!(name, "/users/{}");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_head_emits_head_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("FilesController", "files");
    match detect_route_decorator_flow_emission(
        "Head",
        Some(":id"),
        "FilesController.head",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { method, .. } => {
            assert_eq!(method, Some(HttpMethod::Head));
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_options_emits_options_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("CorsController", "preflight");
    match detect_route_decorator_flow_emission(
        "Options",
        None,
        "CorsController.preflight",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Options));
            assert_eq!(name, "/preflight");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_all_emits_any_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_route_decorator_flow_emission;

    let file_ctx = make_ctx_with_controller_prefix("CatchAllController", "internal");
    match detect_route_decorator_flow_emission(
        "All",
        Some("ping"),
        "CatchAllController.ping",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Any));
            assert_eq!(name, "/internal/ping");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_joins_controller_prefix_with_method_path() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_route_decorator_flow_emission;

    // @Controller('/api/users') + @Get('/:id/details') → /api/users/{}/details
    let file_ctx = make_ctx_with_controller_prefix("UsersController", "/api/users");
    match detect_route_decorator_flow_emission(
        "Get",
        Some("/:id/details"),
        "UsersController.getDetails",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { name, .. } => {
            assert_eq!(name, "/api/users/{}/details");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_empty_prefix_falls_back_to_method_path() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_route_decorator_flow_emission;

    // `@Controller(RouteKey.X)` — the extractor cannot capture the enum
    // expression, so the prefix entry exists but its value is empty.
    let file_ctx = make_ctx_with_controller_prefix("MysteryController", "");
    match detect_route_decorator_flow_emission(
        "Get",
        Some("status"),
        "MysteryController.status",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { name, .. } => {
            assert_eq!(name, "/status");
        }
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_unknown_decorator_does_not_emit() {
    use super::hooks::detect_route_decorator_flow_emission;
    let file_ctx = make_ctx_with_controller_prefix("UsersController", "users");
    let r = detect_route_decorator_flow_emission(
        "Injectable",
        None,
        "UsersController",
        &file_ctx,
    );
    assert!(r.is_none());
}

#[test]
fn test_nestjs_http_consumer_no_controller_prefix_in_context() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_route_decorator_flow_emission;

    // No `__ts_controller_prefix__:` entry — the method's @Get is emitted
    // with just the path. The file still needs to import @nestjs/common so
    // the detector knows the @Get name is a NestJS routing decorator and not
    // an unrelated type import named "Get".
    let file_ctx = crate::indexer::resolve::engine::FileContext {
        file_path: "src/standalone.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: "Get".to_string(),
            module_path: Some("@nestjs/common".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    match detect_route_decorator_flow_emission(
        "Get",
        Some("/standalone"),
        "Stray.handle",
        &file_ctx,
    )
    .unwrap()
    {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/standalone"),
        other => panic!("Expected NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_nestjs_http_consumer_join_route_segments_helper() {
    use super::hooks::join_route_segments;
    assert_eq!(join_route_segments("/api/users", ":id"), "/api/users/:id");
    assert_eq!(join_route_segments("/api/users", "/:id"), "/api/users/:id");
    assert_eq!(join_route_segments("api/users", ""), "/api/users");
    assert_eq!(join_route_segments("", "/health"), "/health");
    assert_eq!(join_route_segments("", ""), "/");
    assert_eq!(join_route_segments("/api/users/", "/:id"), "/api/users/:id");
}

// ---------------------------------------------------------------------------
// Express / Hono / Fastify chain-route Consumer — HttpCall
// ---------------------------------------------------------------------------

fn make_ctx_with_framework_import(framework: &str) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/server.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: framework.to_string(),
            module_path: Some(framework.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

#[test]
fn test_chain_route_consumer_express_app_get() {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("express");
    let chain = make_chain_segs(&[
        ("app", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("/users/:id".to_string()),
        CallArg::Other,
    ];
    let result = detect_chain_flow_emission(&chain, &call_args, &file_ctx);
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, method, name, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/users/{}");
        }
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_chain_route_consumer_express_router_post() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("express");
    let chain = make_chain_segs(&[
        ("router", SegmentKind::Identifier),
        ("post", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("/login".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { role, method, name, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Post));
            assert_eq!(name, "/login");
        }
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_chain_route_consumer_hono_app_get_normalises_colon_param() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("hono");
    let chain = make_chain_segs(&[
        ("app", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("/:eventId/google-calendar".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { role, method, name, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/{}/google-calendar");
        }
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_chain_route_consumer_hono_sub_path_import_accepted() {
    // `import { handle } from "hono/vercel"` — the file imports a Hono
    // sub-path; the detector still treats this file as a chain-router host.
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("hono/vercel");
    let chain = make_chain_segs(&[
        ("app", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("/health".to_string()), CallArg::Other];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/health"),
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_chain_route_consumer_fastify_put() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("fastify");
    let chain = make_chain_segs(&[
        ("fastify", SegmentKind::Identifier),
        ("put", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("/items/:id".to_string()),
        CallArg::Other,
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Put));
            assert_eq!(name, "/items/{}");
        }
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_chain_route_consumer_fastify_plugin_accepted() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("fastify-plugin");
    let chain = make_chain_segs(&[
        ("server", SegmentKind::Identifier),
        ("delete", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("/items/:id".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/items/{}"),
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_chain_route_consumer_no_framework_import_skipped() {
    // Same chain shape as an Express route, but the file imports `lodash`
    // instead of `express` — must not emit.
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("lodash");
    let chain = make_chain_segs(&[
        ("response", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("/users".to_string()),
        CallArg::Other,
    ];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_chain_route_consumer_non_verb_leaf_skipped() {
    // `.use(middleware)` and `.listen(port)` are not HTTP verbs.
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("express");
    let chain = make_chain_segs(&[
        ("app", SegmentKind::Identifier),
        ("use", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_chain_route_consumer_handler_only_call_skipped() {
    // `router.all(handler)` with no path — first arg is the handler
    // function, not a string — emit nothing (can't be paired).
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_framework_import("express");
    let chain = make_chain_segs(&[
        ("router", SegmentKind::Identifier),
        ("all", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Ident("handler".to_string())];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

// ---------------------------------------------------------------------------
// Message-queue chain + decorator emissions
// ---------------------------------------------------------------------------

fn make_ctx_with_mq_import(pkg: &str) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/messaging.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: pkg.to_string(),
            module_path: Some(pkg.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

#[test]
fn test_mq_producer_nats_publish() {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, NamedChannelKind,
    };
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_mq_import("nats");
    let chain = make_chain_segs(&[
        ("nc", SegmentKind::Identifier),
        ("publish", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("user.created".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "user.created");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_producer_redis_publish() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_mq_import("ioredis");
    let chain = make_chain_segs(&[
        ("redis", SegmentKind::Identifier),
        ("publish", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("price-updates".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "price-updates");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_producer_amqp_publish() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_mq_import("amqplib");
    let chain = make_chain_segs(&[
        ("channel", SegmentKind::Identifier),
        ("publish", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("orders".to_string()),
        CallArg::StringLit("order.created".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(role, ChannelRole::Producer);
            // amqplib's first arg is the exchange — that becomes the pairing key.
            assert_eq!(name, "orders");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_producer_mqtt_publish() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_mq_import("mqtt");
    let chain = make_chain_segs(&[
        ("client", SegmentKind::Identifier),
        ("publish", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("home/livingroom/temp".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "home/livingroom/temp");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_consumer_nats_subscribe() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_mq_import("nats");
    let chain = make_chain_segs(&[
        ("nc", SegmentKind::Identifier),
        ("subscribe", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("user.created".to_string())];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "user.created");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_consumer_redis_subscribe() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_mq_import("redis");
    let chain = make_chain_segs(&[
        ("redis", SegmentKind::Identifier),
        ("subscribe", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("price-updates".to_string())];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { role, name, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "price-updates");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_consumer_amqp_consume() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_mq_import("amqplib");
    let chain = make_chain_segs(&[
        ("channel", SegmentKind::Identifier),
        ("consume", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("orders".to_string()),
        CallArg::Ident("handler".to_string()),
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { role, name, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "orders");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_consumer_message_pattern_decorator() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("MessagePattern", Some("user.created"), None);
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "user.created");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_consumer_event_pattern_decorator() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_decorator_flow_emission;

    let result = detect_decorator_flow_emission("EventPattern", Some("order.shipped"), None);
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "order.shipped");
        }
        other => panic!("Expected NamedChannel MessageQueue, got {other:?}"),
    }
}

#[test]
fn test_mq_no_library_import_does_not_emit() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // File imports lodash — `_.publish('x', y)` must not be misclassified
    // as an MQ producer.
    let file_ctx = make_ctx_with_mq_import("lodash");
    let chain = make_chain_segs(&[
        ("_", SegmentKind::Identifier),
        ("publish", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("x".to_string()), CallArg::Other];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_mq_chain_with_no_string_arg_does_not_emit() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `producer.publish(variable)` — first arg is an identifier, not a
    // string literal, so no pairing key. Must not emit.
    let file_ctx = make_ctx_with_mq_import("nats");
    let chain = make_chain_segs(&[
        ("nc", SegmentKind::Identifier),
        ("publish", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Ident("topic".to_string())];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_mq_unknown_verb_does_not_emit() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `nc.close()` is a connection lifecycle method, not a producer or
    // consumer — must not emit.
    let file_ctx = make_ctx_with_mq_import("nats");
    let chain = make_chain_segs(&[
        ("nc", SegmentKind::Identifier),
        ("close", SegmentKind::Property),
    ]);
    let call_args = vec![];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

// ---------------------------------------------------------------------------
// Background-job library tests — BgJob Producer/Consumer
// ---------------------------------------------------------------------------

fn make_ctx_with_bgjob_import(pkg: &str) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/jobs.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: pkg.to_string(),
            module_path: Some(pkg.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

/// Build a FileContext that imports `pkg` AND has a synthetic queue-binding
/// entry mapping `var_name` → `queue_name`, as the resolver's pre-pass would
/// populate after seeing `const var_name = new Queue("queue_name")`.
fn make_ctx_with_bgjob_binding(
    pkg: &str,
    var_name: &str,
    queue_name: &str,
) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/jobs.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![
            crate::indexer::resolve::engine::ImportEntry {
                imported_name: pkg.to_string(),
                module_path: Some(pkg.to_string()),
                alias: None,
                is_wildcard: false,
            },
            crate::indexer::resolve::engine::ImportEntry {
                imported_name: format!("__ts_bgjob_queue_binding__:{}", var_name),
                module_path: Some(queue_name.to_string()),
                alias: None,
                is_wildcard: false,
            },
        ],
        file_namespace: None,
    }
}

#[test]
fn test_bgjob_producer_bullmq_add_with_queue_binding() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `const queue = new Queue('email-queue')` followed by
    // `queue.add('send-email', data)` — pre-pass captures the binding so the
    // pairing key is `queueName/jobName`.
    let file_ctx = make_ctx_with_bgjob_binding("bullmq", "queue", "email-queue");
    let chain = make_chain_segs(&[
        ("queue", SegmentKind::Identifier),
        ("add", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("send-email".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "email-queue/send-email");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_producer_bullmq_add_no_binding_falls_back() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // No queue binding visible in this file — fall back to jobName-only key.
    let file_ctx = make_ctx_with_bgjob_import("bullmq");
    let chain = make_chain_segs(&[
        ("queue", SegmentKind::Identifier),
        ("add", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("send-email".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "send-email");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_producer_bull_add() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_bgjob_import("bull");
    let chain = make_chain_segs(&[
        ("emailQueue", SegmentKind::Identifier),
        ("add", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("welcome".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "welcome");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_producer_agenda_now() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_bgjob_import("agenda");
    let chain = make_chain_segs(&[
        ("agenda", SegmentKind::Identifier),
        ("now", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("send-report".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "send-report");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_producer_agenda_every() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_bgjob_import("agenda");
    let chain = make_chain_segs(&[
        ("agenda", SegmentKind::Identifier),
        ("every", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("daily-cleanup".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "daily-cleanup");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_consumer_bullmq_worker_ctor() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_bgjob_import("bullmq");
    // `new Worker('email-queue', processor)` lands as a single-segment chain
    // with the constructor name. emit_new_ref populates the same chain shape
    // and call_args so the detector treats it as a Consumer binding. The
    // emitted key is `queueName/*` — the Worker handles every job in the queue.
    let chain = make_chain_segs(&[("Worker", SegmentKind::Identifier)]);
    let call_args = vec![
        CallArg::StringLit("email-queue".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "email-queue/*");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_consumer_bullmq_worker_on_lifecycle() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `worker.on('completed', h)` where `worker` was bound to
    // `new Worker('email-queue', ...)`. The pre-pass stashed the binding so
    // the listener gets the same `queueName/*` pairing key as the constructor.
    let file_ctx = make_ctx_with_bgjob_binding("bullmq", "worker", "email-queue");
    let chain = make_chain_segs(&[
        ("worker", SegmentKind::Identifier),
        ("on", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("completed".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "email-queue/*");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_consumer_worker_on_without_binding_does_not_emit() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // Without a queue binding for the chain root, `.on('completed', h)` could
    // be ANY EventEmitter listener — must not emit.
    let file_ctx = make_ctx_with_bgjob_import("bullmq");
    let chain = make_chain_segs(&[
        ("emitter", SegmentKind::Identifier),
        ("on", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("completed".to_string()),
        CallArg::Other,
    ];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_bgjob_consumer_worker_on_non_lifecycle_event_does_not_emit() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // Even with a queue binding, `.on('click', h)` is not a BullMQ lifecycle
    // event and must not emit.
    let file_ctx = make_ctx_with_bgjob_binding("bullmq", "worker", "email-queue");
    let chain = make_chain_segs(&[
        ("worker", SegmentKind::Identifier),
        ("on", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("click".to_string()),
        CallArg::Other,
    ];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_bgjob_consumer_bull_process_with_binding() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_bgjob_binding("bull", "queue", "email-queue");
    let chain = make_chain_segs(&[
        ("queue", SegmentKind::Identifier),
        ("process", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("welcome".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "email-queue/welcome");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_producer_beequeue_createjob() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // bee-queue: `queue.createJob({...data}).save()` — payload is an object
    // literal stored as CallArg::Other. queueName alone is the pairing key,
    // suffixed with `*` to align with Worker constructor / `worker.on`.
    let file_ctx = make_ctx_with_bgjob_binding("bee-queue", "queue", "image-resize");
    let chain = make_chain_segs(&[
        ("queue", SegmentKind::Identifier),
        ("createJob", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "image-resize/*");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_consumer_agenda_define() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_bgjob_import("agenda");
    let chain = make_chain_segs(&[
        ("agenda", SegmentKind::Identifier),
        ("define", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("send-report".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "send-report");
        }
        other => panic!("Expected NamedChannel BgJob, got {other:?}"),
    }
}

#[test]
fn test_bgjob_no_emit_without_import() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // No BgJob library imported — `queue.add('x', data)` could be any
    // user-defined queue API; safer to not emit.
    let file_ctx = make_ctx_with_mq_import("lodash");
    let chain = make_chain_segs(&[
        ("queue", SegmentKind::Identifier),
        ("add", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("send-email".to_string()),
        CallArg::Other,
    ];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_bgjob_no_emit_for_unknown_ctor() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `new Queue('email-queue')` is a Producer-side declaration with no job
    // name yet — declared in a separate statement. Constructor recognition
    // only fires on Consumer constructors like `Worker`.
    let file_ctx = make_ctx_with_bgjob_import("bullmq");
    let chain = make_chain_segs(&[("Queue", SegmentKind::Identifier)]);
    let call_args = vec![CallArg::StringLit("email-queue".to_string())];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_bgjob_no_emit_for_unknown_verb() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `queue.close()` is a lifecycle method, not a Producer/Consumer.
    let file_ctx = make_ctx_with_bgjob_import("bullmq");
    let chain = make_chain_segs(&[
        ("queue", SegmentKind::Identifier),
        ("close", SegmentKind::Property),
    ]);
    let call_args = vec![];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_bgjob_no_emit_without_string_arg() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // Without a literal job name first arg there's no pairing key.
    let file_ctx = make_ctx_with_bgjob_import("bullmq");
    let chain = make_chain_segs(&[
        ("queue", SegmentKind::Identifier),
        ("add", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Ident("dynamicName".to_string())];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

// ---------------------------------------------------------------------------
// gRPC / Connect RpcCall tests — Producer/Consumer
// ---------------------------------------------------------------------------

fn make_ctx_with_rpc_import(pkg: &str) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/rpc.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: pkg.to_string(),
            module_path: Some(pkg.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

#[test]
fn test_rpc_producer_connect_chain_three_segments() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // Connect: `client.users.getUser(req)` → service=`users`, method=`getUser`.
    let file_ctx = make_ctx_with_rpc_import("@connectrpc/connect");
    let chain = make_chain_segs(&[
        ("client", SegmentKind::Identifier),
        ("users", SegmentKind::Property),
        ("getUser", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Producer);
            // Canonical RPC key is lowercase so camelCase clients pair with
            // PascalCase decorator service names.
            assert_eq!(name, "users/getuser");
        }
        other => panic!("Expected NamedChannel RpcCall, got {other:?}"),
    }
}

#[test]
fn test_rpc_producer_nice_grpc_chain() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_rpc_import("nice-grpc");
    let chain = make_chain_segs(&[
        ("client", SegmentKind::Identifier),
        ("UserService", SegmentKind::Property),
        ("getUser", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Producer);
            // `UserService` is service-suffix-stripped to `User`, then
            // lowercased to the canonical RPC pairing key.
            assert_eq!(name, "user/getuser");
        }
        other => panic!("Expected NamedChannel RpcCall, got {other:?}"),
    }
}

#[test]
fn test_rpc_producer_tsproto_two_segments() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // ts-proto client: `userServiceClient.getUser(req)` — root is the
    // generated service client identifier. The `ServiceClient` suffix is
    // stripped so the pairing key aligns with @GrpcMethod('UserService', …).
    let file_ctx = make_ctx_with_rpc_import("@grpc/grpc-js");
    let chain = make_chain_segs(&[
        ("UserServiceClient", SegmentKind::Identifier),
        ("getUser", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "user/getuser");
        }
        other => panic!("Expected NamedChannel RpcCall, got {other:?}"),
    }
}

#[test]
fn test_rpc_consumer_addService_emits_wildcard() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `server.addService(UserService, { getUser: handler })` — first arg is
    // the PascalCase service definition identifier. Method names live in the
    // object literal (CallArg::Other), so the emission is wildcard-suffixed.
    let file_ctx = make_ctx_with_rpc_import("@grpc/grpc-js");
    let chain = make_chain_segs(&[
        ("server", SegmentKind::Identifier),
        ("addService", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::Ident("UserService".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "user/*");
        }
        other => panic!("Expected NamedChannel RpcCall Consumer, got {other:?}"),
    }
}

#[test]
fn test_rpc_consumer_grpc_method_decorator_two_args() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_grpc_decorator_flow_emission;
    use crate::types::CallArg;

    // `@GrpcMethod('UserService', 'getUser')` → "User/getUser".
    let call_args = vec![
        CallArg::StringLit("UserService".to_string()),
        CallArg::StringLit("getUser".to_string()),
    ];
    match detect_grpc_decorator_flow_emission("GrpcMethod", &call_args, "fetchUser").unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "user/getuser");
        }
        other => panic!("Expected NamedChannel RpcCall Consumer, got {other:?}"),
    }
}

#[test]
fn test_rpc_consumer_grpc_method_decorator_one_arg_uses_method_name() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_grpc_decorator_flow_emission;
    use crate::types::CallArg;

    // `@GrpcMethod('UserService') async getUser(...) {}` — second arg
    // absent; enclosing method name `getUser` becomes the method.
    let call_args = vec![CallArg::StringLit("UserService".to_string())];
    match detect_grpc_decorator_flow_emission("GrpcMethod", &call_args, "getUser").unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "user/getuser");
        }
        other => panic!("Expected NamedChannel RpcCall Consumer, got {other:?}"),
    }
}

#[test]
fn test_rpc_consumer_grpc_stream_method_decorator() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_grpc_decorator_flow_emission;
    use crate::types::CallArg;

    let call_args = vec![
        CallArg::StringLit("UserService".to_string()),
        CallArg::StringLit("streamUsers".to_string()),
    ];
    match detect_grpc_decorator_flow_emission("GrpcStreamMethod", &call_args, "h").unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "user/streamusers");
        }
        other => panic!("Expected NamedChannel RpcCall Consumer, got {other:?}"),
    }
}

#[test]
fn test_rpc_no_emit_without_import() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // Without a gRPC import, `client.users.getUser(...)` could be any nested
    // API client — don't emit.
    let file_ctx = make_ctx_with_rpc_import("lodash");
    let chain = make_chain_segs(&[
        ("client", SegmentKind::Identifier),
        ("users", SegmentKind::Property),
        ("getUser", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_rpc_no_emit_for_lifecycle_leaf() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `client.users.then(...)` is a promise chain on the result of a prior
    // call — must not be treated as an RPC method named `then`.
    let file_ctx = make_ctx_with_rpc_import("nice-grpc");
    let chain = make_chain_segs(&[
        ("client", SegmentKind::Identifier),
        ("users", SegmentKind::Property),
        ("then", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_rpc_no_emit_for_addService_with_non_pascal_arg() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `server.addService(serviceVar, ...)` with a lowercase identifier
    // doesn't look like a service definition — don't emit.
    let file_ctx = make_ctx_with_rpc_import("@grpc/grpc-js");
    let chain = make_chain_segs(&[
        ("server", SegmentKind::Identifier),
        ("addService", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::Ident("serviceVar".to_string()),
        CallArg::Other,
    ];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_rpc_canonical_key_pairs_camel_and_pascal() {
    use super::hooks::canonical_rpc_key;

    // Producer (camelCase service from a Connect chain) and Consumer (PascalCase
    // service from a `@GrpcMethod` decorator) produce IDENTICAL canonical keys.
    let prod = canonical_rpc_key("users", "getUser");
    let cons = canonical_rpc_key("Users", "GetUser");
    assert_eq!(prod, cons);
    assert_eq!(prod, "users/getuser");
}

#[test]
fn test_rpc_canonical_key_preserves_wildcard() {
    use super::hooks::canonical_rpc_key;

    // The `*` wildcard must NOT be lowercased away (it's not letters anyway,
    // but the contract is explicit).
    let key = canonical_rpc_key("UserService", "*");
    assert_eq!(key, "userservice/*");
}

#[test]
fn test_rpc_addservice_with_object_keys_expands_to_per_method_emissions() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_addservice_object_keys;
    use crate::types::{CallArg, SegmentKind};

    // `server.addService(UserService, { getUser: h1, listUsers: h2 })` —
    // when the object literal's property names are captured, expand to one
    // Consumer emission per registered method.
    let file_ctx = make_ctx_with_rpc_import("@grpc/grpc-js");
    let chain = make_chain_segs(&[
        ("server", SegmentKind::Identifier),
        ("addService", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::Ident("UserService".to_string()),
        CallArg::ObjectKeys(vec![
            ("getUser".to_string(), None),
            ("listUsers".to_string(), None),
        ]),
    ];
    let emissions = detect_addservice_object_keys(&chain, &call_args, &file_ctx).unwrap();
    assert_eq!(emissions.len(), 2);
    let names: Vec<String> = emissions
        .iter()
        .map(|e| match e {
            FlowEmission::NamedChannel { kind, role, name, .. } => {
                assert_eq!(*kind, NamedChannelKind::RpcCall);
                assert_eq!(*role, ChannelRole::Consumer);
                name.clone()
            }
            _ => panic!("expected NamedChannel RpcCall"),
        })
        .collect();
    assert_eq!(names, vec!["user/getuser", "user/listusers"]);
}

#[test]
fn test_rpc_addservice_falls_back_to_wildcard_without_object_keys() {
    use super::hooks::{detect_addservice_object_keys, detect_chain_flow_emission};
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::{CallArg, SegmentKind};

    // Without object-key capture (second arg is just `Other`), the multi-
    // emission expansion declines (`None`) and the regular chain detector
    // emits the wildcard form.
    let file_ctx = make_ctx_with_rpc_import("@grpc/grpc-js");
    let chain = make_chain_segs(&[
        ("server", SegmentKind::Identifier),
        ("addService", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::Ident("UserService".to_string()),
        CallArg::Other,
    ];
    assert!(detect_addservice_object_keys(&chain, &call_args, &file_ctx).is_none());
    let single = detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap();
    match single {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "user/*"),
        other => panic!("expected wildcard NamedChannel, got {other:?}"),
    }
}

#[test]
fn test_rpc_grpc_decorator_ignored_for_non_rpc_decorator() {
    use super::hooks::detect_grpc_decorator_flow_emission;
    use crate::types::CallArg;

    // The gRPC decorator detector must not fire for unrelated decorators.
    let call_args = vec![CallArg::StringLit("UserService".to_string())];
    assert!(detect_grpc_decorator_flow_emission("Controller", &call_args, "x").is_none());
    assert!(detect_grpc_decorator_flow_emission("Get", &call_args, "x").is_none());
    assert!(detect_grpc_decorator_flow_emission("Injectable", &[], "x").is_none());
}

// ---------------------------------------------------------------------------
// ConfigLookup tests
// ---------------------------------------------------------------------------

#[test]
fn test_config_lookup_process_env_member_access() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_member_access_config_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("process", SegmentKind::Identifier),
        ("env", SegmentKind::Property),
        ("NODE_ENV", SegmentKind::Property),
    ]);
    match detect_member_access_config_emission(&chain).unwrap() {
        FlowEmission::ConfigLookup { key } => assert_eq!(key, "NODE_ENV"),
        other => panic!("Expected ConfigLookup, got {other:?}"),
    }
}

#[test]
fn test_config_lookup_import_meta_env_member_access() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_member_access_config_emission;
    use crate::types::SegmentKind;

    let chain = make_chain_segs(&[
        ("import", SegmentKind::Identifier),
        ("meta", SegmentKind::Property),
        ("env", SegmentKind::Property),
        ("VITE_API_BASE", SegmentKind::Property),
    ]);
    match detect_member_access_config_emission(&chain).unwrap() {
        FlowEmission::ConfigLookup { key } => assert_eq!(key, "VITE_API_BASE"),
        other => panic!("Expected ConfigLookup, got {other:?}"),
    }
}

#[test]
fn test_config_lookup_config_service_get_call() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_config_call_emission;
    use crate::types::{CallArg, SegmentKind};

    let chain = make_chain_segs(&[
        ("configService", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("DATABASE_URL".to_string())];
    match detect_config_call_emission(&chain, &call_args).unwrap() {
        FlowEmission::ConfigLookup { key } => assert_eq!(key, "DATABASE_URL"),
        other => panic!("Expected ConfigLookup, got {other:?}"),
    }
}

#[test]
fn test_config_lookup_rejects_unrelated_get_calls() {
    use super::hooks::detect_config_call_emission;
    use crate::types::{CallArg, SegmentKind};

    // `map.get('key')` (Map data structure) — not a config service.
    let chain = make_chain_segs(&[
        ("map", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("DATABASE_URL".to_string())];
    assert!(detect_config_call_emission(&chain, &call_args).is_none());
}

// ---------------------------------------------------------------------------
// FeatureFlag tests
// ---------------------------------------------------------------------------

fn make_ctx_with_ff_import(pkg: &str) -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/feature.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: pkg.to_string(),
            module_path: Some(pkg.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

#[test]
fn test_feature_flag_growthbook_is_on() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_feature_flag_chain_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_ff_import("@growthbook/growthbook");
    let chain = make_chain_segs(&[
        ("gb", SegmentKind::Identifier),
        ("isOn", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("new-checkout".to_string())];
    match detect_feature_flag_chain_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::FeatureFlag { flag_name } => assert_eq!(flag_name, "new-checkout"),
        other => panic!("Expected FeatureFlag, got {other:?}"),
    }
}

#[test]
fn test_feature_flag_launchdarkly_variation() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_feature_flag_chain_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_ff_import("launchdarkly-js-client-sdk");
    let chain = make_chain_segs(&[
        ("ldClient", SegmentKind::Identifier),
        ("variation", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("dashboard-v2".to_string()),
        CallArg::Other,
        CallArg::Literal("false".to_string()),
    ];
    match detect_feature_flag_chain_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::FeatureFlag { flag_name } => assert_eq!(flag_name, "dashboard-v2"),
        other => panic!("Expected FeatureFlag, got {other:?}"),
    }
}

#[test]
fn test_feature_flag_statsig_check_gate() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_feature_flag_chain_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_ff_import("statsig-js");
    let chain = make_chain_segs(&[
        ("statsig", SegmentKind::Identifier),
        ("checkGate", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("beta_user".to_string())];
    match detect_feature_flag_chain_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::FeatureFlag { flag_name } => assert_eq!(flag_name, "beta_user"),
        other => panic!("Expected FeatureFlag, got {other:?}"),
    }
}

#[test]
fn test_feature_flag_use_feature_flag_hook_without_import() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_feature_flag_chain_emission;
    use crate::types::{CallArg, SegmentKind};

    // `useFeatureFlag('x')` — generic React hook shape, fires without an
    // explicit SDK import (multiple libs export a hook by this name).
    let file_ctx = crate::indexer::resolve::engine::FileContext {
        file_path: "src/component.tsx".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let chain = make_chain_segs(&[("useFeatureFlag", SegmentKind::Identifier)]);
    let call_args = vec![CallArg::StringLit("show-banner".to_string())];
    match detect_feature_flag_chain_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::FeatureFlag { flag_name } => assert_eq!(flag_name, "show-banner"),
        other => panic!("Expected FeatureFlag, got {other:?}"),
    }
}

#[test]
fn test_feature_flag_internal_member_access_two_segments() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_member_access_feature_flag_emission;
    use crate::types::SegmentKind;

    // `featureFlags.configFile` — direct access on a feature-flag-shaped root.
    let chain = make_chain_segs(&[
        ("featureFlags", SegmentKind::Identifier),
        ("configFile", SegmentKind::Property),
    ]);
    match detect_member_access_feature_flag_emission(&chain).unwrap() {
        FlowEmission::FeatureFlag { flag_name } => assert_eq!(flag_name, "configFile"),
        other => panic!("Expected FeatureFlag, got {other:?}"),
    }
}

#[test]
fn test_feature_flag_internal_member_access_via_value() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_member_access_feature_flag_emission;
    use crate::types::SegmentKind;

    // `featureFlagsManager.value.someFlag` — three segments, peer through `value`.
    let chain = make_chain_segs(&[
        ("featureFlagsManager", SegmentKind::Identifier),
        ("value", SegmentKind::Property),
        ("someFlag", SegmentKind::Property),
    ]);
    match detect_member_access_feature_flag_emission(&chain).unwrap() {
        FlowEmission::FeatureFlag { flag_name } => assert_eq!(flag_name, "someFlag"),
        other => panic!("Expected FeatureFlag, got {other:?}"),
    }
}

#[test]
fn test_feature_flag_no_emit_without_library_import() {
    use super::hooks::detect_feature_flag_chain_emission;
    use crate::types::{CallArg, SegmentKind};

    // `gb.isOn('x')` without any feature-flag SDK imported — `gb` could be
    // anything; don't emit.
    let file_ctx = crate::indexer::resolve::engine::FileContext {
        file_path: "src/feature.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let chain = make_chain_segs(&[
        ("gb", SegmentKind::Identifier),
        ("isOn", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("new-checkout".to_string())];
    assert!(detect_feature_flag_chain_emission(&chain, &call_args, &file_ctx).is_none());
}

// ---------------------------------------------------------------------------
// DiBinding tests (via `@Inject` decorator path)
// ---------------------------------------------------------------------------

#[test]
fn test_di_binding_inject_decorator_emits() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext};
    use crate::types::{EdgeKind, ExtractedRef};

    // Construct a synthetic TypeRef ref representing `@Inject('USER_REPO')`.
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Inject".to_string(),
        kind: EdgeKind::TypeRef,
        line: 5,
        col: 0,
        module: Some("USER_REPO".to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let symbols = vec![crate::types::ExtractedSymbol {
        name: "userRepo".to_string(),
        qualified_name: "MyService.userRepo".to_string(),
        kind: crate::types::SymbolKind::Variable,
        visibility: None,
        start_line: 5,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}];
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/service.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![ImportEntry {
            imported_name: "Inject".to_string(),
            module_path: Some("@nestjs/common".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let resolver = super::hooks::TypeScriptResolver;
    let emissions = super::hooks::detect_flow_inner(&file_ctx, &ref_ctx);
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::DiBinding { container, .. } => {
            assert!(container.as_deref().unwrap_or("").starts_with("nestjs"));
        }
        other => panic!("Expected DiBinding, got {other:?}"),
    }
}

#[test]
fn test_di_binding_no_emit_for_unrelated_typeref() {
    use crate::indexer::resolve::engine::{FileContext, RefContext};
    use crate::types::{EdgeKind, ExtractedRef};

    // A non-`Inject` TypeRef ref must not emit a DiBinding.
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: "User".to_string(),
        kind: EdgeKind::TypeRef,
        line: 5,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let symbols = vec![crate::types::ExtractedSymbol {
        name: "user".to_string(),
        qualified_name: "user".to_string(),
        kind: crate::types::SymbolKind::Variable,
        visibility: None,
        start_line: 5,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}];
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/service.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let resolver = super::hooks::TypeScriptResolver;
    let emissions = super::hooks::detect_flow_inner(&file_ctx, &ref_ctx);
    assert!(emissions.is_empty());
}

#[test]
fn test_di_binding_inject_without_token_still_emits() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::indexer::resolve::engine::{FileContext, RefContext};
    use crate::types::{EdgeKind, ExtractedRef};

    // `@Inject()` with no string arg — still a DI binding intent; emit a
    // DiBinding with the bare `nestjs` container hint.
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Inject".to_string(),
        kind: EdgeKind::TypeRef,
        line: 5,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let symbols = vec![crate::types::ExtractedSymbol {
        name: "thing".to_string(),
        qualified_name: "thing".to_string(),
        kind: crate::types::SymbolKind::Variable,
        visibility: None,
        start_line: 5,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}];
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/service.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let resolver = super::hooks::TypeScriptResolver;
    let emissions = super::hooks::detect_flow_inner(&file_ctx, &ref_ctx);
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::DiBinding { container, .. } => {
            assert_eq!(container.as_deref(), Some("nestjs"));
        }
        other => panic!("Expected DiBinding, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// tRPC client Producer detection
// ---------------------------------------------------------------------------

fn make_ctx_with_trpc_import() -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/page.tsx".to_string(),
        language: "typescript".to_string(),
        imports: vec![crate::indexer::resolve::engine::ImportEntry {
            imported_name: "trpc".to_string(),
            module_path: Some("@/trpc/client".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

#[test]
fn test_trpc_producer_use_query_emits_http_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_trpc_import();
    let chain = make_chain_segs(&[
        ("trpc", SegmentKind::Identifier),
        ("polls", SegmentKind::Property),
        ("list", SegmentKind::Property),
        ("useQuery", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "/api/trpc/polls.list");
        }
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_trpc_producer_use_mutation_emits_http_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_with_trpc_import();
    let chain = make_chain_segs(&[
        ("trpc", SegmentKind::Identifier),
        ("auth", SegmentKind::Property),
        ("getLoginMethod", SegmentKind::Property),
        ("useMutation", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "/api/trpc/auth.getLoginMethod");
        }
        other => panic!("Expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_trpc_no_emit_without_trpc_import() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = crate::indexer::resolve::engine::FileContext {
        file_path: "src/page.tsx".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let chain = make_chain_segs(&[
        ("trpc", SegmentKind::Identifier),
        ("polls", SegmentKind::Property),
        ("list", SegmentKind::Property),
        ("useQuery", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::Other];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

// ---------------------------------------------------------------------------
// Electron ipcMain Consumer tests
// ---------------------------------------------------------------------------

#[test]
fn test_ipc_ipcmain_handle_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_mailer(); // Reuse — Electron detection is import-free.
    let chain = make_chain_segs(&[
        ("ipcMain", SegmentKind::Identifier),
        ("handle", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("file:save".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::IpcCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "file:save");
        }
        other => panic!("Expected NamedChannel IpcCall, got {other:?}"),
    }
}

#[test]
fn test_ipc_ipcmain_on_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("ipcMain", SegmentKind::Identifier),
        ("on", SegmentKind::Property),
    ]);
    let call_args = vec![
        CallArg::StringLit("renderer-ready".to_string()),
        CallArg::Other,
    ];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::IpcCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "renderer-ready");
        }
        other => panic!("Expected NamedChannel IpcCall, got {other:?}"),
    }
}

#[test]
fn test_ipc_ipcmain_no_emit_for_lifecycle_verbs() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `ipcMain.removeAllListeners(...)` is a lifecycle call, not a handler
    // registration — must not emit.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("ipcMain", SegmentKind::Identifier),
        ("removeAllListeners", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::StringLit("anything".to_string())];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

// ---------------------------------------------------------------------------
// Mailer Producer chain tests
// ---------------------------------------------------------------------------

fn make_ctx_mailer() -> crate::indexer::resolve::engine::FileContext {
    crate::indexer::resolve::engine::FileContext {
        file_path: "src/jobs/notifier.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    }
}

#[test]
fn test_mailer_producer_nodemailer_send_mail_template() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `transport.sendMail({ template: 'welcome', subject: s })`.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("transport", SegmentKind::Identifier),
        ("sendMail", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::ObjectKeys(vec![
        ("template".to_string(), Some("welcome".to_string())),
        ("subject".to_string(), None),
    ])];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::Mailer);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "welcome");
        }
        other => panic!("Expected NamedChannel Mailer, got {other:?}"),
    }
}

#[test]
fn test_mailer_producer_sendgrid_send_template_id() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `sgMail.send({ to: ..., templateId: 'd-12345', ... })`.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("sgMail", SegmentKind::Identifier),
        ("send", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::ObjectKeys(vec![
        ("to".to_string(), None),
        ("templateId".to_string(), Some("d-12345".to_string())),
    ])];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::Mailer);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "d-12345");
        }
        other => panic!("Expected NamedChannel Mailer, got {other:?}"),
    }
}

#[test]
fn test_mailer_producer_nestjs_mailer_service() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `mailerService.sendMail({ template: 'verify-email', context: ... })`.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("mailerService", SegmentKind::Identifier),
        ("sendMail", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::ObjectKeys(vec![
        ("template".to_string(), Some("verify-email".to_string())),
        ("context".to_string(), None),
    ])];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::Mailer);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "verify-email");
        }
        other => panic!("Expected NamedChannel Mailer, got {other:?}"),
    }
}

#[test]
fn test_mailer_no_emit_without_template_field() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // No `template` / `templateId` key — no static pairing key, no emission.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("transport", SegmentKind::Identifier),
        ("sendMail", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::ObjectKeys(vec![
        ("to".to_string(), None),
        ("subject".to_string(), Some("hi".to_string())),
        ("html".to_string(), None),
    ])];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_mailer_no_emit_when_template_value_is_dynamic() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `template: templateName` — variable, not a string literal. No static key.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("transport", SegmentKind::Identifier),
        ("sendMail", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::ObjectKeys(vec![
        ("template".to_string(), None),
    ])];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_mailer_no_emit_for_unknown_verb() {
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `transport.verify(...)` is a lifecycle call, not a mailer send.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("transport", SegmentKind::Identifier),
        ("verify", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::ObjectKeys(vec![
        ("template".to_string(), Some("welcome".to_string())),
    ])];
    assert!(detect_chain_flow_emission(&chain, &call_args, &file_ctx).is_none());
}

#[test]
fn test_mailer_resend_emails_send_emits_without_template() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_chain_flow_emission;
    use crate::types::{CallArg, SegmentKind};

    // `resend.emails.send({from, to, react: <Welcome />})` — no template
    // key, but the library-name root drives the emission.
    let file_ctx = make_ctx_mailer();
    let chain = make_chain_segs(&[
        ("resend", SegmentKind::Identifier),
        ("emails", SegmentKind::Property),
        ("send", SegmentKind::Property),
    ]);
    let call_args = vec![CallArg::ObjectKeys(vec![
        ("from".to_string(), Some("noreply@x.com".to_string())),
        ("to".to_string(), None),
        ("subject".to_string(), Some("Welcome".to_string())),
    ])];
    match detect_chain_flow_emission(&chain, &call_args, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::Mailer);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "ts.resend");
        }
        other => panic!("Expected Mailer NamedChannel, got {other:?}"),
    }
}

#[test]
fn decorator_subscribe_message_emits_ws_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_decorator_flow_emission;
    let result = detect_decorator_flow_emission("SubscribeMessage", Some("chat.message"), None);
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "chat.message");
        }
        other => panic!("Expected WebSocket Consumer, got {other:?}"),
    }
}

#[test]
fn decorator_websocket_gateway_emits_ws_consumer_with_class_name() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_decorator_flow_emission;
    let result = detect_decorator_flow_emission("WebSocketGateway", None, Some("ChatGateway"));
    match result.unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "ChatGateway");
        }
        other => panic!("Expected WebSocket Consumer, got {other:?}"),
    }
}
