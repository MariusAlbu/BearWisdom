use super::resolve::GoResolver;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex, SymbolInfo};
use crate::types::*;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test helpers
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
        byte_offset: 0,
    }
}

fn make_import_ref(
    source_idx: usize,
    last_segment: &str,
    full_path: &str,
    line: u32,
) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: last_segment.to_string(),
        kind: EdgeKind::Imports,
        line,
        module: Some(full_path.to_string()),
        chain: None,
        byte_offset: 0,
    }
}
fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "go".to_string(),
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
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

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
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

// ---------------------------------------------------------------------------
// go.mod parsing tests
// ---------------------------------------------------------------------------

use crate::indexer::project_context::parse_go_mod;

#[test]
fn test_parse_go_mod_simple() {
    let content = r#"module code.gitea.io/gitea

go 1.21
"#;
    let data = parse_go_mod(content);
    assert_eq!(data.module_path, Some("code.gitea.io/gitea".to_string()));
    assert!(data.require_paths.is_empty());
}

#[test]
fn test_parse_go_mod_require_block() {
    let content = r#"module github.com/mattermost/mattermost-server

go 1.21

require (
    github.com/gin-gonic/gin v1.9.1
    golang.org/x/crypto v0.14.0
    github.com/stretchr/testify v1.8.4
)
"#;
    let data = parse_go_mod(content);
    assert_eq!(
        data.module_path,
        Some("github.com/mattermost/mattermost-server".to_string())
    );
    assert_eq!(data.require_paths.len(), 3);
    assert!(data.require_paths.contains(&"github.com/gin-gonic/gin".to_string()));
    assert!(data.require_paths.contains(&"golang.org/x/crypto".to_string()));
    assert!(data.require_paths.contains(&"github.com/stretchr/testify".to_string()));
}

#[test]
fn test_parse_go_mod_single_line_require() {
    let content = r#"module example.com/myapp

go 1.20

require github.com/some/pkg v1.0.0
"#;
    let data = parse_go_mod(content);
    assert_eq!(data.module_path, Some("example.com/myapp".to_string()));
    assert_eq!(data.require_paths, vec!["github.com/some/pkg".to_string()]);
}

#[test]
fn test_parse_go_mod_indirect_deps() {
    // Indirect deps should be included (we don't distinguish).
    let content = r#"module go-pocketbase.io/pocketbase

go 1.21

require (
    github.com/pocketbase/dbx v1.10.1
    github.com/spf13/cast v1.5.1 // indirect
)
"#;
    let data = parse_go_mod(content);
    assert_eq!(
        data.module_path,
        Some("go-pocketbase.io/pocketbase".to_string())
    );
    assert_eq!(data.require_paths.len(), 2);
}

#[test]
fn test_parse_go_mod_comments_ignored() {
    let content = r#"// This is a comment at the top
module example.com/app

// go version
go 1.21
"#;
    let data = parse_go_mod(content);
    assert_eq!(data.module_path, Some("example.com/app".to_string()));
}

// ---------------------------------------------------------------------------
// ProjectContext.is_external_go_import tests
// ---------------------------------------------------------------------------

#[test]
fn test_is_external_go_import_with_module_path() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut go_mod = ManifestData::default();
    go_mod.module_path = Some("code.gitea.io/gitea".to_string());
    ctx.manifests.insert(ManifestKind::GoMod, go_mod);

    // Internal: exact match
    assert!(!super::resolve::is_manifest_go_external(&ctx,"code.gitea.io/gitea"));
    // Internal: sub-package
    assert!(!super::resolve::is_manifest_go_external(&ctx,"code.gitea.io/gitea/modules/log"));
    assert!(!super::resolve::is_manifest_go_external(&ctx,"code.gitea.io/gitea/services/auth"));
    // External: different host
    assert!(super::resolve::is_manifest_go_external(&ctx,"github.com/gin-gonic/gin"));
    assert!(super::resolve::is_manifest_go_external(&ctx,"golang.org/x/crypto"));
    // External: standard library is internal by our heuristic but shouldn't matter —
    // stdlib won't be in the index anyway
    assert!(super::resolve::is_manifest_go_external(&ctx,"fmt")); // no dot → external per module-path logic
}

