use super::extract::extract;
use crate::types::{ExtractedRef, ExtractedSymbol};
use crate::indexer::resolve::engine::{build_scope_chain, SymbolIndex};
use crate::types::*;
use std::collections::HashMap;

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

fn make_import_ref(source_idx: usize, name: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: name.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: Some(module.to_string()),
        chain: None,
    }
}

fn make_file(path: &str, lang: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: lang.to_string(),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_scope_chain_resolution() {
    let file = make_file(
        "src/OrderService.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("OrderService", "com.example.OrderService", SymbolKind::Class, Visibility::Public, Some("com.example")),
            make_symbol("create", "com.example.OrderService.create", SymbolKind::Method, Visibility::Public, Some("com.example.OrderService")),
            make_symbol("validate", "com.example.OrderService.validate", SymbolKind::Method, Visibility::Private, Some("com.example.OrderService")),
        ],
        vec![make_ref(2, "validate", EdgeKind::Calls, 10)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[2],
        scope_chain: build_scope_chain(file.symbols[2].scope_path.as_deref()),
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve validate via scope chain");
    let res = result.unwrap();
    assert_eq!(res.strategy, "java_scope_chain");
    assert_eq!(res.confidence, 1.0);
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/OrderService.java".to_string(), "com.example.OrderService.validate".to_string())).unwrap()
    );
}

#[test]
fn test_same_package_resolution() {
    let file1 = make_file(
        "src/Order.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("Order", "com.example.Order", SymbolKind::Class, Visibility::Public, Some("com.example")),
        ],
        vec![],
    );

    let file2 = make_file(
        "src/OrderService.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("OrderService", "com.example.OrderService", SymbolKind::Class, Visibility::Public, Some("com.example")),
        ],
        // No import — same package visibility
        vec![make_ref(1, "Order", EdgeKind::TypeRef, 5)],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[1],
        scope_chain: build_scope_chain(file2.symbols[1].scope_path.as_deref()),
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve Order via same-package");
    let res = result.unwrap();
    assert!(
        res.strategy == "java_scope_chain" || res.strategy == "java_same_package",
        "Unexpected strategy: {}",
        res.strategy
    );
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/Order.java".to_string(), "com.example.Order".to_string())).unwrap()
    );
}

#[test]
fn test_exact_import_resolution() {
    let file1 = make_file(
        "src/Product.java",
        "java",
        vec![make_symbol("Product", "com.store.model.Product", SymbolKind::Class, Visibility::Public, Some("com.store.model"))],
        vec![],
    );

    let file2 = make_file(
        "src/ProductController.java",
        "java",
        vec![make_symbol("ProductController", "com.store.web.ProductController", SymbolKind::Class, Visibility::Public, Some("com.store.web"))],
        vec![
            make_ref(0, "Product", EdgeKind::TypeRef, 10),
            make_import_ref(0, "Product", "com.store.model.Product"),
        ],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve Product via import");
    let res = result.unwrap();
    assert_eq!(res.strategy, "java_import");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/Product.java".to_string(), "com.store.model.Product".to_string())).unwrap()
    );
}

#[test]
fn test_wildcard_import_resolution() {
    let file1 = make_file(
        "src/User.java",
        "java",
        vec![make_symbol("User", "com.app.model.User", SymbolKind::Class, Visibility::Public, Some("com.app.model"))],
        vec![],
    );

    let file2 = make_file(
        "src/UserService.java",
        "java",
        vec![make_symbol("UserService", "com.app.service.UserService", SymbolKind::Class, Visibility::Public, Some("com.app.service"))],
        vec![
            make_ref(0, "User", EdgeKind::TypeRef, 5),
            // Wildcard import: import com.app.model.*;
            make_import_ref(0, "*", "com.app.model"),
        ],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve User via wildcard import");
    let res = result.unwrap();
    assert_eq!(res.strategy, "java_wildcard_import");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/User.java".to_string(), "com.app.model.User".to_string())).unwrap()
    );
}

#[test]
fn test_private_cross_file_not_resolved() {
    let file1 = make_file(
        "src/Internal.java",
        "java",
        vec![make_symbol("helper", "com.app.Internal.helper", SymbolKind::Method, Visibility::Private, Some("com.app.Internal"))],
        vec![],
    );

    let file2 = make_file(
        "src/Client.java",
        "java",
        vec![make_symbol("Client", "com.app.Client", SymbolKind::Class, Visibility::Public, Some("com.app"))],
        vec![
            make_ref(0, "helper", EdgeKind::Calls, 5),
            make_import_ref(0, "*", "com.app"),
        ],
    );

    let (index, _) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    };

    // Private cross-file should not resolve.
    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Private cross-file should not resolve"
    );
}

#[test]
fn test_falls_back_for_unknown() {
    let file = make_file(
        "src/Test.java",
        "java",
        vec![make_symbol("Test", "com.app.Test", SymbolKind::Class, Visibility::Public, Some("com.app"))],
        vec![make_ref(0, "Nonexistent", EdgeKind::TypeRef, 5)],
    );

    let (index, _) = build_test_env(&[&file]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Unknown symbol should fall back"
    );
}

#[test]
fn test_infer_stdlib_external() {
    use crate::indexer::resolve::engine::RefContext;

    let file = make_file(
        "src/Test.java",
        "java",
        vec![make_symbol("Test", "com.app.Test", SymbolKind::Class, Visibility::Public, Some("com.app"))],
        vec![make_ref(0, "System", EdgeKind::TypeRef, 5)],
    );

    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert!(ns.is_some(), "System should be inferred as external");
    assert_eq!(ns.unwrap(), "java.lang");
}

#[test]
fn test_build_file_context_extracts_package() {
    let file = make_file(
        "src/Foo.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("Foo", "com.example.Foo", SymbolKind::Class, Visibility::Public, Some("com.example")),
        ],
        vec![],
    );

    let resolver = JavaResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert_eq!(ctx.file_namespace, Some("com.example".to_string()));
}

#[test]
fn test_build_file_context_wildcard_import() {
    let file = make_file(
        "src/Foo.java",
        "java",
        vec![make_symbol("Foo", "com.example.Foo", SymbolKind::Class, Visibility::Public, Some("com.example"))],
        vec![make_import_ref(0, "*", "org.springframework.web.bind.annotation")],
    );

    let resolver = JavaResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert_eq!(ctx.imports.len(), 1);
    assert!(ctx.imports[0].is_wildcard);
    assert_eq!(
        ctx.imports[0].module_path.as_deref(),
        Some("org.springframework.web.bind.annotation")
    );
}
