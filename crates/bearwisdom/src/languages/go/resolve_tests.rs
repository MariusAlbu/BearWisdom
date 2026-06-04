// =============================================================================
// go/resolve_tests.rs — go.mod parsing, external classification, file-context
// construction, alias-target synthesis, and flow-emission detector tests.
// Resolution itself runs through the generic engine (see
// type_checker/core/{chain,default_resolver}_tests.rs).
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, ImportEntry, RefContext, SymbolIndex,
};
use crate::types::*;
use std::collections::HashMap;

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
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
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

fn make_import_ref(
    source_idx: usize,
    last_segment: &str,
    full_path: &str,
    line: u32,
) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: source_idx,
        target_name: last_segment.to_string(),
        kind: EdgeKind::Imports,
        line,
        module: Some(full_path.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
            col: 0,
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
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }
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
    assert!(!super::hooks::is_manifest_go_external(&ctx,"code.gitea.io/gitea"));
    // Internal: sub-package
    assert!(!super::hooks::is_manifest_go_external(&ctx,"code.gitea.io/gitea/modules/log"));
    assert!(!super::hooks::is_manifest_go_external(&ctx,"code.gitea.io/gitea/services/auth"));
    // External: different host
    assert!(super::hooks::is_manifest_go_external(&ctx,"github.com/gin-gonic/gin"));
    assert!(super::hooks::is_manifest_go_external(&ctx,"golang.org/x/crypto"));
    // External: standard library is internal by our heuristic but shouldn't matter —
    // stdlib won't be in the index anyway
    assert!(super::hooks::is_manifest_go_external(&ctx,"fmt")); // no dot → external per module-path logic
}

#[test]
fn test_is_external_go_import_no_module_path_fallback() {
    let ctx = ProjectContext::default(); // no go_module_path

    // Heuristic: dot in first segment → external
    assert!(super::hooks::is_manifest_go_external(&ctx,"github.com/gin-gonic/gin"));
    assert!(super::hooks::is_manifest_go_external(&ctx,"golang.org/x/net"));
    // Standard library: no dot → not external
    assert!(!super::hooks::is_manifest_go_external(&ctx,"fmt"));
    assert!(!super::hooks::is_manifest_go_external(&ctx,"net/http"));
    assert!(!super::hooks::is_manifest_go_external(&ctx,"encoding/json"));
}

#[test]
fn test_is_external_go_import_prefix_boundary() {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut go_mod = ManifestData::default();
    go_mod.module_path = Some("github.com/myorg/myrepo".to_string());
    ctx.manifests.insert(ManifestKind::GoMod, go_mod);

    // "github.com/myorg/myrepox" must NOT be treated as internal
    assert!(super::hooks::is_manifest_go_external(&ctx,"github.com/myorg/myrepox"));
    // Sub-packages are internal
    assert!(!super::hooks::is_manifest_go_external(&ctx,"github.com/myorg/myrepo/pkg/api"));
}

// ---------------------------------------------------------------------------
// build_file_context tests
// ---------------------------------------------------------------------------

use super::hooks::build_file_context_inner;

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

    let ctx = build_file_context_inner(&file, None);

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

    let ctx = build_file_context_inner(&file, None);

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
    file.refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "mygin".to_string(),
        kind: EdgeKind::Imports,
        line: 3,
        col: 0,
        module: Some("github.com/gin-gonic/gin".to_string()),
        chain: None,
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});

    let ctx = build_file_context_inner(&file, None);

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
    file.refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "_".to_string(),
        kind: EdgeKind::Imports,
        line: 3,
        col: 0,
        module: Some("database/sql/driver".to_string()),
        chain: None,
        byte_offset: 1,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});

    let ctx = build_file_context_inner(&file, None);
    assert!(ctx.imports.is_empty());
}

// ---------------------------------------------------------------------------
// external-classification tests (GoHooks::classify_external)
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

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[1], // NewLogger call
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::go::hooks::GoHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
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

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[1], // unexportedHelper call
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::go::hooks::GoHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
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

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[1], // NewLogger call
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::go::hooks::GoHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
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

    let file_ctx = build_file_context_inner(&file, None);
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::go::hooks::GoHooks.classify_external(
            &ref_ctx, &file_ctx, None, &empty_lookup,
        )
    };
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

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0], // the import ref itself
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::go::hooks::GoHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    // Import refs to external packages should be classified as external.
    assert_eq!(
        ns.as_deref(),
        Some("github.com/gin-gonic/gin"),
        "External import refs should return the import path"
    );
}