#[test]
fn test_is_external_go_import_no_module_path_fallback() {
    let ctx = ProjectContext::default(); // no go_module_path

    // Heuristic: dot in first segment → external
    assert!(super::resolve::is_manifest_go_external(&ctx,"github.com/gin-gonic/gin"));
    assert!(super::resolve::is_manifest_go_external(&ctx,"golang.org/x/net"));
    // Standard library: no dot → not external
    assert!(!super::resolve::is_manifest_go_external(&ctx,"fmt"));
    assert!(!super::resolve::is_manifest_go_external(&ctx,"net/http"));
    assert!(!super::resolve::is_manifest_go_external(&ctx,"encoding/json"));
}

#[test]
fn test_is_external_go_import_prefix_boundary() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut go_mod = ManifestData::default();
    go_mod.module_path = Some("github.com/myorg/myrepo".to_string());
    ctx.manifests.insert(ManifestKind::GoMod, go_mod);

    // "github.com/myorg/myrepox" must NOT be treated as internal
    assert!(super::resolve::is_manifest_go_external(&ctx,"github.com/myorg/myrepox"));
    // Sub-packages are internal
    assert!(!super::resolve::is_manifest_go_external(&ctx,"github.com/myorg/myrepo/pkg/api"));
}

// ---------------------------------------------------------------------------
// GoResolver::build_file_context tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_file_context_package_name() {
    let file = make_file(
        "handlers/user.go",
        vec![
            make_symbol(
                "UserHandler",
                "handlers.UserHandler",
                SymbolKind::Struct,
                Visibility::Public,
                Some("handlers"),
            ),
            make_symbol(
                "Handle",
                "handlers.UserHandler.Handle",
                SymbolKind::Method,
                Visibility::Public,
                Some("handlers.UserHandler"),
            ),
        ],
        vec![],
    );

    let resolver = GoResolver;
    let ctx = resolver.build_file_context(&file, None);

    assert_eq!(ctx.file_namespace, Some("handlers".to_string()));
    assert_eq!(ctx.language, "go");
}

#[test]
fn test_build_file_context_imports() {
    let file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![
            make_import_ref(0, "gin", "github.com/gin-gonic/gin", 3),
            make_import_ref(0, "fmt", "fmt", 4),
        ],
    );

    let resolver = GoResolver;
    let ctx = resolver.build_file_context(&file, None);

    assert_eq!(ctx.imports.len(), 2);

    let gin_import = ctx.imports.iter().find(|i| i.imported_name == "gin").unwrap();
    assert_eq!(gin_import.module_path.as_deref(), Some("github.com/gin-gonic/gin"));
    assert!(!gin_import.is_wildcard);

    let fmt_import = ctx.imports.iter().find(|i| i.imported_name == "fmt").unwrap();
    assert_eq!(fmt_import.module_path.as_deref(), Some("fmt"));
}

#[test]
fn test_build_file_context_alias_import() {
    // `import mygin "github.com/gin-gonic/gin"` → target_name = "mygin", module = full path
    let mut file = make_file(
        "main/main.go",
        vec![make_symbol(
            "Run",
            "main.Run",
            SymbolKind::Function,
            Visibility::Public,
            Some("main"),
        )],
        vec![],
    );
    file.refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: "mygin".to_string(),
        kind: EdgeKind::Imports,
        line: 3,
        module: Some("github.com/gin-gonic/gin".to_string()),
        chain: None,
        byte_offset: 0,
    });

    let resolver = GoResolver;
    let ctx = resolver.build_file_context(&file, None);

    assert_eq!(ctx.imports.len(), 1);
    let imp = &ctx.imports[0];
    // imported_name should be the alias
    assert_eq!(imp.imported_name, "mygin");
    assert_eq!(imp.alias.as_deref(), Some("mygin"));
    assert_eq!(imp.module_path.as_deref(), Some("github.com/gin-gonic/gin"));
}

