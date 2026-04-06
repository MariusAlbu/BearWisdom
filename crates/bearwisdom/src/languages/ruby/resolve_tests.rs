use super::resolve::RubyResolver;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex, SymbolInfo};
use crate::types::*;
use std::collections::HashMap;

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    scope: Option<&str>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
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

fn make_require(source_idx: usize, name: &str, module: Option<&str>) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: name.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: module.map(|m| m.to_string()),
        chain: None,
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "ruby".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
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
            mtime: None,
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
        "app/models/order.rb",
        vec![
            make_symbol("Order", "Order", SymbolKind::Class, None),
            make_symbol("create", "Order.create", SymbolKind::Method, Some("Order")),
            make_symbol("validate!", "Order.validate!", SymbolKind::Method, Some("Order")),
        ],
        vec![make_ref(1, "validate!", EdgeKind::Calls)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = RubyResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[1],
        scope_chain: build_scope_chain(file.symbols[1].scope_path.as_deref()),
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve validate! via scope chain");
    let res = result.unwrap();
    assert_eq!(res.strategy, "ruby_scope_chain");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("app/models/order.rb".to_string(), "Order.validate!".to_string())).unwrap()
    );
}

#[test]
fn test_same_file_resolution() {
    let file1 = make_file(
        "app/models/user.rb",
        vec![make_symbol("User", "User", SymbolKind::Class, None)],
        vec![],
    );

    let file2 = make_file(
        "app/services/user_service.rb",
        vec![
            make_symbol("UserService", "UserService", SymbolKind::Class, None),
            make_symbol("UserHelper", "UserHelper", SymbolKind::Class, None),
        ],
        // UserService references UserHelper — same file, no require needed
        vec![make_ref(0, "UserHelper", EdgeKind::TypeRef)],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = RubyResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve UserHelper via same-file");
    let res = result.unwrap();
    assert_eq!(res.strategy, "ruby_same_file");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("app/services/user_service.rb".to_string(), "UserHelper".to_string())).unwrap()
    );
}

#[test]
fn test_same_module_resolution() {
    let file = make_file(
        "lib/myapp/presenter.rb",
        vec![
            // Ruby modules use SymbolKind::Namespace in the index.
            make_symbol("MyApp", "MyApp", SymbolKind::Namespace, None),
            make_symbol("Presenter", "MyApp.Presenter", SymbolKind::Class, Some("MyApp")),
            make_symbol("Formatter", "MyApp.Formatter", SymbolKind::Class, Some("MyApp")),
        ],
        vec![make_ref(1, "Formatter", EdgeKind::TypeRef)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = RubyResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[1],
        scope_chain: build_scope_chain(file.symbols[1].scope_path.as_deref()),
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve Formatter");
    // May resolve via scope_chain or same_module — both correct.
    let res = result.unwrap();
    assert!(
        res.strategy == "ruby_scope_chain" || res.strategy == "ruby_same_module",
        "Unexpected strategy: {}",
        res.strategy
    );
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("lib/myapp/presenter.rb".to_string(), "MyApp.Formatter".to_string())).unwrap()
    );
}

#[test]
fn test_falls_back_for_unknown() {
    let file = make_file(
        "app/models/post.rb",
        vec![make_symbol("Post", "Post", SymbolKind::Class, None)],
        vec![make_ref(0, "UnknownThing", EdgeKind::TypeRef)],
    );

    let (index, _) = build_test_env(&[&file]);
    let resolver = RubyResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Unknown should fall back"
    );
}

#[test]
fn test_infer_builtin_external() {
    let file = make_file(
        "app/models/post.rb",
        vec![make_symbol("Post", "Post", SymbolKind::Class, None)],
        vec![make_ref(0, "puts", EdgeKind::Calls)],
    );

    let resolver = RubyResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("ruby_core".to_string()));
}

#[test]
fn test_require_builds_imports() {
    let file = make_file(
        "app/models/post.rb",
        vec![make_symbol("Post", "Post", SymbolKind::Class, None)],
        vec![
            make_require(0, "json", None),
            make_require(0, "bar", Some("./bar")),
        ],
    );

    let resolver = RubyResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert_eq!(ctx.imports.len(), 2);

    let json_import = ctx.imports.iter().find(|i| i.imported_name == "json").unwrap();
    assert_eq!(json_import.module_path.as_deref(), Some("json"));

    let bar_import = ctx.imports.iter().find(|i| i.imported_name == "bar").unwrap();
    assert_eq!(bar_import.module_path.as_deref(), Some("./bar"));
}

#[test]
fn test_stdlib_require_is_external() {
    let file = make_file(
        "lib/foo.rb",
        vec![make_symbol("Foo", "Foo", SymbolKind::Class, None)],
        vec![make_require(0, "json", None)],
    );

    let resolver = RubyResolver;
    let file_ctx = resolver.build_file_context(&file, None);
    let import_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "json".to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: None,
        chain: None,
    };

    let sym = make_symbol("Foo", "Foo", SymbolKind::Class, None);
    let ref_ctx = RefContext {
        extracted_ref: &import_ref,
        source_symbol: &sym,
        scope_chain: vec![],
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("json".to_string()), "json stdlib require should be external");
}
