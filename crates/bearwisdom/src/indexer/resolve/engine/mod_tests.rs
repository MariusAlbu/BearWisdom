// =============================================================================
// indexer/resolve/engine/mod_tests.rs — sibling tests for engine/mod.rs
// =============================================================================

use crate::indexer::resolve::engine::{
    build_scope_chain, ChainMiss, LocalTypeCache, SymbolIndex, SymbolInfo, SymbolLookup,
};
use crate::indexer::resolve::engine::chain_walker::{
    parse_declared_type_from_signature_for_lang, parse_param_types_from_signature,
    parse_param_types_from_signature_for_lang, parse_return_type_from_signature,
    resolve_type_name_in_scope, tuple_element,
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
        refs: vec![crate::types::ExtractedRef {
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
        refs: vec![crate::types::ExtractedRef {
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
    idx.install_local_cache(Vec::new());
    idx.record_local_type("x".to_string(), "Foo".to_string());
    assert_eq!(idx.local_type("x"), Some("Foo".to_string()));
    assert_eq!(idx.local_type("y"), None);
}

#[test]
fn local_cache_reassignment_last_write_wins() {
    let idx = make_empty_index();
    idx.install_local_cache(Vec::new());
    idx.record_local_type("x".to_string(), "Foo".to_string());
    idx.record_local_type("x".to_string(), "Bar".to_string());
    assert_eq!(idx.local_type("x"), Some("Bar".to_string()));
}

#[test]
fn local_cache_clear_wipes_bindings() {
    let idx = make_empty_index();
    idx.install_local_cache(vec![narrowing("x", "Bar", 0, 100)]);
    idx.record_local_type("x".to_string(), "Foo".to_string());
    idx.clear_local_cache();
    assert_eq!(idx.local_type("x"), None);
}

#[test]
fn local_cache_install_resets_previous_bindings() {
    let idx = make_empty_index();
    idx.install_local_cache(Vec::new());
    idx.record_local_type("x".to_string(), "Foo".to_string());
    // Simulate moving to the next file: install a fresh cache.
    idx.install_local_cache(Vec::new());
    assert_eq!(idx.local_type("x"), None);
}

#[test]
fn local_cache_narrowing_honors_cursor() {
    let idx = make_empty_index();
    // Narrowing for `x` as `Bar` valid in byte range [50, 80).
    idx.install_local_cache(vec![narrowing("x", "Bar", 50, 80)]);
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
    idx.install_local_cache(vec![narrowing("x", "Bar", 50, 80)]);
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
    idx.install_local_cache(sorted);

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
    e.install_local_cache(vec![narrowing("x", "Foo", 0, 10)]);
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