#[test]
fn test_build_file_context_blank_import_skipped() {
    let mut file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![],
    );
    // Blank import: side effects only
    file.refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: "_".to_string(),
        kind: EdgeKind::Imports,
        line: 3,
        module: Some("database/sql/driver".to_string()),
        chain: None,
        byte_offset: 0,
    });

    let resolver = GoResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert!(ctx.imports.is_empty());
}

// ---------------------------------------------------------------------------
// Resolution tests
// ---------------------------------------------------------------------------

#[test]
fn test_same_package_resolution_by_qualified_name() {
    // Two files in the same package. One calls a function from the other.
    let file1 = make_file(
        "handlers/user.go",
        vec![
            make_symbol(
                "UserHandler",
                "handlers.UserHandler",
                SymbolKind::Struct,
                Visibility::Public,
                Some("handlers"),
            ),
            make_symbol(
                "validateUser",
                "handlers.validateUser",
                SymbolKind::Function,
                Visibility::Private,
                Some("handlers"),
            ),
        ],
        vec![],
    );

    let file2 = make_file(
        "handlers/auth.go",
        vec![make_symbol(
            "AuthHandler",
            "handlers.AuthHandler",
            SymbolKind::Struct,
            Visibility::Public,
            Some("handlers"),
        )],
        vec![make_ref(0, "validateUser", EdgeKind::Calls, 15)],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve validateUser in same package");
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    // The scope chain walk (scope_path = "handlers") finds "handlers.validateUser"
    // before the explicit same-package step — all strategies are valid here.
    assert!(
        res.strategy == "go_same_package"
            || res.strategy == "go_same_package_by_name"
            || res.strategy == "go_scope_chain",
        "Expected a same-package resolution strategy, got: {}",
        res.strategy
    );
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("handlers/user.go".to_string(), "handlers.validateUser".to_string()))
            .unwrap()
    );
}

#[test]
fn test_same_package_resolution_method_on_same_receiver() {
    // Method calling sibling method on the same struct via scope chain.
    let file = make_file(
        "server/server.go",
        vec![
            make_symbol(
                "Server",
                "server.Server",
                SymbolKind::Struct,
                Visibility::Public,
                Some("server"),
            ),
            make_symbol(
                "Run",
                "server.Server.Run",
                SymbolKind::Method,
                Visibility::Public,
                Some("server.Server"),
            ),
            make_symbol(
                "init",
                "server.Server.init",
                SymbolKind::Method,
                Visibility::Private,
                Some("server.Server"),
            ),
        ],
        vec![make_ref(1, "init", EdgeKind::Calls, 5)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    // source_symbol = Run, scope_path = "server.Server"
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[1],
        scope_chain: build_scope_chain(file.symbols[1].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve init via scope chain");
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "go_scope_chain");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("server/server.go".to_string(), "server.Server.init".to_string()))
            .unwrap()
    );
}

#[test]
fn test_cross_package_import_resolution() {
    // File imports a package and calls an exported function from it.
    let handlers_file = make_file(
        "handlers/handler.go",
        vec![make_symbol(
            "NewRouter",
            "gin.NewRouter",
            SymbolKind::Function,
            Visibility::Public,
            Some("gin"),
        )],
        vec![],
    );

    let main_file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![
            make_import_ref(0, "gin", "github.com/gin-gonic/gin", 3),
            make_ref(0, "NewRouter", EdgeKind::Calls, 10),
        ],
    );

    let (index, id_map) = build_test_env(&[&handlers_file, &main_file]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&main_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &main_file.refs[1], // NewRouter call, not the import
        source_symbol: &main_file.symbols[0],
        scope_chain: build_scope_chain(main_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve gin.NewRouter via import");
    let res = result.unwrap();
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "go_import");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("handlers/handler.go".to_string(), "gin.NewRouter".to_string()))
            .unwrap()
    );
}