// ---------------------------------------------------------------------------
// HTTP Producer + DbQuery + gRPC flow detection
// ---------------------------------------------------------------------------

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: if i == 0 {
                    "identifier".to_string()
                } else {
                    "field_identifier".to_string()
                },
                kind: if i == 0 {
                    SegmentKind::Identifier
                } else {
                    SegmentKind::Property
                },
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
})
            .collect(),
    }
}

#[test]
fn test_go_http_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_go_http_chain_emission;

    let chain = make_chain(&["http", "Get"]);
    let call_args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_go_http_chain_emission(&chain, &call_args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "/api/users");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        other => panic!("expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_go_http_new_request_uses_method_arg() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_go_http_chain_emission;

    let chain = make_chain(&["http", "NewRequest"]);
    let call_args = vec![
        CallArg::StringLit("POST".to_string()),
        CallArg::StringLit("/api/login".to_string()),
        CallArg::Ident("body".to_string()),
    ];
    match detect_go_http_chain_emission(&chain, &call_args).unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Post));
            assert_eq!(name, "/api/login");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_resty_client_chain_emits_producer() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_go_http_chain_emission;

    let chain = make_chain(&["client", "R", "Get"]);
    let call_args = vec![CallArg::StringLit("/api/x".to_string())];
    match detect_go_http_chain_emission(&chain, &call_args).unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/api/x");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_http_no_emit_when_url_is_not_literal() {
    use super::hooks::detect_go_http_chain_emission;

    let chain = make_chain(&["http", "Get"]);
    let call_args = vec![CallArg::Ident("url".to_string())];
    assert!(detect_go_http_chain_emission(&chain, &call_args).is_none());
}

