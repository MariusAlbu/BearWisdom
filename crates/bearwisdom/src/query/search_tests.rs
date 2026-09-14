use super::*;
use crate::db::Database;

/// Insert a symbol and let the triggers populate symbols_fts.
fn insert_symbol(
    db: &Database,
    path: &str,
    name: &str,
    qname: &str,
    kind: &str,
    sig: Option<&str>,
    doc: Option<&str>,
) -> i64 {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
         ON CONFLICT(path) DO NOTHING",
        [path],
    )
    .unwrap();
    let fid: i64 = conn
        .query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0))
        .unwrap();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, signature, doc_comment)
         VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, ?6)",
        rusqlite::params![fid, name, qname, kind, sig, doc],
    ).unwrap();
    conn.last_insert_rowid()
}

#[test]
fn search_finds_symbol_by_name() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "a.cs",
        "CatalogService",
        "App.CatalogService",
        "class",
        None,
        None,
    );

    let results = search_symbols(
        &db,
        "CatalogService",
        10,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    assert!(!results.is_empty(), "Should find CatalogService");
    assert_eq!(results[0].name, "CatalogService");
}

#[test]
fn search_prefix_match() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "a.cs",
        "CatalogService",
        "App.CatalogService",
        "class",
        None,
        None,
    );
    insert_symbol(
        &db,
        "b.cs",
        "CatalogItem",
        "App.CatalogItem",
        "class",
        None,
        None,
    );
    insert_symbol(
        &db,
        "c.cs",
        "OrderService",
        "App.OrderService",
        "class",
        None,
        None,
    );

    // Prefix query: "Catalog*" should match CatalogService and CatalogItem.
    let results = search_symbols(&db, "Catalog*", 10, &crate::query::QueryOptions::full()).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names.contains(&"CatalogService"),
        "Should match CatalogService"
    );
    assert!(names.contains(&"CatalogItem"), "Should match CatalogItem");
    assert!(
        !names.contains(&"OrderService"),
        "Should not match OrderService"
    );
}

#[test]
fn search_matches_in_doc_comment() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "a.cs",
        "GetItems",
        "App.GetItems",
        "method",
        None,
        Some("Returns all items from the authentication store"),
    );

    let results = search_symbols(
        &db,
        "authentication",
        10,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    assert!(!results.is_empty(), "Should find symbol via doc comment");
    assert_eq!(results[0].name, "GetItems");
}

#[test]
fn search_returns_empty_for_nonexistent_term() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "a.cs",
        "FooService",
        "App.FooService",
        "class",
        None,
        None,
    );

    let results = search_symbols(
        &db,
        "ZzzNotFoundXxx",
        10,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_respects_limit() {
    let db = Database::open_in_memory().unwrap();
    for i in 0..10 {
        insert_symbol(
            &db,
            "a.cs",
            &format!("Widget{i}"),
            &format!("App.Widget{i}"),
            "class",
            None,
            None,
        );
    }

    let results = search_symbols(&db, "Widget*", 3, &crate::query::QueryOptions::full()).unwrap();
    assert!(results.len() <= 3, "Should respect limit of 3");
}

#[test]
fn search_empty_query_returns_empty() {
    let db = Database::open_in_memory().unwrap();
    let results = search_symbols(&db, "", 10, &crate::query::QueryOptions::full()).unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_matches_in_signature() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "a.cs",
        "Fetch",
        "App.Fetch",
        "method",
        Some("Task<CatalogItem> Fetch(int id)"),
        None,
    );

    let results =
        search_symbols(&db, "CatalogItem", 10, &crate::query::QueryOptions::full()).unwrap();
    // The FTS index includes the signature, so "CatalogItem" in the sig should match.
    assert!(!results.is_empty(), "Should match via signature");
}

#[test]
fn search_multi_term_query_falls_back_to_any_matching_identifier() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "registry.rs",
        "all_resolvers_with_workspace",
        "module_resolution.all_resolvers_with_workspace",
        "function",
        None,
        None,
    );
    insert_symbol(
        &db,
        "resolver.rs",
        "ModuleResolver",
        "module_resolution.ModuleResolver",
        "trait",
        None,
        None,
    );

    let results = search_symbols(
        &db,
        "all_resolvers_with_workspace ModuleResolver",
        10,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();

    assert!(names.contains(&"all_resolvers_with_workspace"));
    assert!(names.contains(&"ModuleResolver"));
}