#[test]
fn test_import_alias_resolution() {
    // `import mygin "github.com/gin-gonic/gin"` → code uses `mygin.Default()`
    // Extractor emits target_name = "Default" (just the method name).
    let gin_file = make_file(
        "vendor/gin/gin.go",
        vec![make_symbol(
            "Default",
            "gin.Default",
            SymbolKind::Function,
            Visibility::Public,
            Some("gin"),
        )],
        vec![],
    );

    let mut main_file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![make_ref(0, "Default", EdgeKind::Calls, 10)],
    );
    // Aliased import
    main_file.refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: "mygin".to_string(),
        kind: EdgeKind::Imports,
        line: 3,
        module: Some("github.com/gin-gonic/gin".to_string()),
        chain: None,
        byte_offset: 0,
    });

    let (index, id_map) = build_test_env(&[&gin_file, &main_file]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&main_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &main_file.refs[0], // Default call
        source_symbol: &main_file.symbols[0],
        scope_chain: build_scope_chain(main_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    // Resolution tries last_seg "gin" → "gin.Default" which matches
    assert!(result.is_some(), "Should resolve Default via import (last segment)");
    assert_eq!(result.unwrap().confidence, 1.0);
    let _ = id_map.get(&("vendor/gin/gin.go".to_string(), "gin.Default".to_string())).unwrap();
}

#[test]
fn test_visibility_unexported_same_package() {
    // Unexported function in same directory is visible.
    let file1 = make_file(
        "pkg/util.go",
        vec![make_symbol(
            "helper",
            "pkg.helper",
            SymbolKind::Function,
            Visibility::Private,
            Some("pkg"),
        )],
        vec![],
    );

    let file2 = make_file(
        "pkg/main.go",
        vec![make_symbol(
            "Run",
            "pkg.Run",
            SymbolKind::Function,
            Visibility::Public,
            Some("pkg"),
        )],
        vec![make_ref(0, "helper", EdgeKind::Calls, 5)],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Unexported symbol should be visible in same package");
    assert_eq!(
        result.unwrap().target_symbol_id,
        *id_map
            .get(&("pkg/util.go".to_string(), "pkg.helper".to_string()))
            .unwrap()
    );
}

#[test]
fn test_visibility_unexported_cross_package_not_visible() {
    // Unexported function in a different directory must not resolve.
    let other_file = make_file(
        "internal/util.go",
        vec![make_symbol(
            "helper",
            "internal.helper",
            SymbolKind::Function,
            Visibility::Private,
            Some("internal"),
        )],
        vec![],
    );

    let caller_file = make_file(
        "cmd/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![
            make_import_ref(0, "internal", "example.com/app/internal", 3),
            make_ref(0, "helper", EdgeKind::Calls, 10),
        ],
    );

    let (index, _) = build_test_env(&[&other_file, &caller_file]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&caller_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &caller_file.refs[1], // helper call
        source_symbol: &caller_file.symbols[0],
        scope_chain: build_scope_chain(caller_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    // The "internal" package has a symbol named "helper" but it's Private.
    // Cross-directory access to a private symbol should fail.
    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_none(),
        "Private cross-package symbol should not resolve"
    );
}

#[test]
fn test_import_ref_skipped_in_resolve() {
    // Import refs should never be resolved — they ARE the declarations.
    let file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![make_import_ref(0, "fmt", "fmt", 1)],
    );

    let (index, _) = build_test_env(&[&file]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0], // the import ref
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_none(), "Import refs should be skipped");
}

#[test]
fn test_falls_back_for_unknown() {
    let file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![make_ref(0, "NonExistentFunc", EdgeKind::Calls, 5)],
    );

    let (index, _) = build_test_env(&[&file]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Unknown symbol should fall back to heuristic"
    );
}

// ---------------------------------------------------------------------------
// infer_external_namespace tests
// ---------------------------------------------------------------------------