#[test]
fn test_go_db_query_parses_sql_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_go_db_query_emission;

    let chain = make_chain(&["db", "Query"]);
    let call_args = vec![
        CallArg::StringLit("SELECT * FROM users WHERE id = $1".to_string()),
    ];
    match detect_go_db_query_emission(&chain, &call_args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "go.users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_go_db_exec_parses_sql_update() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_go_db_query_emission;

    let chain = make_chain(&["db", "Exec"]);
    let call_args = vec![
        CallArg::StringLit("UPDATE accounts SET balance = $1 WHERE id = $2".to_string()),
    ];
    match detect_go_db_query_emission(&chain, &call_args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "go.accounts");
            assert_eq!(operation, DbQueryOp::Update);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_go_gorm_first_emits_dbquery_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_go_db_query_emission;

    let chain = make_chain(&["db", "First"]);
    let call_args = vec![CallArg::Ident("User".to_string())];
    match detect_go_db_query_emission(&chain, &call_args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "go.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_go_gorm_create_emits_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_go_db_query_emission;

    let chain = make_chain(&["db", "Create"]);
    let call_args = vec![CallArg::Ident("Poll".to_string())];
    match detect_go_db_query_emission(&chain, &call_args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "go.Poll");
            assert_eq!(operation, DbQueryOp::Insert);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_go_db_no_emit_for_unknown_leaf() {
    use super::hooks::detect_go_db_query_emission;

    let chain = make_chain(&["db", "Ping"]);
    let call_args: Vec<CallArg> = vec![];
    assert!(detect_go_db_query_emission(&chain, &call_args).is_none());
}

#[test]
fn test_go_grpc_three_segment_chain_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_grpc_chain_emission;

    let file_ctx = FileContext {
        file_path: "client.go".to_string(),
        language: "go".to_string(),
        imports: vec![ImportEntry {
            imported_name: "pb".to_string(),
            module_path: Some("github.com/example/api/proto/userpb".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let chain = make_chain(&["client", "UserService", "GetUser"]);
    match detect_go_grpc_chain_emission(&chain, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::RpcCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "user/getuser");
        }
        _ => panic!("expected NamedChannel RpcCall"),
    }
}

#[test]
fn test_go_grpc_two_segment_client_chain_emits_producer() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_go_grpc_chain_emission;

    let file_ctx = FileContext {
        file_path: "client.go".to_string(),
        language: "go".to_string(),
        imports: vec![ImportEntry {
            imported_name: "userpb".to_string(),
            module_path: Some("github.com/example/api/proto/userpb".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let chain = make_chain(&["UserServiceClient", "GetUser"]);
    match detect_go_grpc_chain_emission(&chain, &file_ctx).unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "user/getuser"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_grpc_no_emit_without_proto_import() {
    use super::hooks::detect_go_grpc_chain_emission;

    let file_ctx = FileContext {
        file_path: "client.go".to_string(),
        language: "go".to_string(),
        imports: vec![ImportEntry {
            imported_name: "fmt".to_string(),
            module_path: Some("fmt".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let chain = make_chain(&["client", "UserService", "GetUser"]);
    assert!(detect_go_grpc_chain_emission(&chain, &file_ctx).is_none());
}

// ---------------------------------------------------------------------------
// Mailer / BgJob / MQ / Redis / UDS detectors
// ---------------------------------------------------------------------------

#[test]
fn test_go_smtp_send_mail_emits_mailer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_mailer_emission;
    let chain = make_chain(&["smtp", "SendMail"]);
    match detect_go_mailer_emission(&chain, &[]).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::Mailer);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "go.smtp");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_gomail_dial_and_send_emits_mailer() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_mailer_emission;
    let chain = make_chain(&["d", "DialAndSend"]);
    match detect_go_mailer_emission(&chain, &[]).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert_eq!(kind, NamedChannelKind::Mailer);
            assert_eq!(name, "go.gomail");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_asynq_enqueue_emits_bgjob() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_bgjob_emission;
    let chain = make_chain(&["client", "Enqueue"]);
    match detect_go_bgjob_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert_eq!(kind, NamedChannelKind::BgJob);
            assert_eq!(name, "go.asynq");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_kafka_send_message_emits_mq() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_mq_emission;
    let chain = make_chain(&["producer", "SendMessage"]);
    match detect_go_mq_emission(&chain, &[]).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(name, "go.kafka");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_nats_publish_captures_subject() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_mq_emission;
    let chain = make_chain(&["nc", "Publish"]);
    let args = vec![
        CallArg::StringLit("orders.created".to_string()),
        CallArg::Ident("data".to_string()),
    ];
    match detect_go_mq_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert_eq!(kind, NamedChannelKind::MessageQueue);
            assert_eq!(name, "orders.created");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_redis_get_emits_config_lookup() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_go_redis_config_lookup;
    let chain = make_chain(&["rdb", "Get"]);
    let args = vec![
        CallArg::Ident("ctx".to_string()),
        CallArg::StringLit("feature:enabled".to_string()),
    ];
    match detect_go_redis_config_lookup(&chain, &args).unwrap() {
        FlowEmission::ConfigLookup { key } => assert_eq!(key, "redis:feature:enabled"),
        _ => panic!("expected ConfigLookup"),
    }
}

#[test]
fn test_go_uds_listen_emits_ipc_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_uds_emission;
    let chain = make_chain(&["net", "Listen"]);
    let args = vec![
        CallArg::StringLit("unix".to_string()),
        CallArg::StringLit("/tmp/app.sock".to_string()),
    ];
    match detect_go_uds_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert_eq!(kind, NamedChannelKind::IpcCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/tmp/app.sock");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_uds_dial_emits_ipc_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    use super::hooks::detect_go_uds_emission;
    let chain = make_chain(&["net", "Dial"]);
    let args = vec![
        CallArg::StringLit("unix".to_string()),
        CallArg::StringLit("/tmp/app.sock".to_string()),
    ];
    match detect_go_uds_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_uds_rejects_tcp_network() {
    use super::hooks::detect_go_uds_emission;
    let chain = make_chain(&["net", "Listen"]);
    let args = vec![
        CallArg::StringLit("tcp".to_string()),
        CallArg::StringLit(":8080".to_string()),
    ];
    assert!(detect_go_uds_emission(&chain, &args).is_none());
}

#[test]
fn test_go_gorilla_upgrader_emits_ws_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_go_gorilla_ws_consumer;
    let chain = make_chain(&["upgrader", "Upgrade"]);
    match detect_go_gorilla_ws_consumer(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "go.gorilla.ws");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_go_nhooyr_accept_emits_ws_consumer() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_go_gorilla_ws_consumer;
    let chain = make_chain(&["websocket", "Accept"]);
    match detect_go_gorilla_ws_consumer(&chain).unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "go.nhooyr.ws"),
        _ => panic!("expected NamedChannel"),
    }
}

// ---------------------------------------------------------------------------
// Alias-target synthesis: TRUE alias vs DEFINED type (end-to-end)
//
// These build a real SymbolIndex from extracted Go source — which runs the
// build.rs `field_type → AliasTarget` synthesis — and assert which type names
// become expandable aliases. A Go defined type must NOT (alias expansion would
// rewrite its receiver to the underlying type and drop its own method set); a
// Go true alias (`=` form) MUST (it shares the target's members).
// ---------------------------------------------------------------------------

/// Build a `SymbolIndex` from Go source by running the real extractor, so the
/// build.rs alias-target synthesis fires on the extracted refs.
fn index_from_go_source(path: &str, source: &str) -> SymbolIndex {
    let res = super::extract::extract(source);
    let pf = make_file(path, res.symbols, res.refs);
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for sym in &pf.symbols {
        id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
        next_id += 1;
    }
    SymbolIndex::build(&[pf], &id_map)
}

#[test]
fn composite_defined_type_synthesizes_no_alias_target() {
    // (a) `type Stack []Item`: `s.Push()` must NOT resolve to `Item.Push`. The
    // engine only mis-resolves when `Stack` carries an expandable AliasTarget;
    // the fix is that the composite defined type synthesizes none, so expansion
    // stays a no-op and the receiver keeps its own type.
    use crate::indexer::resolve::engine::SymbolLookup;
    let index = index_from_go_source(
        "coll/stack.go",
        r#"package coll

type Item struct{ V int }

func (i Item) Push() {}

type Stack []Item
"#,
    );
    assert!(
        index.alias_target("Stack").is_none(),
        "composite defined type Stack must have no AliasTarget (would mis-resolve s.Push to Item.Push)"
    );
    assert!(
        index.alias_target("coll.Stack").is_none(),
        "composite defined type coll.Stack must have no AliasTarget"
    );
}

#[test]
fn defined_type_with_method_synthesizes_no_alias_target() {
    // (b) `type Foo Bar` with `func (f Foo) M()`: `f.M()` must resolve to
    // `Foo.M`, NOT be rewritten to `Bar`. A defined type carries no expandable
    // AliasTarget, so the receiver `Foo` is never rewritten away from itself.
    use crate::indexer::resolve::engine::SymbolLookup;
    let index = index_from_go_source(
        "m/foo.go",
        r#"package m

type Bar struct{ X int }

type Foo Bar

func (f Foo) M() {}
"#,
    );
    assert!(
        index.alias_target("Foo").is_none(),
        "defined type Foo must have no AliasTarget (would rewrite f.M to Bar.M)"
    );
    // Its own method is recorded under qname Foo.M, ready to resolve against Foo.
    let foo_m = index.by_qualified_name("m.Foo.M");
    assert!(foo_m.is_some(), "expected method m.Foo.M to be indexed");
}

#[test]
fn true_alias_synthesizes_expandable_alias_target() {
    // (c) `type Alias = Bar`: a value typed `Alias` resolves members through
    // `Bar`. The true alias synthesizes an expandable AliasTarget naming Bar,
    // which alias expansion rewrites to before member lookup.
    use crate::indexer::resolve::engine::SymbolLookup;
    let index = index_from_go_source(
        "m/alias.go",
        r#"package m

type Bar struct{ X int }

func (b Bar) Member() {}

type Alias = Bar
"#,
    );
    let target = index.alias_target("Alias");
    assert!(
        target.is_some(),
        "true alias Alias must synthesize an expandable AliasTarget naming Bar"
    );
    match target.unwrap() {
        AliasTarget::Application { root, .. } => {
            assert_eq!(root, "Bar", "alias root must be Bar");
        }
        other => panic!("expected Application{{root: Bar}}, got {other:?}"),
    }
}