#[test]
fn search_file_stem_returns_tests_from_that_file_alongside_symbol_matches() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "src/languages/php/profile_tests.rs",
        "php_qualified_import_candidates_keep_boundaries",
        "php_qualified_import_candidates_keep_boundaries",
        "test",
        None,
        None,
    );
    insert_symbol(
        &db,
        "src/indexer/chain_root_binding_tests.rs",
        "aliased_import_root_binds_original",
        "aliased_import_root_binds_original",
        "test",
        None,
        None,
    );

    let results = search_symbols(
        &db,
        "php_qualified_import_candidates chain_root_binding_tests",
        8,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    let names: Vec<&str> = results.iter().map(|result| result.name.as_str()).collect();
    assert!(names.contains(&"php_qualified_import_candidates_keep_boundaries"));
    assert!(names.contains(&"aliased_import_root_binds_original"));
}

#[test]
fn natural_query_is_not_flooded_by_common_exact_symbol_names() {
    let db = Database::open_in_memory().unwrap();
    for index in 0..12 {
        insert_symbol(
            &db,
            &format!("src/noise_{index}.rs"),
            "source",
            &format!("Noise{index}.source"),
            "field",
            None,
            None,
        );
    }
    insert_symbol(
        &db,
        "src/type_checker/profile/name_spelling.rs",
        "index_qname_from_source",
        "LanguageProfile.index_qname_from_source",
        "method",
        None,
        None,
    );
    insert_symbol(
        &db,
        "src/type_checker/profile/name_spelling.rs",
        "index_qname_path_from_source",
        "LanguageProfile.index_qname_path_from_source",
        "method",
        None,
        None,
    );
    for name in [
        "source_qualified_module_matches_the_canonical_chain_qname",
        "source_qualified_scope_uses_canonical_index_qname",
        "source_qualified_type_head_normalizes_to_a_canonical_index_qname",
    ] {
        insert_symbol(
            &db,
            "src/indexer/resolve/engine/qualified_name_tests.rs",
            name,
            name,
            "test",
            None,
            None,
        );
    }

    let results = search_symbols(
        &db,
        "canonical indexed qualified name source-language qualified name",
        3,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    assert_eq!(results[0].name, "index_qname_from_source");
}

#[test]
fn natural_ranking_requires_a_query_concept_in_the_identifier() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "src/indexer/resolve/engine/module_trait_inputs.rs",
        "allocate",
        "allocate",
        "function",
        Some("fn allocate(module: &ModuleGraph)"),
        None,
    );
    insert_symbol(
        &db,
        "src/indexer/module_resolution/mod.rs",
        "ModuleResolver",
        "ModuleResolver",
        "trait",
        Some("trait ModuleResolver"),
        None,
    );

    let results = search_symbols(
        &db,
        "module resolution registry factory trait generic engine dispatcher PHP",
        1,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    assert_eq!(results[0].name, "ModuleResolver");
}

