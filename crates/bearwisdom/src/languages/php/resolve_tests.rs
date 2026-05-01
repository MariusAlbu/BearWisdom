use super::resolve::PhpResolver;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex, SymbolInfo};
use crate::types::*;
use std::collections::HashMap;
use super::resolve::normalize_php_ns;

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

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}
fn make_use(source_idx: usize, alias: &str, fqn: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: alias.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: Some(fqn.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}
fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "php".to_string(),
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
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
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
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_scope_chain_resolution() {
    let file = make_file(
        "app/Controllers/UserController.php",
        vec![
            make_symbol("App.Controllers", "App.Controllers", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("UserController", "App.Controllers.UserController", SymbolKind::Class, Visibility::Public, Some("App.Controllers")),
            make_symbol("store", "App.Controllers.UserController.store", SymbolKind::Method, Visibility::Public, Some("App.Controllers.UserController")),
            make_symbol("validate", "App.Controllers.UserController.validate", SymbolKind::Method, Visibility::Private, Some("App.Controllers.UserController")),
        ],
        vec![make_ref(2, "validate", EdgeKind::Calls)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[2],
        scope_chain: build_scope_chain(file.symbols[2].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve validate via scope chain");
    let res = result.unwrap();
    assert_eq!(res.strategy, "php_scope_chain");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("app/Controllers/UserController.php".to_string(), "App.Controllers.UserController.validate".to_string())).unwrap()
    );
}

#[test]
fn test_same_namespace_resolution() {
    let file1 = make_file(
        "app/Models/User.php",
        vec![
            make_symbol("App.Models", "App.Models", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("User", "App.Models.User", SymbolKind::Class, Visibility::Public, Some("App.Models")),
        ],
        vec![],
    );

    let file2 = make_file(
        "app/Models/Post.php",
        vec![
            make_symbol("App.Models", "App.Models", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("Post", "App.Models.Post", SymbolKind::Class, Visibility::Public, Some("App.Models")),
        ],
        // No use statement — same namespace
        vec![make_ref(1, "User", EdgeKind::TypeRef)],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[1],
        scope_chain: build_scope_chain(file2.symbols[1].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve User via same namespace");
    let res = result.unwrap();
    assert!(
        res.strategy == "php_scope_chain" || res.strategy == "php_same_namespace",
        "Unexpected strategy: {}",
        res.strategy
    );
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("app/Models/User.php".to_string(), "App.Models.User".to_string())).unwrap()
    );
}

#[test]
fn test_use_statement_resolution() {
    let file1 = make_file(
        "app/Models/Product.php",
        vec![make_symbol("Product", "App.Models.Product", SymbolKind::Class, Visibility::Public, Some("App.Models"))],
        vec![],
    );

    let file2 = make_file(
        "app/Http/Controllers/ProductController.php",
        vec![make_symbol("ProductController", "App.Http.Controllers.ProductController", SymbolKind::Class, Visibility::Public, Some("App.Http.Controllers"))],
        vec![
            make_ref(0, "Product", EdgeKind::TypeRef),
            // use App\Models\Product; — stored with backslash by extractor
            make_use(0, "Product", "App\\Models\\Product"),
        ],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve Product via use statement");
    let res = result.unwrap();
    assert_eq!(res.strategy, "php_use_statement");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("app/Models/Product.php".to_string(), "App.Models.Product".to_string())).unwrap()
    );
}

#[test]
fn test_use_statement_alias() {
    let file1 = make_file(
        "app/Models/User.php",
        vec![make_symbol("User", "App.Models.User", SymbolKind::Class, Visibility::Public, Some("App.Models"))],
        vec![],
    );

    let file2 = make_file(
        "app/Services/UserService.php",
        vec![make_symbol("UserService", "App.Services.UserService", SymbolKind::Class, Visibility::Public, Some("App.Services"))],
        vec![
            make_ref(0, "UserModel", EdgeKind::TypeRef),
            // use App\Models\User as UserModel;
            make_use(0, "UserModel", "App\\Models\\User"),
        ],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve alias via use statement");
    let res = result.unwrap();
    assert_eq!(res.strategy, "php_use_statement");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("app/Models/User.php".to_string(), "App.Models.User".to_string())).unwrap()
    );
}

#[test]
fn test_private_cross_file_not_resolved() {
    let file1 = make_file(
        "app/Models/Order.php",
        vec![make_symbol("secret", "App.Models.Order.secret", SymbolKind::Method, Visibility::Private, Some("App.Models.Order"))],
        vec![],
    );

    let file2 = make_file(
        "app/Services/OrderService.php",
        vec![make_symbol("OrderService", "App.Services.OrderService", SymbolKind::Class, Visibility::Public, Some("App.Services"))],
        vec![
            make_ref(0, "secret", EdgeKind::Calls),
            make_use(0, "Order", "App\\Models\\Order"),
        ],
    );

    let (index, _) = build_test_env(&[&file1, &file2]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Private cross-file should not resolve"
    );
}

#[test]
fn test_falls_back_for_unknown() {
    let file = make_file(
        "app/Foo.php",
        vec![make_symbol("Foo", "App.Foo", SymbolKind::Class, Visibility::Public, Some("App"))],
        vec![make_ref(0, "NonExistent", EdgeKind::TypeRef)],
    );

    let (index, _) = build_test_env(&[&file]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Unknown should fall back"
    );
}

#[test]
fn test_normalize_php_ns() {
    assert_eq!(normalize_php_ns("App\\Models\\User"), "App.Models.User");
    assert_eq!(normalize_php_ns("\\App\\Models\\User"), "App.Models.User");
    assert_eq!(normalize_php_ns("App.Models.User"), "App.Models.User");
    assert_eq!(normalize_php_ns("Foo"), "Foo");
}

#[test]
fn test_infer_framework_external() {
    let file = make_file(
        "app/Controllers/Foo.php",
        vec![make_symbol("Foo", "App.Foo", SymbolKind::Class, Visibility::Public, Some("App"))],
        vec![make_use(0, "Controller", "Illuminate\\Routing\\Controller")],
    );

    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert!(ns.is_some(), "Illuminate import should be inferred as external");
}

#[test]
fn test_infer_builtin_external() {
    let file = make_file(
        "app/Foo.php",
        vec![make_symbol("Foo", "App.Foo", SymbolKind::Class, Visibility::Public, Some("App"))],
        vec![make_ref(0, "array_map", EdgeKind::Calls)],
    );

    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("php_core".to_string()));
}

#[test]
fn test_build_file_context_extracts_namespace() {
    let file = make_file(
        "app/Models/User.php",
        vec![
            make_symbol("App.Models", "App.Models", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("User", "App.Models.User", SymbolKind::Class, Visibility::Public, Some("App.Models")),
        ],
        vec![],
    );

    let resolver = PhpResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert_eq!(ctx.file_namespace, Some("App.Models".to_string()));
}

#[test]
fn test_build_file_context_normalizes_backslash() {
    let file = make_file(
        "app/Controllers/Foo.php",
        vec![make_symbol("Foo", "App.Foo", SymbolKind::Class, Visibility::Public, Some("App"))],
        // use App\Models\User;
        vec![make_use(0, "User", "App\\Models\\User")],
    );

    let resolver = PhpResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert_eq!(ctx.imports.len(), 1);
    // Module path should be normalized to dotted form.
    assert_eq!(ctx.imports[0].module_path.as_deref(), Some("App.Models.User"));
    assert_eq!(ctx.imports[0].imported_name, "User");
}

/// `$this->account()` in a service that *extends* a base class should resolve
/// via the inheritance walk (Step 6) even though `account` is not defined on
/// the calling class itself.
#[test]
fn test_inherited_method_via_this_resolves() {
    // BaseService defines `account()`.
    let base_file = make_file(
        "app/Services/BaseService.php",
        vec![
            make_symbol("App.Services", "App.Services", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("BaseService", "App.Services.BaseService", SymbolKind::Class, Visibility::Public, Some("App.Services")),
            make_symbol("account", "App.Services.BaseService.account", SymbolKind::Method, Visibility::Public, Some("App.Services.BaseService")),
        ],
        vec![],
    );

    // SetupAccount extends BaseService; execute() calls $this->account().
    let child_file = make_file(
        "app/Domains/Settings/Services/SetupAccount.php",
        vec![
            make_symbol("App.Domains.Settings.Services", "App.Domains.Settings.Services", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("SetupAccount", "App.Domains.Settings.Services.SetupAccount", SymbolKind::Class, Visibility::Public, Some("App.Domains.Settings.Services")),
            make_symbol("execute", "App.Domains.Settings.Services.SetupAccount.execute", SymbolKind::Method, Visibility::Public, Some("App.Domains.Settings.Services.SetupAccount")),
        ],
        vec![
            // class SetupAccount extends BaseService
            ExtractedRef {
                source_symbol_index: 1, // SetupAccount
                target_name: "BaseService".to_string(),
                kind: EdgeKind::Inherits,
                line: 5,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
            // $this->account() inside execute()
            ExtractedRef {
                source_symbol_index: 2, // execute
                target_name: "$this->account".to_string(),
                kind: EdgeKind::Calls,
                line: 20,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
        ],
    );

    let (index, id_map) = build_test_env(&[&base_file, &child_file]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&child_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &child_file.refs[1], // $this->account
        source_symbol: &child_file.symbols[2], // execute
        scope_chain: build_scope_chain(child_file.symbols[2].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve $this->account via inheritance walk");
    let res = result.unwrap();
    assert_eq!(res.strategy, "php_inherited_method");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "app/Services/BaseService.php".to_string(),
                "App.Services.BaseService.account".to_string()
            ))
            .unwrap()
    );
}

/// `File::whereIn(...)` — static Eloquent call.  The chain root is TypeAccess("File").
/// Phase 1 must resolve TypeAccess to the class name, then the inheritance walk finds
/// the Builder method on the parent Model stub.
#[test]
fn test_static_eloquent_call_via_type_access() {
    // External Model class with whereIn — fixture for the chain walker's
    // TypeAccess root + inheritance walk. Path string is just an identifier;
    // the test doesn't depend on any specific ecosystem.
    let model_stub = make_file(
        "ext:test-fixture:eloquent/ModelForwarded.php",
        vec![
            make_symbol("Model", "Model", SymbolKind::Class, Visibility::Public, None),
            make_symbol("whereIn", "Model.whereIn", SymbolKind::Method, Visibility::Public, Some("Model")),
        ],
        vec![],
    );

    // File model class (extends Model)
    let file_model = make_file(
        "app/Models/File.php",
        vec![
            make_symbol("App.Models", "App.Models", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("File", "App.Models.File", SymbolKind::Class, Visibility::Public, Some("App.Models")),
        ],
        vec![
            // class File extends Model
            ExtractedRef {
                source_symbol_index: 1, // File class
                target_name: "Model".to_string(),
                kind: EdgeKind::Inherits,
                line: 5,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
        ],
    );

    // Service that calls File::whereIn(...)
    let service_file = make_file(
        "app/Services/StorageService.php",
        vec![
            make_symbol("StorageService", "App.Services.StorageService", SymbolKind::Class, Visibility::Public, Some("App.Services")),
            make_symbol("data", "App.Services.StorageService.data", SymbolKind::Method, Visibility::Public, Some("App.Services.StorageService")),
        ],
        vec![
            // File::whereIn('vault_id', $ids)
            ExtractedRef {
                source_symbol_index: 1,
                target_name: "whereIn".to_string(),
                kind: EdgeKind::Calls,
                line: 15,
                module: None,
                chain: Some(MemberChain {
                    segments: vec![
                        ChainSegment {
                            name: "File".to_string(),
                            node_kind: "class".to_string(),
                            kind: SegmentKind::TypeAccess,
                            declared_type: None,
                            type_args: vec![],
                            optional_chaining: false,
                        },
                        ChainSegment {
                            name: "whereIn".to_string(),
                            node_kind: "static_call_expression".to_string(),
                            kind: SegmentKind::Property,
                            declared_type: None,
                            type_args: vec![],
                            optional_chaining: false,
                        },
                    ],
                }),
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
        ],
    );

    let (index, id_map) = build_test_env(&[&model_stub, &file_model, &service_file]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&service_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &service_file.refs[0],
        source_symbol: &service_file.symbols[1], // data
        scope_chain: build_scope_chain(service_file.symbols[1].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        result.is_some(),
        "File::whereIn() must resolve via TypeAccess root + chain inheritance walk"
    );
    let res = result.unwrap();
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "ext:test-fixture:eloquent/ModelForwarded.php".to_string(),
                "Model.whereIn".to_string()
            ))
            .unwrap(),
        "Should resolve to Model.whereIn stub, got strategy={}",
        res.strategy
    );
    assert!(
        res.strategy == "php_chain_inherited",
        "Expected php_chain_inherited strategy, got {}",
        res.strategy
    );
}

/// Realistic extractor output: `$this->account()` emits target_name = "account"
/// with a chain whose first segment is SelfRef("this").  Step 6 must detect this
/// via the chain, not via the target_name prefix.
#[test]
fn test_inherited_method_via_chain_selfref() {
    let base_file = make_file(
        "app/Services/BaseService.php",
        vec![
            make_symbol("BaseService", "App.Services.BaseService", SymbolKind::Class, Visibility::Public, Some("App.Services")),
            make_symbol("account", "App.Services.BaseService.account", SymbolKind::Method, Visibility::Public, Some("App.Services.BaseService")),
        ],
        vec![],
    );
    let child_file = make_file(
        "app/Services/ConcreteService.php",
        vec![
            make_symbol("ConcreteService", "App.Services.ConcreteService", SymbolKind::Class, Visibility::Public, Some("App.Services")),
            make_symbol("run", "App.Services.ConcreteService.run", SymbolKind::Method, Visibility::Public, Some("App.Services.ConcreteService")),
        ],
        vec![
            ExtractedRef {
                source_symbol_index: 0,
                target_name: "BaseService".to_string(),
                kind: EdgeKind::Inherits,
                line: 3,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
            // Realistic: extractor emits target_name="account" with SelfRef chain.
            ExtractedRef {
                source_symbol_index: 1,
                target_name: "account".to_string(), // NOT "$this->account"
                kind: EdgeKind::Calls,
                line: 10,
                module: None,
                chain: Some(MemberChain {
                    segments: vec![
                        ChainSegment {
                            name: "this".to_string(),
                            node_kind: "variable_name".to_string(),
                            kind: SegmentKind::SelfRef,
                            declared_type: None,
                            type_args: vec![],
                            optional_chaining: false,
                        },
                        ChainSegment {
                            name: "account".to_string(),
                            node_kind: "member_call_expression".to_string(),
                            kind: SegmentKind::Property,
                            declared_type: None,
                            type_args: vec![],
                            optional_chaining: false,
                        },
                    ],
                }),
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
        ],
    );

    let (index, id_map) = build_test_env(&[&base_file, &child_file]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&child_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &child_file.refs[1],
        source_symbol: &child_file.symbols[1], // run
        scope_chain: build_scope_chain(child_file.symbols[1].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve $this->account (chain-based) via inheritance walk");
    let res = result.unwrap();
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "app/Services/BaseService.php".to_string(),
                "App.Services.BaseService.account".to_string()
            ))
            .unwrap(),
        "Should resolve to BaseService.account, got strategy={}",
        res.strategy
    );
}

/// Two-hop inheritance: SubClass → MidClass → BaseClass.account() should
/// resolve via transitively walking the inherits_map.
#[test]
fn test_transitive_inherited_method_resolves() {
    let base_file = make_file(
        "app/Services/BaseService.php",
        vec![
            make_symbol("BaseService", "App.Services.BaseService", SymbolKind::Class, Visibility::Public, Some("App.Services")),
            make_symbol("account", "App.Services.BaseService.account", SymbolKind::Method, Visibility::Public, Some("App.Services.BaseService")),
        ],
        vec![],
    );
    let mid_file = make_file(
        "app/Services/QueuableService.php",
        vec![
            make_symbol("QueuableService", "App.Services.QueuableService", SymbolKind::Class, Visibility::Public, Some("App.Services")),
        ],
        vec![
            ExtractedRef {
                source_symbol_index: 0, // QueuableService
                target_name: "BaseService".to_string(),
                kind: EdgeKind::Inherits,
                line: 3,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
        ],
    );
    let child_file = make_file(
        "app/Jobs/SetupJob.php",
        vec![
            make_symbol("SetupJob", "App.Jobs.SetupJob", SymbolKind::Class, Visibility::Public, Some("App.Jobs")),
            make_symbol("handle", "App.Jobs.SetupJob.handle", SymbolKind::Method, Visibility::Public, Some("App.Jobs.SetupJob")),
        ],
        vec![
            ExtractedRef {
                source_symbol_index: 0, // SetupJob
                target_name: "QueuableService".to_string(),
                kind: EdgeKind::Inherits,
                line: 3,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
            ExtractedRef {
                source_symbol_index: 1, // handle
                target_name: "$this->account".to_string(),
                kind: EdgeKind::Calls,
                line: 15,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
},
        ],
    );

    let (index, id_map) = build_test_env(&[&base_file, &mid_file, &child_file]);
    let resolver = PhpResolver;
    let file_ctx = resolver.build_file_context(&child_file, None);

    let ref_ctx = RefContext {
        extracted_ref: &child_file.refs[1],
        source_symbol: &child_file.symbols[1], // handle
        scope_chain: build_scope_chain(child_file.symbols[1].scope_path.as_deref()),
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve $this->account via 2-hop inheritance");
    let res = result.unwrap();
    assert_eq!(res.strategy, "php_inherited_method");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "app/Services/BaseService.php".to_string(),
                "App.Services.BaseService.account".to_string()
            ))
            .unwrap()
    );
}
