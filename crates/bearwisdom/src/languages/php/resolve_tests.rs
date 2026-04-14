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