#[test]
fn focused_test_query_keeps_all_high_coverage_tests_from_one_file() {
    let db = Database::open_in_memory().unwrap();
    for name in [
        "php_qualified_import_candidates_keep_namespace_and_containment_boundaries",
        "php_qualified_import_candidates_decline_empty_namespace_components",
        "php_chain_qualification_is_same_package_and_imports",
    ] {
        insert_symbol(
            &db,
            "src/languages/php/profile_tests.rs",
            name,
            name,
            "test",
            None,
            None,
        );
    }
    insert_symbol(
        &db,
        "src/languages/php/profile.rs",
        "php_qualified_import_type_candidates",
        "php_qualified_import_type_candidates",
        "function",
        None,
        None,
    );

    let results = search_symbols(
        &db,
        "php qualified import chain qualification regression",
        20,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    let names: HashSet<&str> = results.iter().map(|result| result.name.as_str()).collect();
    assert!(
        names.contains("php_qualified_import_candidates_keep_namespace_and_containment_boundaries")
    );
    assert!(names.contains("php_qualified_import_candidates_decline_empty_namespace_components"));
    assert!(names.contains("php_chain_qualification_is_same_package_and_imports"));
}

#[test]
fn exact_test_anchor_includes_nearby_regression_siblings() {
    let db = Database::open_in_memory().unwrap();
    for name in [
        "aliased_import_root_binds_the_original_not_a_same_named_stranger",
        "aliased_import_without_its_declaration_is_a_miss_not_a_hijack",
        "active_binding_type_precedes_the_extractors_flat_annotation",
    ] {
        insert_symbol(
            &db,
            "src/indexer/resolve/engine/chain_root_binding_tests.rs",
            name,
            name,
            "test",
            None,
            None,
        );
    }

    let results = search_symbols(
        &db,
        "aliased_import_root_binds_the_original_not_a_same_named_stranger",
        5,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    let names: HashSet<&str> = results.iter().map(|result| result.name.as_str()).collect();
    assert!(names.contains("aliased_import_root_binds_the_original_not_a_same_named_stranger"));
    assert!(names.contains("aliased_import_without_its_declaration_is_a_miss_not_a_hijack"));
    assert!(!names.contains("active_binding_type_precedes_the_extractors_flat_annotation"));
    assert_eq!(
        results[1].name,
        "aliased_import_without_its_declaration_is_a_miss_not_a_hijack"
    );
}

#[test]
fn repeated_directory_terms_do_not_bury_a_denser_dispatcher_name() {
    let terms = ranking_terms("module resolution registry factory module resolver PHP");
    let dispatcher = SearchResult {
        name: "resolve_via_module_resolver".into(),
        qualified_name: "resolve_via_module_resolver".into(),
        kind: "function".into(),
        file_path: "src/indexer/resolve/engine/module_specifier.rs".into(),
        start_line: 88,
        signature: None,
        score: 0.0,
    };
    let wrapper = SearchResult {
        name: "resolve_module_path".into(),
        qualified_name: "Compilation.resolve_module_path".into(),
        kind: "method".into(),
        file_path: "src/indexer/resolve/engine/compilation_module_resolution.rs".into(),
        start_line: 10,
        signature: None,
        score: 0.0,
    };

    assert!(rank_for_task(&dispatcher, &terms) > rank_for_task(&wrapper, &terms));

    let test_helper = SearchResult {
        name: "make_resolver".into(),
        qualified_name: "tests.make_resolver".into(),
        kind: "function".into(),
        file_path: "src/indexer/module_resolution/go_mod.rs".into(),
        start_line: 107,
        signature: None,
        score: 0.0,
    };
    assert!(rank_for_task(&dispatcher, &terms) > rank_for_task(&test_helper, &terms));

    let generic_helper = SearchResult {
        name: "all".into(),
        qualified_name: "all".into(),
        kind: "function".into(),
        file_path: "src/indexer/resolve/engine/trait_obligations.rs".into(),
        start_line: 140,
        signature: None,
        score: 0.0,
    };
    assert!(rank_for_task(&dispatcher, &terms) > rank_for_task(&generic_helper, &terms));

    let path_only_match = SearchResult {
        name: "serialize".into(),
        qualified_name: "Owner.serialize".into(),
        kind: "method".into(),
        file_path: "src/indexer/resolve/engine/module_trait_inputs.rs".into(),
        start_line: 14,
        signature: None,
        score: 0.0,
    };
    assert!(rank_for_task(&dispatcher, &terms) > rank_for_task(&path_only_match, &terms));
}

#[test]
fn natural_query_combines_symbol_and_file_concepts_with_diversity() {
    let db = Database::open_in_memory().unwrap();
    for (path, name, kind) in [
        (
            "src/indexer/resolve/engine/module_specifier.rs",
            "resolve_via_module_resolver",
            "function",
        ),
        (
            "src/indexer/module_resolution/mod.rs",
            "all_resolvers_with_workspace",
            "function",
        ),
        (
            "src/indexer/module_resolution/php_mod.rs",
            "PhpModuleResolver",
            "struct",
        ),
        (
            "src/indexer/resolve/engine/chain_root_binding_tests.rs",
            "aliased_import_root_binds_the_original_not_a_same_named_stranger",
            "test",
        ),
        (
            "src/languages/php/profile_tests.rs",
            "php_qualified_import_candidates_keep_namespace_and_containment_boundaries",
            "test",
        ),
    ] {
        insert_symbol(&db, path, name, name, kind, None, None);
    }

    let results = search_symbols(
        &db,
        "language registry module resolution php chain root regression tests",
        12,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    let names: HashSet<&str> = results.iter().map(|result| result.name.as_str()).collect();
    assert!(names.contains("resolve_via_module_resolver"));
    assert!(names.contains("all_resolvers_with_workspace"));
    assert!(names.contains("PhpModuleResolver"));
    assert!(names.contains("aliased_import_root_binds_the_original_not_a_same_named_stranger"));
    assert!(
        names.contains("php_qualified_import_candidates_keep_namespace_and_containment_boundaries")
    );
}

#[test]
fn natural_ranking_uses_identifier_adjacency_and_neutral_code_roles() {
    let terms = ranking_terms("module resolver registry factory dispatcher");

    assert_eq!(adjacent_term_pairs("resolve_dts_module", &terms), 0);
    assert_eq!(
        adjacent_term_pairs("resolve_via_module_resolver", &terms),
        3
    );
    assert_eq!(adjacent_term_pairs("PhpModuleResolver", &terms), 1);
    assert!(contains_term("all_resolvers", "factory"));
    assert!(contains_term("resolve_via_language_resolver", "dispa"));
    assert_eq!(
        result_family("PhpModuleResolver", "struct").0,
        result_family("ModuleResolver", "trait").0
    );
    assert_eq!(
        result_family("resolve_via_module_resolver", "function"),
        result_family("resolve_module_via_language_resolver", "method")
    );
    assert_eq!(
        result_family("resolve_via_module_resolver", "function").1,
        1
    );
}

#[test]
fn uppercase_language_acronym_does_not_become_an_exact_code_anchor() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db,
        "src/profile.rs",
        "PHP",
        "Language.PHP",
        "variable",
        None,
        None,
    );
    insert_symbol(
        &db,
        "src/indexer/module_resolution/php_mod.rs",
        "PhpModuleResolver",
        "PhpModuleResolver",
        "struct",
        None,
        None,
    );

    let results = search_symbols(
        &db,
        "module resolver registry factory dispatcher PHP",
        2,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    assert_eq!(results[0].name, "PhpModuleResolver");
    assert!(!results.iter().any(|result| result.name == "PHP"));
}

#[test]
fn test_filter_recovers_complete_behavior_groups_without_test_words() {
    let db = Database::open_in_memory().unwrap();
    for (path, name) in [
        (
            "src/engine/chain_root_tests.rs",
            "aliased_import_root_binds_the_original_not_a_same_named_stranger",
        ),
        (
            "src/engine/chain_root_tests.rs",
            "aliased_import_without_its_declaration_is_a_miss_not_a_hijack",
        ),
        (
            "src/php/profile_tests.rs",
            "php_qualified_import_candidates_keep_namespace_and_containment_boundaries",
        ),
        (
            "src/php/profile_tests.rs",
            "php_qualified_import_candidates_decline_empty_namespace_components",
        ),
        (
            "src/php/profile_tests.rs",
            "php_chain_qualification_is_same_package_and_imports",
        ),
    ] {
        insert_symbol(&db, path, name, name, "test", None, None);
    }
    insert_symbol(
        &db,
        "src/engine/chain_root.rs",
        "imported_qualified_root_anchor",
        "imported_qualified_root_anchor",
        "function",
        None,
        None,
    );

    let results = search_symbols_filtered(
        &db,
        "aliased import chain root PHP qualified import chain qualification",
        10,
        &crate::query::QueryOptions::full(),
        SearchResultFilter::Tests,
    )
    .unwrap();
    let names: HashSet<&str> = results.iter().map(|result| result.name.as_str()).collect();
    assert_eq!(results.len(), 5);
    assert!(names.contains("aliased_import_root_binds_the_original_not_a_same_named_stranger"));
    assert!(names.contains("aliased_import_without_its_declaration_is_a_miss_not_a_hijack"));
    assert!(
        names.contains("php_qualified_import_candidates_keep_namespace_and_containment_boundaries")
    );
    assert!(names.contains("php_qualified_import_candidates_decline_empty_namespace_components"));
    assert!(names.contains("php_chain_qualification_is_same_package_and_imports"));
    assert!(results.iter().all(|result| result.kind == "test"));
}

#[test]
fn registry_ranking_prefers_the_structural_factory_over_a_narrow_wrapper() {
    let db = Database::open_in_memory().unwrap();
    let registry = insert_symbol(
        &db,
        "src/module_resolution/mod.rs",
        "all_resolvers_with_workspace",
        "all_resolvers_with_workspace",
        "function",
        Some("fn all_resolvers_with_workspace("),
        None,
    );
    let wrapper = insert_symbol(
        &db,
        "src/module_resolution/mod.rs",
        "all_resolvers_with_go_module",
        "all_resolvers_with_go_module",
        "function",
        Some("fn all_resolvers_with_go_module(go_module: Option<&str>)"),
        None,
    );
    for index in 0..10 {
        let target = insert_symbol(
            &db,
            &format!("src/module_resolution/resolver_{index}.rs"),
            &format!("Resolver{index}"),
            &format!("Resolver{index}"),
            "struct",
            None,
            None,
        );
        db.conn()
            .execute(
                "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
                rusqlite::params![registry, target],
            )
            .unwrap();
    }
    db.conn()
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
            rusqlite::params![wrapper, registry],
        )
        .unwrap();

    let results = search_symbols(
        &db,
        "module resolution registry factory trait dispatcher",
        10,
        &crate::query::QueryOptions::full(),
    )
    .unwrap();
    let registry_position = results
        .iter()
        .position(|result| result.name == "all_resolvers_with_workspace")
        .unwrap();
    let wrapper_position = results
        .iter()
        .position(|result| result.name == "all_resolvers_with_go_module")
        .unwrap();
    assert!(registry_position < wrapper_position);
}