#[test]
fn test_infer_external_namespace_exported_symbol() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut go_mod = ManifestData::default();
    go_mod.module_path = Some("code.gitea.io/gitea".to_string());
    ctx.manifests.insert(ManifestKind::GoMod, go_mod);

    let file = make_file(
        "modules/log/log.go",
        vec![make_symbol(
            "Logger",
            "log.Logger",
            SymbolKind::Struct,
            Visibility::Public,
            Some("log"),
        )],
        vec![
            make_import_ref(0, "zap", "go.uber.org/zap", 3),
            make_ref(0, "NewLogger", EdgeKind::Calls, 10),
        ],
    );

    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[1], // NewLogger call
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert!(ns.is_some(), "Exported symbol with external import should be inferred");
    assert_eq!(ns.unwrap(), "go.uber.org/zap");
}

#[test]
fn test_infer_external_namespace_unexported_returns_none() {
    // Unexported names can't come from external packages.
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut go_mod = ManifestData::default();
    go_mod.module_path = Some("example.com/app".to_string());
    ctx.manifests.insert(ManifestKind::GoMod, go_mod);

    let file = make_file(
        "cmd/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![
            make_import_ref(0, "gin", "github.com/gin-gonic/gin", 3),
            make_ref(0, "unexportedHelper", EdgeKind::Calls, 10),
        ],
    );

    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[1], // unexportedHelper call
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert!(
        ns.is_none(),
        "Unexported symbols cannot come from external packages"
    );
}

#[test]
fn test_infer_external_namespace_internal_import_not_returned() {
    // An import that is internal (starts with the project module path) should not be
    // returned as external namespace.
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut go_mod = ManifestData::default();
    go_mod.module_path = Some("code.gitea.io/gitea".to_string());
    ctx.manifests.insert(ManifestKind::GoMod, go_mod);

    let file = make_file(
        "routers/web/web.go",
        vec![make_symbol(
            "Routes",
            "web.Routes",
            SymbolKind::Function,
            Visibility::Public,
            Some("web"),
        )],
        vec![
            // Internal import: same module
            make_import_ref(0, "log", "code.gitea.io/gitea/modules/log", 3),
            make_ref(0, "NewLogger", EdgeKind::Calls, 10),
        ],
    );

    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[1], // NewLogger call
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert!(
        ns.is_none(),
        "Internal import should not be returned as external namespace"
    );
}

#[test]
fn test_infer_no_imports_returns_none() {
    let file = make_file(
        "pkg/simple.go",
        vec![make_symbol(
            "Foo",
            "pkg.Foo",
            SymbolKind::Function,
            Visibility::Public,
            Some("pkg"),
        )],
        vec![make_ref(0, "Bar", EdgeKind::Calls, 5)],
    );

    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert!(ns.is_none(), "No imports → no external namespace inference");
}

#[test]
fn test_infer_external_namespace_import_ref_skipped() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut go_mod = ManifestData::default();
    go_mod.module_path = Some("example.com/app".to_string());
    ctx.manifests.insert(ManifestKind::GoMod, go_mod);

    let file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![make_import_ref(0, "gin", "github.com/gin-gonic/gin", 3)],
    );

    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0], // the import ref itself
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    // Import refs to external packages should now be classified as external.
    assert_eq!(
        ns.as_deref(),
        Some("github.com/gin-gonic/gin"),
        "External import refs should return the import path"
    );
}

// ---------------------------------------------------------------------------
// is_visible tests
// ---------------------------------------------------------------------------

#[test]
fn test_is_visible_public_always() {
    let file_ctx = FileContext {
        file_path: "pkg/a.go".to_string(),
        language: "go".to_string(),
        imports: vec![],
        file_namespace: Some("pkg".to_string()),
    };

    let sym = SymbolInfo {
        id: 1,
        name: "Exported".to_string(),
        qualified_name: "other.Exported".to_string(),
        kind: "function".to_string(),
        visibility: Some("public".to_string()),
        file_path: Arc::from("other/b.go"),
        scope_path: Some("other".to_string()),
        package_id: None,
    };

    // Dummy ref_ctx (not used by is_visible for public symbols)
    let sym_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Exported".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
    };
    let source_sym = make_symbol("Run", "pkg.Run", SymbolKind::Function, Visibility::Public, Some("pkg"));
    let ref_ctx = RefContext {
        extracted_ref: &sym_ref,
        source_symbol: &source_sym,
        scope_chain: vec![],
    file_package_id: None,
    };

    let resolver = GoResolver;
    assert!(resolver.is_visible(&file_ctx, &ref_ctx, &sym));
}

