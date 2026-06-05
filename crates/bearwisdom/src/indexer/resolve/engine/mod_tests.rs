// =============================================================================
// indexer/resolve/engine/mod_tests.rs — sibling tests for engine/mod.rs
// =============================================================================

use crate::indexer::resolve::engine::{
    build_scope_chain, ChainMiss, LocalTypeCache, SymbolIndex, SymbolInfo, SymbolLookup,
};
use crate::indexer::resolve::engine::chain_walker::{
    parse_declared_type_from_signature_for_lang,
    parse_param_types_from_signature, parse_param_types_from_signature_for_lang,
    parse_return_type_from_signature, resolve_type_name_in_scope, tuple_element,
};
use crate::indexer::resolve::engine::index::LOCAL_TYPE_CACHE;
use crate::type_checker::core::types::Type;
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[test]
fn test_scope_chain_from_path() {
    let chain = build_scope_chain(Some("A.B.C"));
    assert_eq!(chain, vec!["A.B.C", "A.B", "A"]);
}

fn dummy_sym(qname: &str) -> SymbolInfo {
    SymbolInfo {
        id: 0,
        name: qname.rsplit('.').next().unwrap().to_string(),
        qualified_name: qname.to_string(),
        kind: "class".to_string(),
        visibility: None,
        file_path: Arc::from(""),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

#[test]
fn scope_resolve_walks_outward_and_matches_first() {
    // dayjs shape: `namespace dayjs { class Dayjs { clone(): Dayjs } }`.
    // Method scope_path is "dayjs.Dayjs"; return-type ref is the raw
    // "Dayjs" from source. The resolver must probe
    // "dayjs.Dayjs.Dayjs" → "dayjs.Dayjs" → "dayjs" → "Dayjs" and
    // pick the first present in the index.
    let mut map: BTreeMap<String, SymbolInfo> = BTreeMap::new();
    map.insert("dayjs.Dayjs".to_string(), dummy_sym("dayjs.Dayjs"));

    let resolved =
        resolve_type_name_in_scope("Dayjs", Some("dayjs.Dayjs"), &map);
    assert_eq!(resolved, "dayjs.Dayjs");
}

#[test]
fn scope_resolve_prefers_innermost_shadow() {
    // Type name shadowing: if there's a class-scoped type with the
    // same name as an outer namespace type, innermost wins.
    let mut map: BTreeMap<String, SymbolInfo> = BTreeMap::new();
    map.insert("ns.Outer.X".to_string(), dummy_sym("ns.Outer.X"));
    map.insert("ns.X".to_string(), dummy_sym("ns.X"));

    let resolved =
        resolve_type_name_in_scope("X", Some("ns.Outer"), &map);
    assert_eq!(resolved, "ns.Outer.X");
}

#[test]
fn scope_resolve_fallback_to_raw_when_no_match() {
    // When nothing matches, return the raw text so downstream
    // consumers can still use it (e.g. builtin type lookups).
    let map: BTreeMap<String, SymbolInfo> = BTreeMap::new();
    let resolved =
        resolve_type_name_in_scope("boolean", Some("dayjs.Dayjs"), &map);
    assert_eq!(resolved, "boolean");
}

#[test]
fn scope_resolve_preserves_already_qualified_name() {
    // Extractor-emitted fully-qualified names pass through unchanged
    // even when the shorter form would match.
    let mut map: BTreeMap<String, SymbolInfo> = BTreeMap::new();
    map.insert("dayjs.Dayjs".to_string(), dummy_sym("dayjs.Dayjs"));
    let resolved = resolve_type_name_in_scope(
        "dayjs.Dayjs",
        Some("dayjs.Dayjs"),
        &map,
    );
    assert_eq!(resolved, "dayjs.Dayjs");
}

#[test]
fn scope_resolve_without_scope_returns_raw() {
    let mut map: BTreeMap<String, SymbolInfo> = BTreeMap::new();
    map.insert("Foo".to_string(), dummy_sym("Foo"));
    assert_eq!(resolve_type_name_in_scope("Foo", None, &map), "Foo");
}

#[test]
fn parse_param_types_dotnet_style() {
    assert_eq!(
        parse_param_types_from_signature("Greet(string): string"),
        Some(vec!["string".to_string()])
    );
    assert_eq!(
        parse_param_types_from_signature("Add<K, V>(K, V): Dictionary<K, V>"),
        Some(vec!["K".to_string(), "V".to_string()])
    );
    assert_eq!(
        parse_param_types_from_signature("Foo(): void"),
        Some(Vec::<String>::new())
    );
}

#[test]
fn parse_param_types_typescript_style() {
    assert_eq!(
        parse_param_types_from_signature("(x: number, y: string): boolean"),
        Some(vec!["number".to_string(), "string".to_string()])
    );
    assert_eq!(
        parse_param_types_from_signature("get<T>(input: T): T"),
        Some(vec!["T".to_string()])
    );
}

#[test]
fn parse_param_types_handles_generics_inside_args() {
    assert_eq!(
        parse_param_types_from_signature("Set(Map<K, V>): void"),
        Some(vec!["Map<K, V>".to_string()])
    );
}

#[test]
fn parse_param_types_go_style_postfix() {
    // Go: `name(a Type, b Type) Ret` — type is the last whitespace-
    // separated token in each arg.
    assert_eq!(
        parse_param_types_from_signature_for_lang("Add(a int, b int) int", "go"),
        Some(vec!["int".to_string(), "int".to_string()])
    );
    assert_eq!(
        parse_param_types_from_signature_for_lang("Set(items []int) bool", "go"),
        Some(vec!["[]int".to_string()])
    );
}

#[test]
fn parse_param_types_c_style_prefix() {
    // C / C++ / Java / C#: `name(Type a, Type b)` — type is everything
    // before the last whitespace per arg.
    assert_eq!(
        parse_param_types_from_signature_for_lang("add(int a, int b)", "c"),
        Some(vec!["int".to_string(), "int".to_string()])
    );
    assert_eq!(
        parse_param_types_from_signature_for_lang("add(int a, int b)", "java"),
        Some(vec!["int".to_string(), "int".to_string()])
    );
    assert_eq!(
        parse_param_types_from_signature_for_lang("Send(string message)", "csharp"),
        Some(vec!["string".to_string()])
    );
}

#[test]
fn parse_param_types_rust_style_colon() {
    assert_eq!(
        parse_param_types_from_signature_for_lang("add(a: i32, b: i32) -> i32", "rust"),
        Some(vec!["i32".to_string(), "i32".to_string()])
    );
}

#[test]
fn parse_declared_type_typescript_colon() {
    assert_eq!(
        parse_declared_type_from_signature_for_lang("users: UserMap", "typescript"),
        Some("UserMap".to_string())
    );
    assert_eq!(
        parse_declared_type_from_signature_for_lang("repo: Repository<User>", "typescript"),
        Some("Repository<User>".to_string())
    );
    assert_eq!(
        parse_declared_type_from_signature_for_lang("count: number = 0", "typescript"),
        Some("number".to_string())
    );
}

#[test]
fn parse_declared_type_go_postfix() {
    assert_eq!(
        parse_declared_type_from_signature_for_lang("count int", "go"),
        Some("int".to_string())
    );
    assert_eq!(
        parse_declared_type_from_signature_for_lang("items []int", "go"),
        Some("[]int".to_string())
    );
}

#[test]
fn parse_declared_type_c_prefix() {
    assert_eq!(
        parse_declared_type_from_signature_for_lang("int count", "c"),
        Some("int".to_string())
    );
    assert_eq!(
        parse_declared_type_from_signature_for_lang("string name", "csharp"),
        Some("string".to_string())
    );
    assert_eq!(
        parse_declared_type_from_signature_for_lang("List<User> users", "java"),
        Some("List<User>".to_string())
    );
}

#[test]
fn parse_declared_type_empty_returns_none() {
    assert_eq!(parse_declared_type_from_signature_for_lang("", "typescript"), None);
}

#[test]
fn parse_param_types_returns_none_on_no_parens() {
    assert_eq!(parse_param_types_from_signature("class Foo"), None);
    assert_eq!(parse_param_types_from_signature(""), None);
}

#[test]
fn parse_return_type_from_dotnet_signature() {
    assert_eq!(
        parse_return_type_from_signature("Greet(string): string"),
        Some("string".to_string())
    );
    assert_eq!(
        parse_return_type_from_signature("Get<T>(int): Task<T>"),
        Some("Task<T>".to_string())
    );
    assert_eq!(
        parse_return_type_from_signature(
            "Add<K, V>(K, V): Dictionary<K, V>"
        ),
        Some("Dictionary<K, V>".to_string())
    );
    // No trailing colon — Java style leading-return-type. Nothing to extract.
    assert_eq!(parse_return_type_from_signature("String foo()"), None);
    // Constructor — no colon suffix. Parser must not claim `Foo` or `()`.
    assert_eq!(parse_return_type_from_signature("Foo()"), None);
    // Generic with nested angle brackets — colons INSIDE must not fire.
    assert_eq!(
        parse_return_type_from_signature(
            "Map<K: Ord, V>(K): V"
        ),
        Some("V".to_string())
    );
    assert_eq!(parse_return_type_from_signature(""), None);
}

#[test]
fn tuple_element_simple_pair() {
    assert_eq!(
        tuple_element("[boolean, Dispatch<SetStateAction<boolean>>]", 0),
        Some("boolean".to_string())
    );
    assert_eq!(
        tuple_element("[boolean, Dispatch<SetStateAction<boolean>>]", 1),
        Some("Dispatch<SetStateAction<boolean>>".to_string())
    );
}

#[test]
fn tuple_element_handles_whitespace_and_commas_in_generics() {
    assert_eq!(
        tuple_element(" [ A , B<X, Y> , C ] ", 0),
        Some("A".to_string())
    );
    assert_eq!(
        tuple_element(" [ A , B<X, Y> , C ] ", 1),
        Some("B<X, Y>".to_string())
    );
    assert_eq!(
        tuple_element(" [ A , B<X, Y> , C ] ", 2),
        Some("C".to_string())
    );
}

#[test]
fn tuple_element_handles_nested_tuples() {
    assert_eq!(
        tuple_element("[[inner, tuple], outer]", 0),
        Some("[inner, tuple]".to_string())
    );
    assert_eq!(
        tuple_element("[[inner, tuple], outer]", 1),
        Some("outer".to_string())
    );
}

#[test]
fn tuple_element_out_of_range_returns_none() {
    assert_eq!(tuple_element("[A, B]", 2), None);
}

#[test]
fn tuple_element_rejects_non_tuple() {
    assert_eq!(tuple_element("Foo<Bar>", 0), None);
    assert_eq!(tuple_element("string", 0), None);
    assert_eq!(tuple_element("", 0), None);
}

#[test]
fn test_scope_chain_single() {
    let chain = build_scope_chain(Some("Namespace"));
    assert_eq!(chain, vec!["Namespace"]);
}

#[test]
fn test_scope_chain_empty() {
    assert!(build_scope_chain(None).is_empty());
    assert!(build_scope_chain(Some("")).is_empty());
}

#[test]
fn test_symbol_index_by_name() {
    // Build a minimal ParsedFile + symbol_id_map
    let pf = ParsedFile {
        path: "src/foo.cs".to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![
            ExtractedSymbol {
                name: "Foo".to_string(),
                qualified_name: "NS.Foo".to_string(),
                kind: SymbolKind::Class,
                visibility: Some(Visibility::Public),
                start_line: 1,
                end_line: 10,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: Some("NS".to_string()),
                parent_index: None,
                byte_offset: 0,
                            declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
},
        ],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("src/foo.cs".to_string(), "NS.Foo".to_string()), 42);

    let index = SymbolIndex::build(&[pf], &id_map);

    // by_name
    let results = index.by_name("Foo");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 42);
    assert_eq!(results[0].qualified_name, "NS.Foo");

    // by_qualified_name
    let result = index.by_qualified_name("NS.Foo");
    assert!(result.is_some());
    assert_eq!(result.unwrap().id, 42);

    // in_namespace
    let ns_results = index.in_namespace("NS");
    assert_eq!(ns_results.len(), 1);

    // in_file
    let file_results = index.in_file("src/foo.cs");
    assert_eq!(file_results.len(), 1);

    // missing
    assert!(index.by_name("Bar").is_empty());
    assert!(index.by_qualified_name("NS.Bar").is_none());
}

#[test]
fn class_symbol_return_type_id_interned_into_arena() {
    // A Class symbol's return_type is its own qualified name. The post-merge
    // intern pass turns that string into a TypeId; the arena resolves it
    // back to a Type::Class.
    let pf = ParsedFile {
        path: "src/foo.cs".to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "Foo".to_string(),
            qualified_name: "NS.Foo".to_string(),
            kind: SymbolKind::Class,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 10,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: Some("NS".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("src/foo.cs".to_string(), "NS.Foo".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    // String surface still works.
    assert_eq!(index.return_type_name("NS.Foo"), Some("NS.Foo"));

    // TypeId surface: same data, interned.
    let rt_id = index.return_type_id("NS.Foo").expect("class has return_type_id");
    let arena = index.type_arena().expect("SymbolIndex exposes an arena");
    match arena.get(rt_id) {
        Type::Class(q) => assert_eq!(q, "NS.Foo"),
        other => panic!("expected Type::Class, got {other:?}"),
    }
}

#[test]
fn signature_derived_return_type_id_interned() {
    // A Method symbol with a .NET-style signature (`(args): ReturnType`) and
    // no TypeRef refs gets its return type parsed via
    // parse_return_type_from_signature, then interned into the arena. Before
    // the build_with_context loop fix, an early-continue on empty type_refs
    // skipped this fallback entirely — only `augment_from_parsed` ran it.
    let pf = ParsedFile {
        path: "ext:lib/foo.dll".to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "GetUser".to_string(),
            qualified_name: "Svc.GetUser".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some("GetUser(int): User".to_string()),
            doc_comment: None,
            scope_path: Some("Svc".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("ext:lib/foo.dll".to_string(), "Svc.GetUser".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    assert_eq!(index.return_type_name("Svc.GetUser"), Some("User"));
    let rt_id = index.return_type_id("Svc.GetUser").expect("method has return_type_id");
    let arena = index.type_arena().expect("SymbolIndex exposes an arena");
    match arena.get(rt_id) {
        Type::Class(q) => assert_eq!(q, "User"),
        other => panic!("expected Type::Class('User'), got {other:?}"),
    }
}

#[test]
fn module_tagged_usage_typeref_feeds_field_type_binding_excluded() {
    // EXT-2: a field whose type is a module-tagged USAGE TypeRef now feeds
    // field_type; the import STATEMENT's own binding ref (is_import_binding)
    // stays excluded so it is never mis-attributed as a type. The binding ref
    // is placed FIRST — were it not excluded, field_type would wrongly take its
    // target ("Account") as the first TypeRef instead of the usage's ("User").
    use crate::types::{EdgeKind, ExtractedRef};
    let pf = ParsedFile {
        path: "ext:pkg/svc.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "repo".to_string(),
            qualified_name: "Svc.repo".to_string(),
            kind: SymbolKind::Property,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: Some("Svc".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![
            ExtractedRef {
                is_import_binding: true,
                is_reexport: false,
                source_symbol_index: 0,
                target_name: "Account".to_string(),
                kind: EdgeKind::TypeRef,
                line: 0,
                col: 0,
                module: Some("@pkg".to_string()),
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            },
            ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: 0,
                target_name: "User".to_string(),
                kind: EdgeKind::TypeRef,
                line: 0,
                col: 0,
                module: Some("@pkg".to_string()),
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            },
        ],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("ext:pkg/svc.ts".to_string(), "Svc.repo".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    // The module-tagged USAGE TypeRef fed the field type; the binding ("Account",
    // placed first) was excluded, so field_type is "User", not "Account".
    assert_eq!(index.field_type_name("Svc.repo"), Some("User"));
}

#[test]
fn signature_derived_return_type_arrow_form() {
    // An external Method with a Python-style arrow signature (`(args) -> Ret`)
    // and no TypeRef refs gets its return type via the arrow-form scan in
    // parse_return_type_from_signature, flowing through build_with_context's
    // signature fallback exactly like the .NET colon form above.
    let pf = ParsedFile {
        path: "ext:site-packages/repo.py".to_string(),
        language: "python".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "find_one".to_string(),
            qualified_name: "Repo.find_one".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some("find_one(self, id) -> User".to_string()),
            doc_comment: None,
            scope_path: Some("Repo".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(
        ("ext:site-packages/repo.py".to_string(), "Repo.find_one".to_string()),
        1,
    );

    let index = SymbolIndex::build(&[pf], &id_map);

    assert_eq!(index.return_type_name("Repo.find_one"), Some("User"));
}

#[test]
fn signature_derived_return_type_jvm_descriptor() {
    // An external JVM Method whose signature is a raw bytecode descriptor
    // (`(params)Ret`) and has no TypeRef refs gets its return type via the
    // JVM-descriptor decoder rung, gated on the JVM language set. The array
    // element form `[L...;` resolves through to the element type.
    let pf = ParsedFile {
        path: "ext:maven/repo.class".to_string(),
        language: "java".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "findOne".to_string(),
            qualified_name: "com.foo.Repo.findOne".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some("(Ljava/lang/String;)Lcom/foo/Bar;".to_string()),
            doc_comment: None,
            scope_path: Some("com.foo.Repo".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(
        ("ext:maven/repo.class".to_string(), "com.foo.Repo.findOne".to_string()),
        1,
    );

    let index = SymbolIndex::build(&[pf], &id_map);

    assert_eq!(index.return_type_name("com.foo.Repo.findOne"), Some("com.foo.Bar"));
}

#[test]
fn c_external_struct_return_hydrates() {
    // A C external function returning an aggregate type carries a leading-form
    // signature whose first depth-0 token is the `struct` keyword. The return
    // type is the elaborated-type-specifier's next token; it hydrates via the
    // positional rung after the colon/arrow parse declines.
    let pf = ParsedFile {
        path: "ext:c:curl/curl.h".to_string(),
        language: "c".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "curl_slist_append".to_string(),
            qualified_name: "curl_slist_append".to_string(),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some(
                "struct curl_slist curl_slist_append(struct curl_slist list, char string)"
                    .to_string(),
            ),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("ext:c:curl/curl.h".to_string(), "curl_slist_append".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    assert_eq!(index.return_type_name("curl_slist_append"), Some("curl_slist"));
}

#[test]
fn set_inferred_return_gap_fills_only_when_return_absent() {
    // INFER-3: `makeUser` has no declared/signature return; `Svc.GetUser` has a
    // signature-derived one. set_inferred_return must fill the gap on the first
    // and refuse to override the second — and report newly-filled via its bool.
    let pf = ParsedFile {
        path: "a.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![
            ExtractedSymbol {
                name: "makeUser".to_string(),
                qualified_name: "makeUser".to_string(),
                kind: SymbolKind::Function,
                visibility: Some(Visibility::Public),
                start_line: 1,
                end_line: 3,
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
            },
            ExtractedSymbol {
                name: "GetUser".to_string(),
                qualified_name: "Svc.GetUser".to_string(),
                kind: SymbolKind::Method,
                visibility: Some(Visibility::Public),
                start_line: 5,
                end_line: 5,
                start_col: 0,
                end_col: 0,
                signature: Some("GetUser(): User".to_string()),
                doc_comment: None,
                scope_path: Some("Svc".to_string()),
                parent_index: None,
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            },
        ],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("a.ts".to_string(), "makeUser".to_string()), 1);
    id_map.insert(("a.ts".to_string(), "Svc.GetUser".to_string()), 2);

    let mut index = SymbolIndex::build(&[pf], &id_map);

    // makeUser has no return → the inferred return fills the gap and reports true.
    assert_eq!(index.return_type_name("makeUser"), None);
    assert!(index.set_inferred_return("makeUser".to_string(), "User".to_string()));
    assert_eq!(index.return_type_name("makeUser"), Some("User"));
    // Re-applying does NOT override and reports false (drives fixpoint convergence).
    assert!(!index.set_inferred_return("makeUser".to_string(), "Admin".to_string()));
    assert_eq!(index.return_type_name("makeUser"), Some("User"));

    // Svc.GetUser has a signature-derived return → never overridden by inference.
    assert_eq!(index.return_type_name("Svc.GetUser"), Some("User"));
    assert!(!index.set_inferred_return("Svc.GetUser".to_string(), "Wrong".to_string()));
    assert_eq!(index.return_type_name("Svc.GetUser"), Some("User"));
}

#[test]
fn join_inferred_returns_skips_conflicts_and_cross_file_collisions() {
    use crate::indexer::resolve::loop_body::join_inferred_returns;
    let c = |q: &str, id: i64, t: &str| (q.to_string(), id, t.to_string());

    // Agreement: one function (db_id 1), two returns both "User" → infer User.
    let agree = vec![c("makeUser", 1, "User"), c("makeUser", 1, "User")];
    assert_eq!(
        join_inferred_returns(&agree, |_| false).get("makeUser"),
        Some(&"User".to_string())
    );

    // Conflict: one function, two returns of different types → infer nothing.
    let conflict = vec![c("pick", 2, "User"), c("pick", 2, "Account")];
    assert!(join_inferred_returns(&conflict, |_| false).is_empty());

    // Cross-file collision: two distinct functions (db_id 3 and 4) share the
    // qname "helper" and even agree on the type — still infer nothing, because
    // the qname-keyed type map cannot tell them apart (applying one's return to
    // the other would be wrong).
    let collision = vec![c("helper", 3, "User"), c("helper", 4, "User")];
    assert!(join_inferred_returns(&collision, |_| false).is_empty());

    // already_known filters out a qname that already carries a return.
    assert!(join_inferred_returns(&agree, |q| q == "makeUser").is_empty());
}

#[test]
fn method_return_type_id_interned_via_typeref() {
    // A Method symbol with a `TypeRef` ref to "User" gets `return_type`
    // populated from that ref. The post-merge intern pass turns the string
    // into a TypeId; the arena resolves back to a Type::Class.
    let pf = ParsedFile {
        path: "src/svc.cs".to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "GetUser".to_string(),
            qualified_name: "Svc.GetUser".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: Some("Svc".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![crate::types::ExtractedRef { is_import_binding: false, is_reexport: false,
            kind: crate::types::EdgeKind::TypeRef,
            source_symbol_index: 0,
            target_name: "User".to_string(),
            line: 1,
            col: 0,
            byte_offset: 0,
            module: None,
            namespace_segments: Vec::new(),
            chain: None,
            call_args: Vec::new(),
        }],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("src/svc.cs".to_string(), "Svc.GetUser".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    // TypeRef-derived path lands in both the string and TypeId surfaces.
    assert_eq!(index.return_type_name("Svc.GetUser"), Some("User"));
    let rt_id = index.return_type_id("Svc.GetUser").expect("method has return_type_id");
    let arena = index.type_arena().expect("SymbolIndex exposes an arena");
    match arena.get(rt_id) {
        Type::Class(q) => assert_eq!(q, "User"),
        other => panic!("expected Type::Class('User'), got {other:?}"),
    }
}

#[test]
fn generic_return_type_decomposes_into_apply() {
    // A method returning `Repository<User>` lands in TypeInfo.return_type as
    // the string "Repository<User>". The post-merge intern pass decomposes
    // it structurally into `Apply(Class("Repository"), [Class("User")])`,
    // letting downstream consumers separately resolve the base and the
    // type args.
    let pf = ParsedFile {
        path: "src/svc.cs".to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "GetUserRepo".to_string(),
            qualified_name: "Svc.GetUserRepo".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: Some("Svc".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![crate::types::ExtractedRef { is_import_binding: false, is_reexport: false,
            kind: crate::types::EdgeKind::TypeRef,
            source_symbol_index: 0,
            target_name: "Repository<User>".to_string(),
            line: 1,
            col: 0,
            byte_offset: 0,
            module: None,
            namespace_segments: Vec::new(),
            chain: None,
            call_args: Vec::new(),
        }],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("src/svc.cs".to_string(), "Svc.GetUserRepo".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    let rt_id = index
        .return_type_id("Svc.GetUserRepo")
        .expect("method has return_type_id");
    let arena = index.type_arena().expect("SymbolIndex exposes an arena");
    match arena.get(rt_id) {
        Type::Apply { base, args } => {
            assert_eq!(args.len(), 1);
            assert_eq!(arena.get(base), Type::Class("Repository".to_string()));
            assert_eq!(arena.get(args[0]), Type::Class("User".to_string()));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn structural_return_type_records_no_generic_args() {
    // A colon-form return that is a STRUCTURAL type wrapping an inner generic
    // (tuple `[A, B<C>]`, union, function type) splits at the first `<` into a
    // non-identifier head. Capture must treat it as non-generic — no element
    // args recorded — so the chain walker never binds a bogus element type.
    let pf = ParsedFile {
        path: "src/svc.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "getTuple".to_string(),
            qualified_name: "Svc.getTuple".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some("getTuple(): [string, Promise<number>]".to_string()),
            doc_comment: None,
            scope_path: Some("Svc".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("src/svc.ts".to_string(), "Svc.getTuple".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    assert!(
        index.return_type_args("Svc.getTuple").is_none(),
        "structural return must not record generic element args, got {:?}",
        index.return_type_args("Svc.getTuple")
    );
}

#[test]
fn leading_form_generic_return_records_element_args() {
    // Java/C# `RetType name(params)` is leading-form: the return type is the
    // first signature token. A generic application binds its element args even
    // with NO return-type TypeRefs (the Java extractor emits none for methods),
    // so the signature is the sole source.
    let pf = ParsedFile {
        path: "src/Repo.java".to_string(),
        language: "java".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "getItems".to_string(),
            qualified_name: "Repo.getItems".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some("List<User> getItems()".to_string()),
            doc_comment: None,
            scope_path: Some("Repo".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    let mut id_map = HashMap::new();
    id_map.insert(("src/Repo.java".to_string(), "Repo.getItems".to_string()), 1);

    let index = SymbolIndex::build(&[pf], &id_map);

    assert_eq!(index.return_type_name("Repo.getItems"), Some("List"));
    assert_eq!(
        index.return_type_args("Repo.getItems").map(|a| a.to_vec()),
        Some(vec!["User".to_string()])
    );
}

/// A ParsedFile with one Method symbol and the given return-type TypeRef target
/// names — for exercising the method-return capture branch.
fn method_return_pf(
    path: &str,
    language: &str,
    qname: &str,
    scope: &str,
    signature: &str,
    ret_refs: &[&str],
) -> ParsedFile {
    let refs = ret_refs
        .iter()
        .map(|t| crate::types::ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            kind: crate::types::EdgeKind::TypeRef,
            source_symbol_index: 0,
            target_name: t.to_string(),
            line: 1,
            col: 0,
            byte_offset: 0,
            module: None,
            namespace_segments: Vec::new(),
            chain: None,
            call_args: Vec::new(),
        })
        .collect();
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: qname.rsplit('.').next().unwrap().to_string(),
            qualified_name: qname.to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some(signature.to_string()),
            doc_comment: None,
            scope_path: Some(scope.to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
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

#[test]
fn go_bracket_generic_return_records_element_args() {
    // Go expresses generics with `[]` and trails the return type after the
    // params — `parse_return_type_trailing` + the `[]` split bind the element.
    let pf = method_return_pf(
        "src/repo.go",
        "go",
        "Repo.Items",
        "Repo",
        "func (r *Repo) Items() Result[User]",
        &[],
    );
    let mut id_map = HashMap::new();
    id_map.insert(("src/repo.go".to_string(), "Repo.Items".to_string()), 1);
    let index = SymbolIndex::build(&[pf], &id_map);
    assert_eq!(index.return_type_name("Repo.Items"), Some("Result"));
    assert_eq!(
        index.return_type_args("Repo.Items").map(|a| a.to_vec()),
        Some(vec!["User".to_string()])
    );
}

#[test]
fn leading_form_head_preferred_over_trailing_param_ref() {
    // A return-first ref list (C# shape) ends on the last PARAM, so `last()` is
    // the wrong head. The plain signature return type must win.
    let pf = method_return_pf(
        "src/Svc.cs",
        "csharp",
        "Svc.GetName",
        "Svc",
        "Foo GetName(Bar b)",
        &["Foo", "Bar"],
    );
    let mut id_map = HashMap::new();
    id_map.insert(("src/Svc.cs".to_string(), "Svc.GetName".to_string()), 1);
    let index = SymbolIndex::build(&[pf], &id_map);
    assert_eq!(index.return_type_name("Svc.GetName"), Some("Foo"));
}

#[test]
fn default_symbol_lookup_returns_no_typeid_surface() {
    // Synthetic SymbolLookup impls that don't override the TypeId methods
    // must return None across the surface — confirms the default impls
    // are the opt-out path.
    struct EmptyLookup;
    impl SymbolLookup for EmptyLookup {
        fn by_name(&self, _: &str) -> &[SymbolInfo] {
            &[]
        }
        fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
            None
        }
        fn members_of(&self, _: &str) -> &[SymbolInfo] {
            &[]
        }
        fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
            &[]
        }
        fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
            Vec::new()
        }
        fn has_in_namespace(&self, _: &str) -> bool {
            false
        }
        fn in_file(&self, _: &str) -> &[SymbolInfo] {
            &[]
        }
        fn field_type_name(&self, _: &str) -> Option<&str> {
            None
        }
        fn return_type_name(&self, _: &str) -> Option<&str> {
            None
        }
        fn field_type_args(&self, _: &str) -> Option<&[String]> {
            None
        }
        fn generic_params(&self, _: &str) -> Option<&[String]> {
            None
        }
        fn reexports_from(&self, _: &str) -> &[(String, String)] {
            &[]
        }
        fn is_external_name(&self, _: &str, _: &str) -> bool {
            false
        }
    }

    let lookup = EmptyLookup;
    assert!(lookup.field_type_id("anything").is_none());
    assert!(lookup.return_type_id("anything").is_none());
    assert!(lookup.field_type_arg_ids("anything").is_none());
    assert!(lookup.type_arena().is_none());
}

fn make_class_sym(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Class,
        visibility: None,
        start_line: 1,
        end_line: 2,
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
}
}

fn make_pf(path: &str, syms: Vec<ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
        refs: vec![],
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

#[test]
fn test_in_namespace_sorted_multiple() {
    let pf = make_pf(
        "src/a.cs",
        vec![
            make_class_sym("Foo", "NS.Foo"),
            make_class_sym("Bar", "NS.Bar"),
            make_class_sym("Baz", "Other.Baz"),
        ],
    );

    let mut id_map = HashMap::new();
    id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
    id_map.insert(("src/a.cs".to_string(), "NS.Bar".to_string()), 2);
    id_map.insert(("src/a.cs".to_string(), "Other.Baz".to_string()), 3);

    let index = SymbolIndex::build(&[pf], &id_map);

    let ns_results = index.in_namespace("NS");
    assert_eq!(ns_results.len(), 2);
    let mut ids: Vec<i64> = ns_results.iter().map(|s| s.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2]);

    let other_results = index.in_namespace("Other");
    assert_eq!(other_results.len(), 1);
    assert_eq!(other_results[0].id, 3);
}

#[test]
fn test_in_namespace_no_prefix_bleed() {
    // "NS" must not match "NSX.Thing"
    let pf = make_pf(
        "src/a.cs",
        vec![
            make_class_sym("Foo", "NS.Foo"),
            make_class_sym("Thing", "NSX.Thing"),
        ],
    );

    let mut id_map = HashMap::new();
    id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
    id_map.insert(("src/a.cs".to_string(), "NSX.Thing".to_string()), 2);

    let index = SymbolIndex::build(&[pf], &id_map);

    let ns_results = index.in_namespace("NS");
    assert_eq!(ns_results.len(), 1);
    assert_eq!(ns_results[0].qualified_name, "NS.Foo");
}

#[test]
fn test_in_namespace_empty() {
    let pf = make_pf("src/a.cs", vec![make_class_sym("Foo", "NS.Foo")]);
    let mut id_map = HashMap::new();
    id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
    let index = SymbolIndex::build(&[pf], &id_map);

    assert!(index.in_namespace("Missing").is_empty());
    assert!(index.in_namespace("N").is_empty()); // "N" is a prefix of "NS" but not "NS."
}

// -----------------------------------------------------------------
// R5 per-file flow-typing cache — synthetic tests
//
// Verify the `LocalTypeCache` round-trip:
//   • forward inference via `forward` map (reassignment = last write wins)
//   • cursor-based narrowing lookup
//   • innermost narrowing wins on overlap
//   • clear_local_cache wipes both maps
//
// These tests operate directly on `SymbolIndex` via the SymbolLookup
// trait; the resolver-loop wiring is exercised indirectly through
// `install_local_cache` + `record_local_type` + `local_type`.
// -----------------------------------------------------------------

fn make_empty_index() -> SymbolIndex {
    let id_map: HashMap<(String, String), i64> = HashMap::new();
    SymbolIndex::build(&[], &id_map)
}

fn narrowing(name: &str, ty: &str, start: u32, end: u32) -> crate::types::Narrowing {
    crate::types::Narrowing {
        name: name.to_string(),
        narrowed_type: ty.to_string(),
        byte_start: start,
        byte_end: end,
    }
}

#[test]
fn local_cache_forward_inference() {
    let idx = make_empty_index();
    idx.install_local_cache(Vec::new(), Vec::new(), Default::default());
    idx.record_local_type("x".to_string(), "Foo".to_string());
    assert_eq!(idx.local_type("x"), Some("Foo".to_string()));
    assert_eq!(idx.local_type("y"), None);
}

#[test]
fn local_cache_reassignment_last_write_wins() {
    let idx = make_empty_index();
    idx.install_local_cache(Vec::new(), Vec::new(), Default::default());
    idx.record_local_type("x".to_string(), "Foo".to_string());
    idx.record_local_type("x".to_string(), "Bar".to_string());
    assert_eq!(idx.local_type("x"), Some("Bar".to_string()));
}

#[test]
fn local_cache_clear_wipes_bindings() {
    let idx = make_empty_index();
    idx.install_local_cache(vec![narrowing("x", "Bar", 0, 100)], Vec::new(), Default::default());
    idx.record_local_type("x".to_string(), "Foo".to_string());
    idx.clear_local_cache();
    assert_eq!(idx.local_type("x"), None);
}

#[test]
fn local_cache_install_resets_previous_bindings() {
    let idx = make_empty_index();
    idx.install_local_cache(Vec::new(), Vec::new(), Default::default());
    idx.record_local_type("x".to_string(), "Foo".to_string());
    // Simulate moving to the next file: install a fresh cache.
    idx.install_local_cache(Vec::new(), Vec::new(), Default::default());
    assert_eq!(idx.local_type("x"), None);
}

#[test]
fn local_cache_narrowing_honors_cursor() {
    let idx = make_empty_index();
    // Narrowing for `x` as `Bar` valid in byte range [50, 80).
    idx.install_local_cache(vec![narrowing("x", "Bar", 50, 80)], Vec::new(), Default::default());
    // Baseline forward type is `Foo`.
    idx.record_local_type("x".to_string(), "Foo".to_string());

    // Cursor before the range → forward type wins.
    idx.set_cursor(10);
    assert_eq!(idx.local_type("x"), Some("Foo".to_string()));

    // Cursor inside the narrowing range → narrowed type wins.
    idx.set_cursor(60);
    assert_eq!(idx.local_type("x"), Some("Bar".to_string()));

    // Cursor past the range → back to forward type.
    idx.set_cursor(90);
    assert_eq!(idx.local_type("x"), Some("Foo".to_string()));
}

#[test]
fn local_cache_narrowing_upper_bound_exclusive() {
    let idx = make_empty_index();
    idx.install_local_cache(vec![narrowing("x", "Bar", 50, 80)], Vec::new(), Default::default());
    // Exactly at end is outside (half-open range).
    idx.set_cursor(80);
    assert_eq!(idx.local_type("x"), None);
    // One less is inside.
    idx.set_cursor(79);
    assert_eq!(idx.local_type("x"), Some("Bar".to_string()));
}

#[test]
fn local_cache_innermost_narrowing_wins() {
    let idx = make_empty_index();
    // Outer range narrows `x` to `A` across [0, 100), inner range narrows
    // `x` to `B` across [40, 60). Both apply at cursor 50 — innermost
    // (smallest range) must win because `install_local_cache` sorts by
    // ascending range size.
    let narrowings = vec![
        narrowing("x", "A", 0, 100),
        narrowing("x", "B", 40, 60),
    ];
    // The resolver sorts these before install; replicate that here.
    let mut sorted = narrowings.clone();
    sorted.sort_by_key(|n| n.byte_end.saturating_sub(n.byte_start));
    idx.install_local_cache(sorted, Vec::new(), Default::default());

    idx.set_cursor(50);
    assert_eq!(idx.local_type("x"), Some("B".to_string()));

    idx.set_cursor(10);
    assert_eq!(idx.local_type("x"), Some("A".to_string()));
}

#[test]
fn local_cache_default_impls_noop_for_non_symbol_index() {
    // A non-SymbolIndex lookup (synthetic test double) must not crash
    // or change behavior when the resolver calls flow-cache methods.
    struct Empty;
    impl SymbolLookup for Empty {
        fn by_name(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> { None }
        fn members_of(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
        fn has_in_namespace(&self, _: &str) -> bool { false }
        fn in_file(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn field_type_name(&self, _: &str) -> Option<&str> { None }
        fn return_type_name(&self, _: &str) -> Option<&str> { None }
        fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
        fn generic_params(&self, _: &str) -> Option<&[String]> { None }
        fn reexports_from(&self, _: &str) -> &[(String, String)] { &[] }
        fn is_external_name(&self, _: &str, _: &str) -> bool { false }
    }
    let e = Empty;
    // All defaulted methods should be no-ops / None.
    assert_eq!(e.local_type("anything"), None);
    e.install_local_cache(vec![narrowing("x", "Foo", 0, 10)], Vec::new(), Default::default());
    e.set_cursor(5);
    e.record_local_type("x".to_string(), "Foo".to_string());
    e.clear_local_cache();
    // Still None — default impl doesn't record.
    assert_eq!(e.local_type("x"), None);
}

#[test]
fn local_cache_type_cache_generics_roundtrip() {
    // Sanity check: ensure `TypeEnvironment::enter_generic_context` used
    // by the chain walker correctly binds call-site type args so the
    // yield type comes out substituted.
    use crate::type_checker::type_env::TypeEnvironment;
    let mut env = TypeEnvironment::new();
    let pushed = env.enter_generic_context(
        "UserRepo.findOne",
        &["User".to_string()],
        |name| {
            if name == "UserRepo.findOne" {
                Some(vec!["T".to_string()])
            } else {
                None
            }
        },
    );
    assert!(pushed);
    assert_eq!(env.resolve("T"), "User");
}

#[test]
fn augment_from_parsed_generic_return_records_element_args() {
    // An external Method added via augment_from_parsed with a generic return
    // signature (`Repository<User> getRepo()`, no TypeRef refs) must end up
    // with return_type = "Repository" and return_type_args = ["User"].
    let ext_pf = ParsedFile {
        path: "ext:lib/repo.dll".to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![ExtractedSymbol {
            name: "getRepo".to_string(),
            qualified_name: "Svc.getRepo".to_string(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 1,
            start_col: 0,
            end_col: 0,
            signature: Some("Repository<User> getRepo()".to_string()),
            doc_comment: None,
            scope_path: Some("Svc".to_string()),
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: vec![],
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
    };

    // Build an empty seed index, then fold the external file in via augment.
    let seed_index = SymbolIndex::build(&[], &HashMap::new());
    let mut index = seed_index;
    let mut aug_id_map = HashMap::new();
    aug_id_map.insert(
        ("ext:lib/repo.dll".to_string(), "Svc.getRepo".to_string()),
        42i64,
    );
    index.augment_from_parsed(&[ext_pf], &aug_id_map);

    assert_eq!(index.return_type_name("Svc.getRepo"), Some("Repository"));
    assert_eq!(
        index.return_type_args("Svc.getRepo").map(|a| a.to_vec()),
        Some(vec!["User".to_string()])
    );
}
