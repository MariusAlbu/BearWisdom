use super::resolve::*;
use crate::indexer::resolve::engine::{LanguageResolver, RefContext};
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
        chain: None,
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
        symbol_from_snippet: vec![],
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
            symbol_from_snippet: vec![],
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

/// Build a minimal ProjectContext with react and @tanstack/react-query as packages.
fn make_ts_project_ctx() -> ProjectContext {
    use crate::indexer::manifest::{ManifestData, ManifestKind};
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
    file_package_id: None,
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
    file_package_id: None,
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
    file_package_id: None,
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
    file_package_id: None,
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
    file_package_id: None,
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
    file_package_id: None,
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

    assert!(super::resolve::is_manifest_ts_package(&ctx, None,"react"));
    assert!(super::resolve::is_manifest_ts_package(&ctx, None,"@tanstack/react-query"));
    assert!(super::resolve::is_manifest_ts_package(&ctx, None,"@tanstack"));
    assert!(super::resolve::is_manifest_ts_package(&ctx, None,"fs"));
    assert!(super::resolve::is_manifest_ts_package(&ctx, None,"path"));
    assert!(super::resolve::is_manifest_ts_package(&ctx, None,"node:fs")); // node: protocol always external

    assert!(!super::resolve::is_manifest_ts_package(&ctx, None,"./utils"));
    assert!(!super::resolve::is_manifest_ts_package(&ctx, None,"../shared"));
    assert!(!super::resolve::is_manifest_ts_package(&ctx, None,"MyInternalService"));
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

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
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
    use crate::indexer::manifest::{ManifestData, ManifestKind};

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
    use crate::indexer::manifest::{ManifestData, ManifestKind};

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
    use crate::indexer::manifest::{ManifestData, ManifestKind};

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
        module: Some("./quick-create-button".to_string()),
        chain: None,
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
    use crate::indexer::manifest::{ManifestData, ManifestKind};

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
fn tsconfig_alias_parser_handles_realworld_landing_shape() {
    // Exact shape from ts-rallly's apps/landing/tsconfig.json — the wild
    // case we missed. Has `extends`, `baseUrl`, mixed-type compilerOptions,
    // and trailing commas may or may not appear.
    use crate::indexer::manifest::npm::parse_tsconfig_paths;
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
    use crate::indexer::manifest::npm::parse_tsconfig_paths;

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