#[test]
fn test_is_visible_private_same_dir() {
    let file_ctx = FileContext {
        file_path: "pkg/a.go".to_string(),
        language: "go".to_string(),
        imports: vec![],
        file_namespace: Some("pkg".to_string()),
    };

    let sym = SymbolInfo {
        id: 2,
        name: "unexported".to_string(),
        qualified_name: "pkg.unexported".to_string(),
        kind: "function".to_string(),
        visibility: Some("private".to_string()),
        file_path: Arc::from("pkg/b.go"), // same directory
        scope_path: Some("pkg".to_string()),
        package_id: None,
    };

    let sym_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "unexported".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
    };
    let source_sym = make_symbol("Run", "pkg.Run", SymbolKind::Function, Visibility::Public, Some("pkg"));
    let ref_ctx = RefContext {
        extracted_ref: &sym_ref,
        source_symbol: &source_sym,
        scope_chain: vec![],
    file_package_id: None,
    };

    let resolver = GoResolver;
    assert!(resolver.is_visible(&file_ctx, &ref_ctx, &sym), "Same dir private should be visible");
}

#[test]
fn test_is_visible_private_different_dir() {
    let file_ctx = FileContext {
        file_path: "cmd/main.go".to_string(),
        language: "go".to_string(),
        imports: vec![],
        file_namespace: Some("main".to_string()),
    };

    let sym = SymbolInfo {
        id: 3,
        name: "unexported".to_string(),
        qualified_name: "pkg.unexported".to_string(),
        kind: "function".to_string(),
        visibility: Some("private".to_string()),
        file_path: Arc::from("pkg/b.go"), // different directory
        scope_path: Some("pkg".to_string()),
        package_id: None,
    };

    let sym_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "unexported".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
    };
    let source_sym = make_symbol("main", "main.main", SymbolKind::Function, Visibility::Private, Some("main"));
    let ref_ctx = RefContext {
        extracted_ref: &sym_ref,
        source_symbol: &source_sym,
        scope_chain: vec![],
    file_package_id: None,
    };

    let resolver = GoResolver;
    assert!(
        !resolver.is_visible(&file_ctx, &ref_ctx, &sym),
        "Private cross-dir should not be visible"
    );
}

#[test]
fn test_instantiates_ref_resolution() {
    // Composite literal `handlers.UserHandler{...}` → target_name = "UserHandler"
    let handler_file = make_file(
        "handlers/user.go",
        vec![make_symbol(
            "UserHandler",
            "handlers.UserHandler",
            SymbolKind::Struct,
            Visibility::Public,
            Some("handlers"),
        )],
        vec![],
    );

    let main_file = make_file(
        "main/main.go",
        vec![make_symbol(
            "main",
            "main.main",
            SymbolKind::Function,
            Visibility::Private,
            Some("main"),
        )],
        vec![
            make_import_ref(0, "handlers", "example.com/app/handlers", 3),
            ExtractedRef {
                source_symbol_index: 0,
                target_name: "UserHandler".to_string(),
                kind: EdgeKind::Instantiates,
                line: 10,
                module: None,
                chain: None,
                byte_offset: 0,
            },
        ],
    );

    let (index, id_map) = build_test_env(&[&handler_file, &main_file]);
    let resolver = GoResolver;
    let file_ctx = resolver.build_file_context(&main_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &main_file.refs[1], // Instantiates
        source_symbol: &main_file.symbols[0],
        scope_chain: build_scope_chain(main_file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve UserHandler struct via import");
    assert_eq!(
        result.unwrap().target_symbol_id,
        *id_map
            .get(&("handlers/user.go".to_string(), "handlers.UserHandler".to_string()))
            .unwrap()
    );
}
