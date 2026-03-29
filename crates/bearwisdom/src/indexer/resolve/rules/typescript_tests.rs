use super::*;
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
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line,
        module: None,
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
    }
}

fn make_ts_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        content: None,
        has_errors: false,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
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
            content: None,
            has_errors: false,
            symbols: f.symbols.clone(),
            refs: f.refs.clone(),
            routes: vec![],
            db_sets: vec![],
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

/// Build a minimal ProjectContext with react and @tanstack/react-query as packages.
fn make_ts_project_ctx() -> ProjectContext {
    let mut ctx = ProjectContext::default();
    ctx.ts_packages.insert("react".to_string());
    ctx.ts_packages.insert("react-dom".to_string());
    ctx.ts_packages.insert("@tanstack/react-query".to_string());
    ctx.ts_packages.insert("@tanstack".to_string());
    ctx.ts_packages.insert("express".to_string());
    ctx.ts_packages.insert("lodash".to_string());
    // Node.js built-ins (subset)
    for builtin in &["fs", "path", "http", "https", "crypto", "os", "events", "stream"] {
        ctx.ts_packages.insert(builtin.to_string());
    }
    ctx.ts_packages.insert("node".to_string());
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
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_none(), "External package import should not resolve");
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
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
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
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
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
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
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
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
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
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
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
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
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
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &usage_ref_ctx, Some(&ctx));
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

    assert!(ctx.is_external_ts_package("react"));
    assert!(ctx.is_external_ts_package("@tanstack/react-query"));
    assert!(ctx.is_external_ts_package("@tanstack"));
    assert!(ctx.is_external_ts_package("fs"));
    assert!(ctx.is_external_ts_package("path"));
    assert!(ctx.is_external_ts_package("node:fs")); // node: protocol always external

    assert!(!ctx.is_external_ts_package("./utils"));
    assert!(!ctx.is_external_ts_package("../shared"));
    assert!(!ctx.is_external_ts_package("MyInternalService"));
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
    };

    // The import binding itself is classified as external.
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert!(ns.is_some(), "React namespace import should be classified as external");
    assert_eq!(ns.unwrap(), "react");

    // Resolve returns None (bare specifier, not in index).
    let resolution = resolver.resolve(&file_ctx, &ref_ctx, &index_empty());
    assert!(resolution.is_none());
}

fn index_empty() -> SymbolIndex {
    SymbolIndex::build(&[], &HashMap::new())
}
