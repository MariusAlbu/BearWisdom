// =============================================================================
// engine/tree_tests.rs — unit tests for Compilation
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::type_checker::core::types::TypeArena;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    parent_index: Option<usize>,
    declared_type: Option<crate::type_checker::core::types::TypeId>,
    return_type: Option<crate::type_checker::core::types::TypeId>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index,
        declared_type,
        return_type,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn inherits_ref(source_symbol_index: usize, parent_name: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index,
        target_name: parent_name.to_string(),
        kind: EdgeKind::Inherits,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
    }
}

fn type_ref(source_symbol_index: usize, type_name: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index,
        target_name: type_name.to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
    }
}

/// Build a minimal `ParsedFile` with the given path, symbols, and refs.
fn make_parsed_file(
    path: &str,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Fixture builder
// ---------------------------------------------------------------------------

/// Constructs the canonical test scenario:
///
/// ```text
/// class Repo {          // index 0
///   find(): User        // index 1 — return_type = arena.class("User")
///   db: Database        // index 2 — declared_type = arena.class("Database")
/// }
/// class UserRepo extends Repo {}  // index 3 — Inherits ref → "Repo"
/// ```
///
/// Returns `(tree, arena)`.
fn build_fixture() -> (Compilation, Arc<TypeArena>) {
    let arena = Arc::new(TypeArena::new());

    let user_id = arena.class("User");
    let db_id = arena.class("Database");

    // Symbols: Repo(0), find(1, parent=0), db(2, parent=0), UserRepo(3)
    let symbols = vec![
        make_symbol("Repo", "Repo", SymbolKind::Class, None, None, None),
        make_symbol(
            "find",
            "Repo.find",
            SymbolKind::Method,
            Some(0),
            None,
            Some(user_id),
        ),
        make_symbol(
            "db",
            "Repo.db",
            SymbolKind::Field,
            Some(0),
            Some(db_id),
            None,
        ),
        make_symbol("UserRepo", "UserRepo", SymbolKind::Class, None, None, None),
    ];

    // UserRepo (index 3) inherits Repo.
    let refs = vec![inherits_ref(3, "Repo")];

    let pf = make_parsed_file("src/repo.ts", symbols, refs);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/repo.ts".to_string(), "Repo".to_string()), 1);
    id_map.insert(("src/repo.ts".to_string(), "Repo.find".to_string()), 2);
    id_map.insert(("src/repo.ts".to_string(), "Repo.db".to_string()), 3);
    id_map.insert(("src/repo.ts".to_string(), "UserRepo".to_string()), 4);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));
    (tree, arena)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn by_qualified_name_finds_class() {
    let (tree, _) = build_fixture();
    let sym = tree.by_qualified_name("Repo").expect("Repo should be indexed");
    assert_eq!(sym.qualified_name, "Repo");
    assert_eq!(sym.kind, "class");
}

#[test]
fn members_of_repo_contains_find() {
    let (tree, _) = build_fixture();
    let members = tree.members_of("Repo");
    let names: Vec<&str> = members.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"find"),
        "members_of(Repo) should contain `find`; got {names:?}"
    );
}

#[test]
fn members_of_id_matches_members_of() {
    let (tree, _) = build_fixture();
    let repo_id = tree.by_qualified_name("Repo").expect("Repo should be indexed").id;
    let qname_set = tree.members_of("Repo");
    let id_set = tree.members_of_id(repo_id);
    let mut by_qname: Vec<&str> = qname_set.iter().map(|s| s.name.as_str()).collect();
    let mut by_id: Vec<&str> = id_set.iter().map(|s| s.name.as_str()).collect();
    by_qname.sort_unstable();
    by_id.sort_unstable();
    assert_eq!(
        by_id, by_qname,
        "members_of_id(Repo) must equal members_of(\"Repo\")"
    );
    assert!(
        by_id.contains(&"find"),
        "members_of_id(Repo) should contain `find`; got {by_id:?}"
    );
}

/// `lib.es5.d.ts` declares every builtin as BOTH an instance `interface Array`
/// and a constructor-typed `declare var Array: ArrayConstructor`, sharing the
/// single qname `Array`. The value's `ArrayConstructor` field_type must not land
/// in the qname-keyed `type_info` slot the instance interface owns: reading
/// `field_type("Array")` to seed a literal-typed local (`const xs = []`) would
/// otherwise bind the receiver to `ArrayConstructor`, which has `from`/`of` but
/// none of `push`/`includes`/`map`.
#[test]
fn constructor_var_does_not_poison_instance_type_field_slot() {
    let arena = Arc::new(TypeArena::new());
    let symbols = vec![
        make_symbol("Array", "Array", SymbolKind::Interface, None, None, None),
        make_symbol("push", "Array.push", SymbolKind::Method, Some(0), None, None),
        make_symbol("Array", "Array", SymbolKind::Variable, None, None, None),
    ];
    // `declare var Array: ArrayConstructor` — the value's declared type.
    let refs = vec![type_ref(2, "ArrayConstructor")];
    let pf = make_parsed_file("lib.es5.d.ts", symbols, refs);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("lib.es5.d.ts".to_string(), "Array".to_string()), 1);
    id_map.insert(("lib.es5.d.ts".to_string(), "Array.push".to_string()), 2);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_ne!(
        tree.field_type_str("Array").as_deref(),
        Some("ArrayConstructor"),
        "the constructor value's field_type must not poison the instance \
         interface's qname slot"
    );
}

/// A field/value type duplicated across packages shares one qname, so the
/// qname-keyed `type_info` field slot is first-writer-wins. The id-keyed slot
/// (`field_type_id_of`) must keep each declaration's own field type distinct.
#[test]
fn colliding_qname_field_types_are_kept_per_id() {
    let arena = Arc::new(TypeArena::new());

    let cfg_a = make_symbol("config", "config", SymbolKind::Variable, None, None, None);
    let cfg_b = make_symbol("config", "config", SymbolKind::Variable, None, None, None);

    let pf_a =
        make_parsed_file("packages/a/config.ts", vec![cfg_a], vec![type_ref(0, "ConfigA")]);
    let pf_b =
        make_parsed_file("packages/b/config.ts", vec![cfg_b], vec![type_ref(0, "ConfigB")]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("packages/a/config.ts".to_string(), "config".to_string()), 10);
    id_map.insert(("packages/b/config.ts".to_string(), "config".to_string()), 20);

    let tree = Compilation::build(&[pf_a, pf_b], &id_map, Arc::clone(&arena));

    let id10 = tree.field_type_id_of(10).expect("field_type_id_of(10) populated");
    let id20 = tree.field_type_id_of(20).expect("field_type_id_of(20) populated");
    assert_ne!(
        id10, id20,
        "id-keyed field types of a colliding qname must stay distinct"
    );
    assert_eq!(arena.format_type(id10), "ConfigA");
    assert_eq!(arena.format_type(id20), "ConfigB");
}

#[test]
fn symbol_by_id_recovers_the_record() {
    let (tree, _) = build_fixture();
    let repo_id = tree.by_qualified_name("Repo").expect("Repo indexed").id;
    let recovered = tree.symbol_by_id(repo_id).expect("symbol_by_id recovers Repo");
    assert_eq!(recovered.qualified_name, "Repo");
}

/// A public API duplicated across monorepo packages shares one qname, so the
/// qname-keyed `type_info` return slot is first-writer-wins. The id-keyed slot
/// (`return_type_id_of`) must keep each declaration's own return distinct.
#[test]
fn colliding_qname_return_types_are_kept_per_id() {
    let arena = Arc::new(TypeArena::new());

    let mut react_fn =
        make_symbol("useQuery", "useQuery", SymbolKind::Function, None, None, None);
    react_fn.signature = Some("function useQuery(): ReactResult".to_string());
    let mut preact_fn =
        make_symbol("useQuery", "useQuery", SymbolKind::Function, None, None, None);
    preact_fn.signature = Some("function useQuery(): PreactResult".to_string());

    let react_pf = make_parsed_file("packages/react/useQuery.ts", vec![react_fn], vec![]);
    let preact_pf = make_parsed_file("packages/preact/useQuery.ts", vec![preact_fn], vec![]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        ("packages/react/useQuery.ts".to_string(), "useQuery".to_string()),
        10,
    );
    id_map.insert(
        ("packages/preact/useQuery.ts".to_string(), "useQuery".to_string()),
        20,
    );

    let tree = Compilation::build(&[react_pf, preact_pf], &id_map, Arc::clone(&arena));

    // Qname slot is first-writer-wins (one type for both copies); the id slot
    // must keep each declaration's OWN return.
    let react_ret = tree
        .return_type_id_of(10)
        .expect("react useQuery should have a return type by id");
    let preact_ret = tree
        .return_type_id_of(20)
        .expect("preact useQuery should have a return type by id");

    assert_eq!(arena.format_type(react_ret), "ReactResult");
    assert_eq!(arena.format_type(preact_ret), "PreactResult");
    assert_ne!(
        react_ret, preact_ret,
        "same-qname overloads must keep distinct returns by id"
    );
}

/// `useQuery` declared with generic signatures in two packages: the qname slot
/// collapses to one first-winner, but each declaration's id must keep its OWN
/// params — the fix for a name shared across packages / doc fences losing the
/// real declarations' generics to whichever wins the qname slot.
#[test]
fn same_qname_overload_generics_survive_by_id() {
    let arena = Arc::new(TypeArena::new());
    let mut react_fn =
        make_symbol("useQuery", "useQuery", SymbolKind::Function, None, None, None);
    react_fn.signature = Some("function useQuery<TData, TError>(): ReactResult".to_string());
    let mut preact_fn =
        make_symbol("useQuery", "useQuery", SymbolKind::Function, None, None, None);
    preact_fn.signature = Some("function useQuery<TData, TError>(): PreactResult".to_string());

    let react_pf = make_parsed_file("packages/react/useQuery.ts", vec![react_fn], vec![]);
    let preact_pf = make_parsed_file("packages/preact/useQuery.ts", vec![preact_fn], vec![]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("packages/react/useQuery.ts".to_string(), "useQuery".to_string()), 10);
    id_map.insert(("packages/preact/useQuery.ts".to_string(), "useQuery".to_string()), 20);

    let tree = Compilation::build(&[react_pf, preact_pf], &id_map, Arc::clone(&arena));

    let expected = vec!["TData".to_string(), "TError".to_string()];
    assert_eq!(
        tree.generic_params_of(10),
        Some(expected.clone()),
        "react useQuery must keep its generics by id despite the shared qname"
    );
    assert_eq!(
        tree.generic_params_of(20),
        Some(expected),
        "preact useQuery must keep its generics by id despite the shared qname"
    );
}

/// `<TData = string, TError = TData>` — a parameter's default binds it when the
/// call site leaves it unbound; `TError` defaults to the earlier `TData`. Captured
/// index-aligned with the params.
#[test]
fn generic_param_defaults_are_captured_by_id() {
    let arena = Arc::new(TypeArena::new());
    let mut f = make_symbol("useQuery", "useQuery", SymbolKind::Function, None, None, None);
    f.signature = Some("function useQuery<TData = string, TError = TData>(): R".to_string());
    let pf = make_parsed_file("a.ts", vec![f], vec![]);
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("a.ts".to_string(), "useQuery".to_string()), 10);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    let expected = vec![Some("string".to_string()), Some("TData".to_string())];
    assert_eq!(
        tree.generic_param_defaults_of(10),
        Some(expected),
        "defaults `= string` / `= TData` must be captured index-aligned"
    );
}

/// `function usePost() { return useQuery() }` — usePost's return is inferred
/// from the returned call's return type, so a `usePost().data` chain can root.
#[test]
fn call_wrapper_return_inferred_from_returned_call() {
    use crate::indexer::resolve::engine::testkit::call_ref;

    let arena = Arc::new(TypeArena::new());
    let mut uq = make_symbol("useQuery", "useQuery", SymbolKind::Function, None, None, None);
    uq.signature = Some("function useQuery(): UQR".to_string());
    let usepost = make_symbol("usePost", "usePost", SymbolKind::Function, None, None, None);

    // The returned `useQuery()` call (ref 0), whose enclosing function is usePost.
    let mut r = call_ref("useQuery");
    r.source_symbol_index = 1;
    let mut pf = make_parsed_file("src/hooks.ts", vec![uq, usepost], vec![r]);
    pf.flow.flow_return_lhs.insert(0, 1);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/hooks.ts".to_string(), "useQuery".to_string()), 1);
    id_map.insert(("src/hooks.ts".to_string(), "usePost".to_string()), 2);

    let mut tree = Compilation::build(std::slice::from_ref(&pf), &id_map, Arc::clone(&arena));
    // useQuery's own return derives from its signature; usePost's is inferred.
    assert_eq!(tree.return_type_str("usePost"), None, "no return before the pass");
    tree.infer_call_wrapper_returns(std::slice::from_ref(&pf));
    assert_eq!(
        tree.return_type_str("usePost").as_deref(),
        Some("UQR"),
        "usePost's return inferred from `return useQuery()`"
    );
}

/// A factory `createScopedLogger() { return createLogger() }` whose nested
/// `createLogger` shares its bare name with a top-level `createLogger` in another
/// scope must bind its OWN nested builder's `$Ret`, not the namesake's. The
/// returned-call inference resolves `createLogger` with the enclosing factory's
/// scope so `{enclosing}.createLogger` wins over the unscoped namesake.
#[test]
fn call_wrapper_return_prefers_scoped_nested_callee_over_namesake() {
    use crate::indexer::resolve::engine::testkit::call_ref;

    let arena = Arc::new(TypeArena::new());
    // Top-level namesake `createLogger` + its `$Ret` (a DIFFERENT shape) — listed
    // first so the unscoped `by_name` path would pick it.
    let namesake = make_symbol("createLogger", "createLogger", SymbolKind::Function, None, None, None);
    let namesake_ret =
        make_symbol("createLogger$Ret", "createLogger$Ret", SymbolKind::Interface, None, None, None);
    let namesake_member =
        make_symbol("ship", "createLogger$Ret.ship", SymbolKind::Property, Some(1), None, None);
    // The factory + its nested builder + the builder's member-bearing `$Ret`.
    let factory =
        make_symbol("createScopedLogger", "createScopedLogger", SymbolKind::Function, None, None, None);
    let mut nested = make_symbol(
        "createLogger",
        "createScopedLogger.createLogger",
        SymbolKind::Function,
        None,
        None,
        None,
    );
    nested.scope_path = Some("createScopedLogger".to_string());
    let nested_ret = make_symbol(
        "createLogger$Ret",
        "createScopedLogger.createLogger$Ret",
        SymbolKind::Interface,
        None,
        None,
        None,
    );
    let nested_member = make_symbol(
        "info",
        "createScopedLogger.createLogger$Ret.info",
        SymbolKind::Property,
        Some(5),
        None,
        None,
    );

    // `return createLogger()` inside the factory (symbol index 3).
    let mut r = call_ref("createLogger");
    r.source_symbol_index = 3;
    let mut pf = make_parsed_file(
        "src/logger.ts",
        vec![namesake, namesake_ret, namesake_member, factory, nested, nested_ret, nested_member],
        vec![r],
    );
    pf.flow.flow_return_lhs.insert(0, 3);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    for (qname, id) in [
        ("createLogger", 100),
        ("createLogger$Ret", 101),
        ("createLogger$Ret.ship", 102),
        ("createScopedLogger", 103),
        ("createScopedLogger.createLogger", 104),
        ("createScopedLogger.createLogger$Ret", 105),
        ("createScopedLogger.createLogger$Ret.info", 106),
    ] {
        id_map.insert(("src/logger.ts".to_string(), qname.to_string()), id);
    }

    let mut tree = Compilation::build(std::slice::from_ref(&pf), &id_map, Arc::clone(&arena));
    assert_eq!(tree.return_type_str("createScopedLogger"), None, "no return before the pass");
    tree.infer_call_wrapper_returns(std::slice::from_ref(&pf));
    assert_eq!(
        tree.return_type_str("createScopedLogger").as_deref(),
        Some("createScopedLogger.createLogger$Ret"),
        "factory must bind its OWN nested builder's $Ret, not the top-level namesake's"
    );
}

/// `class C { svc = makeThing() }` — the field's type is the initializer call's
/// return, so `this.svc.member()` can root on it.
#[test]
fn field_init_call_types_the_field() {
    use crate::indexer::resolve::engine::testkit::call_ref;

    let arena = Arc::new(TypeArena::new());
    let mut mk = make_symbol("makeThing", "makeThing", SymbolKind::Function, None, None, None);
    mk.signature = Some("function makeThing(): Thing".to_string());
    let c = make_symbol("C", "C", SymbolKind::Class, None, None, None);
    let svc = make_symbol("svc", "C.svc", SymbolKind::Property, Some(1), None, None);

    // `svc = makeThing()` — the initializer call, attributed to the field (idx 2).
    let mut r = call_ref("makeThing");
    r.source_symbol_index = 2;
    let pf = make_parsed_file("src/c.ts", vec![mk, c, svc], vec![r]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/c.ts".to_string(), "makeThing".to_string()), 1);
    id_map.insert(("src/c.ts".to_string(), "C".to_string()), 2);
    id_map.insert(("src/c.ts".to_string(), "C.svc".to_string()), 3);

    let mut tree = Compilation::build(std::slice::from_ref(&pf), &id_map, Arc::clone(&arena));
    assert_eq!(tree.field_type_str("C.svc"), None, "no field type before the pass");
    tree.infer_field_init_types(std::slice::from_ref(&pf), &rustc_hash::FxHashMap::default());
    assert_eq!(
        tree.field_type_str("C.svc").as_deref(),
        Some("Thing"),
        "field typed from its initializer call's return",
    );
}

/// `const res = await fetch()` — the initializer's raw return type is the
/// async wrapper itself (`Promise<Response>`), not what `await` yields
/// (`Response`). `flow_binding_await` names the binding as awaited;
/// `infer_field_init_types` must peel one wrapper layer (per
/// `profile.async_wrappers`) before recording the field's type.
#[test]
fn field_init_await_unwraps_the_async_wrapper() {
    use crate::indexer::resolve::engine::testkit::call_ref;
    use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};

    static ASYNC_PROFILE: LanguageProfile = LanguageProfile {
        async_wrappers: &["Promise"],
        ..DEFAULT_PROFILE
    };

    let arena = Arc::new(TypeArena::new());
    let mut mk = make_symbol("fetch", "fetch", SymbolKind::Function, None, None, None);
    mk.signature = Some("function fetch(): Promise<Response>".to_string());
    let res_sym = make_symbol("res", "res", SymbolKind::Variable, None, None, None);

    // `const res = await fetch()` — the Calls ref, attributed to the variable
    // (idx 1), same as the non-async `field_init_call_types_the_field` shape.
    let mut r = call_ref("fetch");
    r.source_symbol_index = 1;
    let mut pf = make_parsed_file("src/c.ts", vec![mk, res_sym], vec![r]);
    pf.flow.flow_binding_await.insert(1);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/c.ts".to_string(), "fetch".to_string()), 1);
    id_map.insert(("src/c.ts".to_string(), "res".to_string()), 2);

    let mut tree = Compilation::build(std::slice::from_ref(&pf), &id_map, Arc::clone(&arena));
    let mut profiles: rustc_hash::FxHashMap<&'static str, &'static LanguageProfile> =
        rustc_hash::FxHashMap::default();
    profiles.insert("typescript", &ASYNC_PROFILE);
    tree.infer_field_init_types(std::slice::from_ref(&pf), &profiles);
    assert_eq!(
        tree.field_type_str("res").as_deref(),
        Some("Response"),
        "await-bound field must unwrap the async wrapper, not record Promise<Response>",
    );
}

#[test]
fn local_var_init_call_types_the_variable_not_the_callee() {
    use crate::indexer::resolve::engine::testkit::call_ref;
    use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};

    let arena = Arc::new(TypeArena::new());
    let mut mk = make_symbol("makeThing", "makeThing", SymbolKind::Function, None, None, None);
    mk.signature = Some("function makeThing(): Thing".to_string());
    // `const r = makeThing()` — a local variable (idx 1).
    let r_sym = make_symbol("r", "r", SymbolKind::Variable, None, None, None);

    // The extractor emits BOTH: a chain-bearing TypeRef (target = callee name)
    // AND the call's Calls ref, both attributed to the variable.
    let chain_typeref = ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 1,
        target_name: "makeThing".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: Some(MemberChain {
            segments: vec![ChainSegment {
                name: "makeThing".to_string(),
                node_kind: String::new(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: true,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            }],
        }),
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let mut calls = call_ref("makeThing");
    calls.source_symbol_index = 1;
    let pf = make_parsed_file("src/m.ts", vec![mk, r_sym], vec![chain_typeref, calls]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/m.ts".to_string(), "makeThing".to_string()), 1);
    id_map.insert(("src/m.ts".to_string(), "r".to_string()), 2);

    let mut tree = Compilation::build(std::slice::from_ref(&pf), &id_map, Arc::clone(&arena));
    // derive_type_info_from_refs (run during build) must SKIP the chain-bearing
    // TypeRef — `r` must NOT be mis-typed to the callee name "makeThing".
    assert_eq!(
        tree.field_type_str("r"),
        None,
        "chain-bearing initializer must not type the variable to the callee name",
    );
    tree.infer_field_init_types(std::slice::from_ref(&pf), &rustc_hash::FxHashMap::default());
    assert_eq!(
        tree.field_type_str("r").as_deref(),
        Some("Thing"),
        "local variable typed from its initializer call's return",
    );
}

#[test]
fn return_type_name_for_find() {
    let (tree, _) = build_fixture();
    assert_eq!(
        tree.return_type_str("Repo.find").as_deref(),
        Some("User"),
        "return_type_str(Repo.find) should be Some(\"User\")"
    );
}

/// A params-first method with NO explicit return annotation (TS infers it from
/// the body) must NOT capture its last PARAMETER type as the return type. The
/// trailing-TypeRef fallback exists for a return-position TypeRef; for a callable
/// with no expressible return, the trailing TypeRef is a parameter.
#[test]
fn inferred_return_method_does_not_capture_last_param_as_return() {
    let arena = Arc::new(TypeArena::new());
    let mut method = make_symbol(
        "elementByCss",
        "Browser.elementByCss",
        SymbolKind::Method,
        Some(0),
        None,
        None,
    );
    method.signature = Some("elementByCss(selector: string, opts?: ElementByCssOpts)".to_string());
    let symbols = vec![
        make_symbol("Browser", "Browser", SymbolKind::Class, None, None, None),
        method,
    ];
    // The method's TypeRefs are its two PARAMETER types, in source order.
    let refs = vec![type_ref(1, "string"), type_ref(1, "ElementByCssOpts")];
    let pf = make_parsed_file("src/browser.ts", symbols, refs);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/browser.ts".to_string(), "Browser".to_string()), 1);
    id_map.insert(
        ("src/browser.ts".to_string(), "Browser.elementByCss".to_string()),
        2,
    );

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("Browser.elementByCss"),
        None,
        "a params-first method with no return annotation must have no derived \
         return type, not its last parameter's type"
    );
}

/// A function with an inline object-type return annotation (`fn(): { x: X }`) and
/// a synthesized member-bearing `{fn}$Ret` interface must (1) route its return
/// THROUGH `$Ret` and (2) type each `$Ret` member from the annotation — so member
/// access / destructuring of the call result resolves on the real members.
#[test]
fn object_type_return_routes_through_synth_ret_with_typed_members() {
    let arena = Arc::new(TypeArena::new());
    // The extractor sets the function's return to the inline object type and
    // mirrors it onto BOTH the qname and id slots — the routing must override it.
    let obj_ret = arena.intern_type_str("{ browser: Browser; flag: boolean }");
    let mut setup =
        make_symbol("setup", "setup", SymbolKind::Function, None, None, Some(obj_ret));
    setup.signature = Some("setup(): { browser: Browser; flag: boolean }".to_string());
    let symbols = vec![
        make_symbol("Browser", "Browser", SymbolKind::Class, None, None, None),
        setup,
        // The synth `$Ret` interface + its members (from the object-literal return),
        // created untyped by the flow pass.
        make_symbol("setup$Ret", "setup$Ret", SymbolKind::Interface, None, None, None),
        make_symbol("browser", "setup$Ret.browser", SymbolKind::Property, Some(2), None, None),
        make_symbol("flag", "setup$Ret.flag", SymbolKind::Property, Some(2), None, None),
    ];
    let pf = make_parsed_file("src/lib.ts", symbols, Vec::new());

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/lib.ts".to_string(), "Browser".to_string()), 1);
    id_map.insert(("src/lib.ts".to_string(), "setup".to_string()), 2);
    id_map.insert(("src/lib.ts".to_string(), "setup$Ret".to_string()), 3);
    id_map.insert(("src/lib.ts".to_string(), "setup$Ret.browser".to_string()), 4);
    id_map.insert(("src/lib.ts".to_string(), "setup$Ret.flag".to_string()), 5);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("setup").as_deref(),
        Some("setup$Ret"),
        "setup()'s return must route through the member-bearing $Ret interface"
    );
    // The destructure/chain seed reads the ID slot first — it must also be $Ret,
    // not the extractor-mirrored inline object-type string.
    assert_eq!(
        tree.return_type_id_of(2),
        Some(arena.class("setup$Ret")),
        "setup's id-slot return must route through $Ret (read before the qname slot)"
    );
    assert_eq!(
        tree.field_type_str("setup$Ret.browser").as_deref(),
        Some("Browser"),
        "$Ret.browser must be typed from the object-type return annotation"
    );
}

#[test]
fn field_type_name_for_db() {
    let (tree, _) = build_fixture();
    assert_eq!(
        tree.field_type_str("Repo.db").as_deref(),
        Some("Database"),
        "field_type_str(Repo.db) should be Some(\"Database\")"
    );
}

#[test]
fn parent_class_qname_for_user_repo() {
    let (tree, _) = build_fixture();
    assert_eq!(
        tree.parent_class_qname("UserRepo"),
        Some("Repo"),
        "parent_class_qname(UserRepo) should be Some(\"Repo\")"
    );
}

#[test]
fn parent_class_id_resolves_to_specific_parent() {
    let (tree, _) = build_fixture();
    let user_repo_id = tree.by_qualified_name("UserRepo").unwrap().id;
    let repo_id = tree.by_qualified_name("Repo").unwrap().id;
    assert_eq!(
        tree.parent_class_id(user_repo_id),
        Some(repo_id),
        "parent_class_id(UserRepo) should resolve to Repo's symbol id"
    );
}

/// Two packages each declare a class `Base`; a child in package 2 extends
/// `Base`. `inherits_by_id` must bind the child to package 2's `Base`, not
/// package 1's same-named class that won the first-wins qname race.
#[test]
fn inherits_by_id_prefers_same_package_parent() {
    let arena = Arc::new(TypeArena::new());

    // Package 1: class Base (qname "Base"), wins by_qname first-insert.
    let mut pf1 = make_parsed_file(
        "p1/base.ts",
        vec![make_symbol("Base", "Base", SymbolKind::Class, None, None, None)],
        vec![],
    );
    pf1.package_id = Some(1);

    // Package 2: class Base (also qname "Base") and class Child extends Base.
    let mut pf2 = make_parsed_file(
        "p2/mod.ts",
        vec![
            make_symbol("Base", "Base", SymbolKind::Class, None, None, None),
            make_symbol("Child", "Child", SymbolKind::Class, None, None, None),
        ],
        // Child (index 1 in pf2) inherits Base.
        vec![inherits_ref(1, "Base")],
    );
    pf2.package_id = Some(2);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("p1/base.ts".to_string(), "Base".to_string()), 1);
    id_map.insert(("p2/mod.ts".to_string(), "Base".to_string()), 2);
    id_map.insert(("p2/mod.ts".to_string(), "Child".to_string()), 3);

    let tree = Compilation::build(&[pf1, pf2], &id_map, arena);

    let child_id = 3;
    // Child lives in package 2, so its Base must be package 2's Base (id 2),
    // not package 1's same-named Base (id 1) that won the qname index.
    assert_eq!(
        tree.parent_class_id(child_id),
        Some(2),
        "Child's parent must be its own package's Base (id 2), not the first-wins Base (id 1)"
    );
}

#[test]
fn by_name_finds_both_classes() {
    let (tree, _) = build_fixture();
    let repos = tree.by_name("Repo");
    assert!(!repos.is_empty(), "by_name(Repo) should return at least one entry");
    let user_repos = tree.by_name("UserRepo");
    assert!(!user_repos.is_empty(), "by_name(UserRepo) should return at least one entry");
}

#[test]
fn types_by_name_returns_class_kinds() {
    let (tree, _) = build_fixture();
    let types = tree.types_by_name("Repo");
    assert!(
        !types.is_empty(),
        "types_by_name(Repo) should surface Repo (kind=class)"
    );
    for t in types.iter() {
        assert!(
            super::is_type_like_for_test(&t.kind),
            "types_by_name should only return type-like symbols, got kind={:?}",
            t.kind
        );
    }
}

#[test]
fn in_file_returns_symbols_for_path() {
    let (tree, _) = build_fixture();
    let syms = tree.in_file("src/repo.ts");
    let count = syms.len();
    assert!(count >= 4, "in_file should return at least 4 symbols; got {count}");
}

#[test]
fn has_in_namespace_for_repo() {
    let (tree, _) = build_fixture();
    // `Repo.find` and `Repo.db` exist, so `Repo` has members in its namespace.
    assert!(
        tree.has_in_namespace("Repo"),
        "has_in_namespace(Repo) should be true"
    );
    assert!(
        !tree.has_in_namespace("NonExistent"),
        "has_in_namespace(NonExistent) should be false"
    );
}

#[test]
fn reexports_from_returns_empty_for_non_barrel() {
    let (tree, _) = build_fixture();
    let reexports = tree.reexports_from("src/repo.ts");
    assert!(
        reexports.is_empty(),
        "no re-export refs were emitted, so reexports_from should be empty"
    );
}

/// `export * from '<bare-pkg>'`: extractor emits kind=Imports, is_reexport=true,
/// target_name="*", module=Some(pkg) — mirrors how `reexport_map` is fed.
fn star_reexport(module: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: "*".to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: true,
    }
}

/// `export { I } from '<bare-pkg>'`: a NAMED re-export, the shape a package's
/// entry uses to surface another package's declaration under its own module.
fn named_reexport(name: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: name.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: true,
    }
}

/// A bare named import `{ computed } from 'vue'` must resolve through the indexed
/// `vue → @vue/runtime-dom → @vue/runtime-core` `export *` chain to the `computed`
/// defined in runtime-core. Exercises Compilation's `resolve_external_reexport` /
/// `resolve_module_from` / `in_module_from` overrides driving `follow_reexports`.
#[test]
fn resolve_external_reexport_follows_export_star_package_chain() {
    let arena = Arc::new(TypeArena::new());
    let vue_entry = make_parsed_file(
        "ext:ts:vue/dist/vue.d.ts",
        vec![make_symbol(
            "__barrel",
            "vue.__barrel",
            SymbolKind::Variable,
            None,
            None,
            None,
        )],
        vec![star_reexport("@vue/runtime-dom")],
    );
    let rt_dom = make_parsed_file(
        "ext:ts:@vue/runtime-dom/dist/runtime-dom.d.ts",
        vec![make_symbol(
            "__barrel",
            "@vue/runtime-dom.__barrel",
            SymbolKind::Variable,
            None,
            None,
            None,
        )],
        vec![star_reexport("@vue/runtime-core")],
    );
    let rt_core = make_parsed_file(
        "ext:ts:@vue/runtime-core/dist/runtime-core.d.ts",
        vec![make_symbol(
            "computed",
            "@vue/runtime-core.computed",
            SymbolKind::Variable,
            None,
            None,
            None,
        )],
        vec![],
    );

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        (
            "ext:ts:@vue/runtime-core/dist/runtime-core.d.ts".into(),
            "@vue/runtime-core.computed".into(),
        ),
        700,
    );

    let tree = Compilation::build(&[vue_entry, rt_dom, rt_core], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.resolve_external_reexport("computed", "computed", "vue"),
        Some(700),
        "bare `import {{ computed }} from 'vue'` must bind through the indexed \
         vue → @vue/runtime-dom → @vue/runtime-core export* chain to runtime-core.computed"
    );
}

/// `<ngx-legend-chart>` binds to ECommerceLegendChartComponent via the
/// `@Component({selector:'ngx-legend-chart'})` pair the extractor records in
/// `ParsedFile::component_selectors`. Proves Compilation consumes that field into
/// `selector_qname` (the map SelectorMapRule consults).
#[test]
fn selector_qname_resolves_component_selector_from_parsed_file() {
    let symbols = vec![make_symbol(
        "ECommerceLegendChartComponent",
        "ECommerceLegendChartComponent",
        SymbolKind::Class,
        None,
        None,
        None,
    )];
    let mut pf = make_parsed_file(
        "src/app/pages/e-commerce/legend-chart/legend-chart.component.ts",
        symbols,
        vec![],
    );
    pf.component_selectors = vec![(
        "ngx-legend-chart".to_string(),
        "ECommerceLegendChartComponent".to_string(),
    )];

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        (pf.path.clone(), "ECommerceLegendChartComponent".to_string()),
        1,
    );

    let tree = Compilation::build(&[pf], &id_map, Arc::new(TypeArena::new()));

    assert_eq!(
        tree.selector_qname("ngx-legend-chart"),
        Some("ECommerceLegendChartComponent"),
        "Compilation must consume ParsedFile.component_selectors into selector_qname"
    );
    assert_eq!(tree.selector_qname("nb-card"), None);
}

#[test]
fn is_external_name_returns_false() {
    let (tree, _) = build_fixture();
    // External classification is deferred; conservative false.
    assert!(!tree.is_external_name("Repo", "typescript"));
}

#[test]
fn type_arena_is_some() {
    let (tree, _) = build_fixture();
    assert!(
        tree.type_arena().is_some(),
        "type_arena() should expose the shared arena"
    );
}

#[test]
fn symbols_absent_from_id_map_are_skipped() {
    let arena = Arc::new(TypeArena::new());
    let symbols = vec![
        make_symbol("Ghost", "Ghost", SymbolKind::Class, None, None, None),
        make_symbol("Real", "Real", SymbolKind::Class, None, None, None),
    ];
    let pf = make_parsed_file("src/x.ts", symbols, vec![]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    // Only "Real" is in the map; "Ghost" is absent.
    id_map.insert(("src/x.ts".to_string(), "Real".to_string()), 99);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));
    assert!(tree.by_qualified_name("Real").is_some());
    assert!(tree.by_qualified_name("Ghost").is_none());
}

#[test]
fn workspace_package_id_resolves_declared_name_and_deep_import() {
    let arena = Arc::new(TypeArena::new());
    let ctx = ProjectContext {
        workspace_pkg_by_declared_name: [("@org/utils".to_string(), 42)].into_iter().collect(),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &[],
        &HashMap::new(),
        arena,
        Some(&ctx),
        &std::collections::HashSet::new(),
    );
    assert_eq!(tree.workspace_package_id("@org/utils"), Some(42), "exact declared name");
    assert_eq!(
        tree.workspace_package_id("@org/utils/sub/mod"),
        Some(42),
        "deep import peels to the package root"
    );
    assert_eq!(tree.workspace_package_id("@other/pkg"), None);
    assert!(tree.is_workspace_declared_name("@org/utils"));
    assert!(!tree.is_workspace_declared_name("@org/utils/sub"));
}

#[test]
fn symbols_in_package_groups_symbols_by_package_id() {
    let arena = Arc::new(TypeArena::new());
    let symbols = vec![make_symbol("Util", "Util", SymbolKind::Class, None, None, None)];
    let mut pf = make_parsed_file("packages/utils/x.ts", symbols, vec![]);
    pf.package_id = Some(7);
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("packages/utils/x.ts".to_string(), "Util".to_string()), 1);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));
    let in_pkg = tree.symbols_in_package(7);
    assert_eq!(in_pkg.len(), 1);
    assert_eq!(in_pkg.first().unwrap().qualified_name, "Util");
    assert!(tree.symbols_in_package(999).is_empty());
}

// ---------------------------------------------------------------------------
// TypeRef-derived type_info tests (Phase B / derive_type_info_from_refs)
// ---------------------------------------------------------------------------

fn typeref_ref(source_symbol_index: usize, target_name: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index,
        target_name: target_name.to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
    }
}

fn make_symbol_with_sig(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    parent_index: Option<usize>,
    signature: Option<&str>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: signature.map(str::to_string),
        doc_comment: None,
        scope_path: None,
        parent_index,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// A method with no extractor-set TypeId but with a TypeRef ref expressing
/// its return type. `return_type_name` must resolve through Phase B.
#[test]
fn typeref_derived_return_type_for_method() {
    let arena = Arc::new(TypeArena::new());

    // class User {}            index 0
    // class Svc {              index 1
    //   getUser(): User        index 2  — no TypeId; TypeRef → "User"
    // }
    let symbols = vec![
        make_symbol("User", "User", SymbolKind::Class, None, None, None),
        make_symbol("Svc", "Svc", SymbolKind::Class, None, None, None),
        make_symbol("getUser", "Svc.getUser", SymbolKind::Method, Some(1), None, None),
    ];
    let refs = vec![
        // TypeRef from getUser (index 2) → "User"
        typeref_ref(2, "User"),
    ];
    let pf = make_parsed_file("src/svc.ts", symbols, refs);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/svc.ts".to_string(), "User".to_string()), 1);
    id_map.insert(("src/svc.ts".to_string(), "Svc".to_string()), 2);
    id_map.insert(("src/svc.ts".to_string(), "Svc.getUser".to_string()), 3);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("Svc.getUser").as_deref(),
        Some("User"),
        "return_type_str should be derived from the TypeRef ref"
    );
}

/// A field with no extractor-set TypeId but with a TypeRef ref expressing
/// its declared type. `field_type_name` must resolve through Phase B.
#[test]
fn typeref_derived_field_type_for_property() {
    let arena = Arc::new(TypeArena::new());

    // class Config {}    index 0
    // class App {        index 1
    //   config: Config   index 2  — no TypeId; TypeRef → "Config"
    // }
    let symbols = vec![
        make_symbol("Config", "Config", SymbolKind::Class, None, None, None),
        make_symbol("App", "App", SymbolKind::Class, None, None, None),
        make_symbol("config", "App.config", SymbolKind::Property, Some(1), None, None),
    ];
    let refs = vec![typeref_ref(2, "Config")];
    let pf = make_parsed_file("src/app.ts", symbols, refs);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/app.ts".to_string(), "Config".to_string()), 1);
    id_map.insert(("src/app.ts".to_string(), "App".to_string()), 2);
    id_map.insert(("src/app.ts".to_string(), "App.config".to_string()), 3);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.field_type_str("App.config").as_deref(),
        Some("Config"),
        "field_type_str should be derived from the TypeRef ref"
    );
}

/// A method with a signature that encodes its return type (e.g. `-> User`).
/// No TypeRef ref emitted; Phase B should parse the signature.
#[test]
fn signature_derived_return_type_for_method() {
    let arena = Arc::new(TypeArena::new());

    let symbols = vec![
        make_symbol("User", "User", SymbolKind::Class, None, None, None),
        make_symbol_with_sig(
            "load",
            "load",
            SymbolKind::Function,
            None,
            Some("fn load() -> User"),
        ),
    ];
    let pf = make_parsed_file("src/loader.ts", symbols, vec![]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/loader.ts".to_string(), "User".to_string()), 1);
    id_map.insert(("src/loader.ts".to_string(), "load".to_string()), 2);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("load").as_deref(),
        Some("User"),
        "return_type_str should be derived from the signature string"
    );
}

/// A fluent self-returning method (`m(p: P): this`) must capture `this` as its
/// return type. The `this` return emits no TypeRef, so the param's TypeRef is the
/// only one present — the return-type derivation must NOT pick that parameter
/// type. Reproduces the vitest `mockImplementation(fn: NormalizedProcedure): this`
/// chain break where the receiver typed to the parameter instead of the receiver.
#[test]
fn this_return_is_captured_over_parameter_typeref() {
    let arena = Arc::new(TypeArena::new());

    let symbols = vec![
        make_symbol("Proc", "Proc", SymbolKind::Class, None, None, None),
        make_symbol_with_sig(
            "mockImpl",
            "Mock.mockImpl",
            SymbolKind::Method,
            None,
            Some("mockImpl(fn: Proc): this"),
        ),
    ];
    // The parameter type emits a TypeRef; the `this` return emits none.
    let refs = vec![typeref_ref(1, "Proc")];
    let pf = make_parsed_file("src/mock.ts", symbols, refs);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/mock.ts".to_string(), "Proc".to_string()), 1);
    id_map.insert(("src/mock.ts".to_string(), "Mock.mockImpl".to_string()), 2);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("Mock.mockImpl").as_deref(),
        Some("this"),
        "a `: this` return must be captured verbatim, not the parameter type"
    );
}

/// An extractor-set TypeId must NOT be overwritten by a TypeRef ref for the
/// same symbol. The TypeId-derived value from Phase A wins.
#[test]
fn typeid_is_not_overwritten_by_typeref() {
    let arena = Arc::new(TypeArena::new());

    // The extractor set return_type = TypeArena::class("User") on the method.
    // A TypeRef ref points to "Other". Phase B must not clobber the TypeId value.
    let user_id = arena.class("User");

    let symbols = vec![
        make_symbol(
            "fetch",
            "fetch",
            SymbolKind::Method,
            None,
            None,
            Some(user_id),
        ),
    ];
    // TypeRef claiming the return type is "Other" — must be ignored.
    let refs = vec![typeref_ref(0, "Other")];
    let pf = make_parsed_file("src/f.ts", symbols, refs);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/f.ts".to_string(), "fetch".to_string()), 1);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("fetch").as_deref(),
        Some("User"),
        "extractor TypeId value must survive Phase B — must not be overwritten by TypeRef"
    );
}

/// A symbol whose qualified name the materialization layer flagged ambient is
/// indexed into `ambient_scope` keyed by its simple name, so the ambient-scope
/// rung can bind a bare reference to it.
#[test]
fn ambient_scope_indexes_globals_namespace_symbols() {
    let arena = Arc::new(TypeArena::new());

    let globals_qname = format!("{}.expect", crate::ecosystem::npm::NPM_GLOBALS_MODULE);
    let symbols = vec![
        make_symbol("expect", &globals_qname, SymbolKind::Variable, None, None, None),
        make_symbol("ordinary", "ordinary", SymbolKind::Function, None, None, None),
    ];
    let pf = make_parsed_file("ext:ts:__npm_globals__/vitest.d.ts", symbols, vec![]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        ("ext:ts:__npm_globals__/vitest.d.ts".to_string(), globals_qname.clone()),
        42,
    );
    id_map.insert(
        ("ext:ts:__npm_globals__/vitest.d.ts".to_string(), "ordinary".to_string()),
        43,
    );

    // Only the globals-namespace qname is flagged ambient by the materialization layer.
    let ambient: std::collections::HashSet<String> = [globals_qname].into_iter().collect();
    let arena2 = Arc::clone(&arena);
    let mut tree = Compilation::build(&[], &HashMap::new(), arena2);
    tree.ingest(&[pf], &id_map, &ambient);

    let hits = tree.ambient_symbols("expect");
    assert_eq!(hits.len(), 1, "the flagged globals symbol is in ambient scope");
    assert_eq!(hits.first().unwrap().id, 42);

    // An ordinary top-level symbol is not ambient.
    assert!(
        tree.ambient_symbols("ordinary").is_empty(),
        "ordinary symbols stay out of ambient scope"
    );
}

// ---------------------------------------------------------------------------
// Bare-identifier return-type inference (Phase C / infer_bare_identifier_returns)
// ---------------------------------------------------------------------------

/// Build a hook whose body is `return <ident>`, where `<ident>` is a typed
/// parameter. The function carries no declared/extractor return, so its return
/// type must be inferred from the parameter's declared type.
///
/// ```text
/// class QueryClient { clear(): void }
/// function useQueryClient(qc: QueryClient) { return qc }   // → QueryClient
/// ```
///
/// `qc` is emitted as a Parameter child symbol `useQueryClient.qc` with a
/// TypeRef ref to `QueryClient`; the bare return is recorded in
/// `flow.flow_return_ident` as `(fn_idx, "qc")`.
fn build_hook_fixture(
    path: &str,
    hook_qname: &str,
    param_type: &str,
    next_id: &mut i64,
) -> (ParsedFile, HashMap<(String, String), i64>) {
    let param_qname = format!("{hook_qname}.qc");
    // QueryClient(0), QueryClient.clear(1), hook(2), hook.qc param(3)
    let symbols = vec![
        make_symbol(param_type, param_type, SymbolKind::Class, None, None, None),
        make_symbol(
            "clear",
            &format!("{param_type}.clear"),
            SymbolKind::Method,
            Some(0),
            None,
            None,
        ),
        make_symbol("useQueryClient", hook_qname, SymbolKind::Function, None, None, None),
        make_symbol("qc", &param_qname, SymbolKind::Parameter, Some(2), None, None),
    ];
    // The param (index 3) has a TypeRef → param_type, so Phase B types it.
    let refs = vec![typeref_ref(3, param_type)];
    let mut pf = make_parsed_file(path, symbols, refs);
    // `return qc` — bare identifier, no ref; recorded by the extractor as the
    // function (index 2) returning identifier "qc".
    pf.flow.flow_return_ident = vec![(2, "qc".to_string())];

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    for qname in [
        param_type.to_string(),
        format!("{param_type}.clear"),
        hook_qname.to_string(),
        param_qname.clone(),
    ] {
        id_map.insert((path.to_string(), qname), *next_id);
        *next_id += 1;
    }
    (pf, id_map)
}

/// A function whose body is `return <param>` infers its return type from the
/// typed parameter, even though no ref or signature names the return.
#[test]
fn bare_identifier_return_infers_param_type() {
    let arena = Arc::new(TypeArena::new());
    let mut next_id = 1;
    let (pf, id_map) =
        build_hook_fixture("src/hooks.ts", "useQueryClient", "QueryClient", &mut next_id);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("useQueryClient").as_deref(),
        Some("QueryClient"),
        "the function's return must be inferred from the returned parameter's type"
    );
}

/// A declared/extractor return wins: a function that already has a return type
/// must not be overwritten by the bare-identifier inference.
#[test]
fn declared_return_blocks_bare_identifier_inference() {
    let arena = Arc::new(TypeArena::new());
    let other_id = arena.class("Other");

    // hook(0) has an extractor return_type = Other; qc param(1): QueryClient.
    // The bare-return harvest must leave the declared Other in place.
    let symbols = vec![
        make_symbol(
            "useQueryClient",
            "useQueryClient",
            SymbolKind::Function,
            None,
            None,
            Some(other_id),
        ),
        make_symbol("qc", "useQueryClient.qc", SymbolKind::Parameter, Some(0), None, None),
    ];
    let refs = vec![typeref_ref(1, "QueryClient")];
    let mut pf = make_parsed_file("src/hooks.ts", symbols, refs);
    pf.flow.flow_return_ident = vec![(0, "qc".to_string())];

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/hooks.ts".to_string(), "useQueryClient".to_string()), 1);
    id_map.insert(("src/hooks.ts".to_string(), "useQueryClient.qc".to_string()), 2);

    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("useQueryClient").as_deref(),
        Some("Other"),
        "an extractor-declared return must survive bare-identifier inference"
    );
}

/// A hook copied across two monorepo packages — identical qname, distinct
/// symbol ids, identical returned-parameter type — infers correctly: the shared
/// qname-keyed slot agrees, so the agreement gate folds it.
#[test]
fn cross_module_agreement_infers_shared_qname() {
    let arena = Arc::new(TypeArena::new());
    let mut next_id = 1;
    let (mut pf1, id1) =
        build_hook_fixture("p1/hooks.ts", "useQueryClient", "QueryClient", &mut next_id);
    let (mut pf2, id2) =
        build_hook_fixture("p2/hooks.ts", "useQueryClient", "QueryClient", &mut next_id);
    pf1.package_id = Some(1);
    pf2.package_id = Some(2);

    let mut id_map = id1;
    id_map.extend(id2);

    let tree = Compilation::build(&[pf1, pf2], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.return_type_str("useQueryClient").as_deref(),
        Some("QueryClient"),
        "two copies agreeing on the same return must infer the shared slot"
    );
}

/// The agreement gate, driven directly on candidate tuples: agreement across
/// distinct owners folds, disagreement skips. Driving the gate directly is the
/// only way to exercise the disagreement branch — in the full pass two copies of
/// a function share the param's qname, so the param-derived candidate type is
/// the same first-winner string for both owners and can never disagree.
#[test]
fn agreement_gate_folds_agreement_and_skips_disagreement() {
    let c = |q: &str, t: &str| (q.to_string(), t.to_string(), None);

    // Agreement across two owners of the shared qname → one folded entry.
    let agree = super::agree_inferred_returns_for_test(vec![
        c("helper", "User"),
        c("helper", "User"),
    ]);
    assert_eq!(agree.len(), 1, "agreeing candidates fold to one entry");
    assert_eq!(agree[0].0, "helper");
    assert_eq!(agree[0].1, "User");

    // Disagreement on the shared qname → the qname is dropped entirely.
    let disagree = super::agree_inferred_returns_for_test(vec![
        c("helper2", "User"),
        c("helper2", "Account"),
    ]);
    assert!(
        disagree.is_empty(),
        "one shared slot cannot hold two types; disagreement skips"
    );

    // A second qname that agrees still survives alongside a dropped one.
    let mixed = super::agree_inferred_returns_for_test(vec![
        c("a", "User"),
        c("a", "Account"),
        c("b", "Account"),
    ]);
    let folded: Vec<&str> = mixed.iter().map(|(q, _, _)| q.as_str()).collect();
    assert_eq!(folded, vec!["b"], "only the agreeing qname `b` is folded");
}

/// End-to-end: once the inferred return lands, a call-root chain
/// `useQueryClient().clear()` resolves through the chain walker against the
/// `Compilation`, binding the leaf to `QueryClient.clear`.
#[test]
fn inferred_return_lets_call_root_chain_resolve() {
    use crate::indexer::resolve::engine::chain::bind_member_access;
    use crate::indexer::resolve::engine::testkit::{call_ref, file_ctx, ref_ctx, source_symbol};
    use crate::types::{ChainSegment, MemberChain, SegmentKind};

    let arena = Arc::new(TypeArena::new());
    let mut next_id = 1;
    let (pf, id_map) =
        build_hook_fixture("src/hooks.ts", "useQueryClient", "QueryClient", &mut next_id);
    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    // `useQueryClient().clear()` — the call root reads useQueryClient's inferred
    // return (QueryClient), then `clear` binds on QueryClient.
    let mk_seg = |name: &str, is_call: bool, kind: SegmentKind| ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    };
    let segs = vec![
        mk_seg("useQueryClient", true, SegmentKind::Identifier),
        mk_seg("clear", true, SegmentKind::Property),
    ];
    let mut r = call_ref("clear");
    r.chain = Some(MemberChain { segments: segs });
    let src = source_symbol("caller");
    let rc = ref_ctx(&r, &src, vec![]);

    let fc = file_ctx(vec![], None);
    use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let resolved = bind_member_access(&rc, &fc, &tree, &DEFAULT_PROFILE)
        .ok()
        .map(|res| res.target_symbol_id);
    let clear_id = tree
        .by_qualified_name("QueryClient.clear")
        .expect("QueryClient.clear indexed")
        .id;
    assert_eq!(
        resolved,
        Some(clear_id),
        "the call-root chain must resolve `clear` on the inferred QueryClient return"
    );
}

/// The keystone for `@/`-aliased imports: `Compilation` must expose the tsconfig
/// path aliases snapshotted from `ProjectContext`. Without this override the trait
/// default returns `None` and every `import … from "@/…"` falls through
/// `AliasedImportRule` unresolved.
#[test]
fn resolve_path_alias_honors_per_package_isolation_and_global() {
    let arena = Arc::new(TypeArena::new());
    let mut c = Compilation::empty(Arc::clone(&arena));
    c.path_aliases_by_pkg
        .insert(8, vec![("@/".to_string(), "./".to_string())]);
    c.path_aliases_global = vec![("~/".to_string(), "lib/".to_string())];

    // Per-package alias, longest-prefix rewrite.
    assert_eq!(
        c.resolve_path_alias(Some(8), "@/utils/types").as_deref(),
        Some("./utils/types")
    );
    // A specifier matching no alias in the package → None (not a global borrow).
    assert_eq!(c.resolve_path_alias(Some(8), "react"), None);
    // A ref with no package id uses the workspace-wide aliases.
    assert_eq!(c.resolve_path_alias(None, "~/db").as_deref(), Some("lib/db"));
    // An isolated package that declares NO aliases declines — it never borrows the
    // global set (mirrors ProjectContext::manifests_for isolation).
    c.path_aliases_by_pkg.insert(9, Vec::new());
    assert_eq!(c.resolve_path_alias(Some(9), "~/db"), None);
}

/// A TypeRef the extractor tagged with a module — `typeof import('m')['k']`
/// stores `{ target_name: k, module: Some(m) }` on the value it types — must
/// resolve to the type of the VALUE module `m` exports as `k`, not to a
/// self-referential `m.k` and not to the bare name `k`.
///
/// Mirrors the vitest globals shape: `let expect: typeof import('vitest')['expect']`
/// where `vitest` exports `expect` as a local rename of `globalExpect`, whose
/// declared type is the `ExpectStatic` interface. The derived `field_type` for
/// the global `expect` must be that interface's qname.
#[test]
fn module_tagged_value_typeref_resolves_to_exported_value_type() {
    let arena = Arc::new(TypeArena::new());

    // Module `mod` (a `.d.ts`-shaped external file):
    //   interface TheType {}                       index 0  → mod.TheType
    //   const globalExp: TheType                    index 1  → mod.globalExp (TypeRef → TheType)
    //   export { globalExp as exp }                 local rename, exposed name `exp`
    let mod_symbols = vec![
        make_symbol("TheType", "mod.TheType", SymbolKind::Interface, None, None, None),
        make_symbol("globalExp", "mod.globalExp", SymbolKind::Variable, None, None, None),
    ];
    // `globalExp`'s declared type is `TheType`, scoped to module `mod`.
    let mod_global_type_ref = typeref_ref(1, "TheType");
    let mut mod_pf = make_parsed_file("ext:ts:mod/index.d.ts", mod_symbols, vec![]);
    if let Some(s) = mod_pf.symbols.get_mut(1) {
        s.scope_path = Some("mod".to_string());
    }
    // Local export rename `export { globalExp as exp }`: the extractor records
    // this as a re-export ref carrying the local source name in `target_name`
    // and the exposed name in `namespace_segments[0]`, with no `module`.
    let local_rename = ExtractedRef {
        source_symbol_index: 1,
        target_name: "globalExp".to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: vec!["exp".to_string()],
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: true,
    };
    mod_pf.refs = vec![mod_global_type_ref, local_rename];

    // Consumer: a global `exp` typed `typeof import('mod')['exp']` — the
    // extractor's `lookup_type` arm emits `{ target_name: "exp", module: Some("mod") }`.
    // Its qname (`g.exp`) under scope `g` makes the bare-name resolution of the
    // ref's `exp` self-match the global, the signal that the ref names a value
    // export rather than a type — exactly the vitest global-`expect` shape.
    let mut consumer = make_symbol("exp", "g.exp", SymbolKind::Variable, None, None, None);
    consumer.scope_path = Some("g".to_string());
    let mut module_tagged = typeref_ref(0, "exp");
    module_tagged.module = Some("mod".to_string());
    let consumer_pf = make_parsed_file("ext:ts:g/globals.d.ts", vec![consumer], vec![module_tagged]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("ext:ts:mod/index.d.ts".to_string(), "mod.TheType".to_string()), 1);
    id_map.insert(("ext:ts:mod/index.d.ts".to_string(), "mod.globalExp".to_string()), 2);
    id_map.insert(("ext:ts:g/globals.d.ts".to_string(), "g.exp".to_string()), 3);

    let tree = Compilation::build(&[mod_pf, consumer_pf], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.field_type_str("g.exp").as_deref(),
        Some("mod.TheType"),
        "a module-tagged value TypeRef that self-resolves must type to the exported \
         value's declared type, following the local export rename — not self-referential \
         `g.exp`, not bare `exp`"
    );
}

/// A top-level symbol whose qname the materialization layer flagged ambient is
/// indexed into `ambient_scope` by its simple name; a nested member of the same
/// file is not (only top-level lib globals are ambient).
#[test]
fn ambient_scope_indexes_materialization_flagged_globals() {
    let arena = Arc::new(TypeArena::new());

    let symbols = vec![
        make_symbol("Record", "Record", SymbolKind::TypeAlias, None, None, None),
        make_symbol("then", "Promise.then", SymbolKind::Method, None, None, None),
    ];
    let pf = make_parsed_file("ext:ts:__ts_lib__/lib.es5.d.ts", symbols, vec![]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("ext:ts:__ts_lib__/lib.es5.d.ts".to_string(), "Record".to_string()), 71);
    id_map.insert(("ext:ts:__ts_lib__/lib.es5.d.ts".to_string(), "Promise.then".to_string()), 72);

    // The materialization layer flags only the top-level global.
    let ambient: std::collections::HashSet<String> = ["Record".to_string()].into_iter().collect();

    let arena2 = Arc::clone(&arena);
    let mut tree = Compilation::build(&[], &HashMap::new(), arena2);
    tree.ingest(&[pf], &id_map, &ambient);

    let hits = tree.ambient_symbols("Record");
    assert_eq!(hits.len(), 1, "the flagged lib global is in ambient scope");
    assert_eq!(hits.first().unwrap().id, 71);

    // A nested member must not leak into ambient scope under its leaf name.
    assert!(
        tree.ambient_symbols("then").is_empty(),
        "nested members stay out of ambient scope"
    );
}

// ---------------------------------------------------------------------------
// enclosing_chain — parent_index ancestry walk, cycle-guarded
// ---------------------------------------------------------------------------

#[test]
fn enclosing_chain_terminates_on_self_parent() {
    // A symbol whose parent_index points at its own slot. A name-based merge of
    // a same-named SCSS selector parent/descendant produces exactly this. The
    // walk must terminate (the self is type-like, so it is its own enclosing
    // type; there is no namespace), not spin forever.
    let symbols = vec![make_symbol(
        "fixed-top",
        "fixed-top",
        SymbolKind::Class,
        Some(0),
        None,
        None,
    )];
    let (found_type, found_ns) = super::enclosing_chain(&symbols, symbols[0].parent_index);
    assert_eq!(found_type.as_deref(), Some("fixed-top"));
    assert_eq!(found_ns, None);
}

#[test]
fn enclosing_chain_terminates_on_two_cycle() {
    // A → B → A parent_index loop with no namespace anywhere in it.
    let symbols = vec![
        make_symbol("a", "a", SymbolKind::Class, Some(1), None, None),
        make_symbol("b", "b", SymbolKind::Class, Some(0), None, None),
    ];
    let (found_type, found_ns) = super::enclosing_chain(&symbols, symbols[0].parent_index);
    assert_eq!(found_type.as_deref(), Some("b"));
    assert_eq!(found_ns, None);
}

#[test]
fn enclosing_chain_finds_nearest_type_and_namespace() {
    // method ⊂ class ⊂ namespace — an acyclic chain still resolves both.
    let symbols = vec![
        make_symbol("App", "App", SymbolKind::Namespace, None, None, None),
        make_symbol("Svc", "App.Svc", SymbolKind::Class, Some(0), None, None),
        make_symbol("run", "App.Svc.run", SymbolKind::Method, Some(1), None, None),
    ];
    let (found_type, found_ns) = super::enclosing_chain(&symbols, symbols[2].parent_index);
    assert_eq!(found_type.as_deref(), Some("App.Svc"));
    assert_eq!(found_ns.as_deref(), Some("App"));
}

// ---------------------------------------------------------------------------
// apply_external_reexport_aliases — cross-package re-export alias lookup
// ---------------------------------------------------------------------------

/// Barrel file binding `wrapper-pkg.util` as a typeless re-export marker,
/// declaring package carrying the real `lib-pkg.util` declaration.
fn build_reexport_alias_fixture() -> Compilation {
    let arena = Arc::new(TypeArena::new());
    let barrel = make_parsed_file(
        "ext:ts:wrapper-pkg/index.d.ts",
        vec![make_symbol(
            "util",
            "wrapper-pkg.util",
            SymbolKind::Variable,
            None,
            None,
            None,
        )],
        Vec::new(),
    );
    let decl_ty = arena.class("UtilApi");
    let lib = make_parsed_file(
        "ext:ts:lib-pkg/index.d.ts",
        vec![make_symbol(
            "util",
            "lib-pkg.util",
            SymbolKind::Variable,
            None,
            Some(decl_ty),
            None,
        )],
        Vec::new(),
    );
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        ("ext:ts:wrapper-pkg/index.d.ts".to_string(), "wrapper-pkg.util".to_string()),
        10,
    );
    id_map.insert(
        ("ext:ts:lib-pkg/index.d.ts".to_string(), "lib-pkg.util".to_string()),
        20,
    );
    let mut tree = Compilation::build(&[barrel, lib], &id_map, arena);
    tree.apply_external_reexport_aliases(&[(
        "wrapper-pkg.util".to_string(),
        "lib-pkg.util".to_string(),
        "ext:ts:lib-pkg/index.d.ts".to_string(),
    )]);
    tree
}

#[test]
fn reexport_alias_target_names_the_declaration_and_leaves_the_single_slot_alone() {
    let tree = build_reexport_alias_fixture();
    let target = tree.reexport_alias_target("wrapper-pkg.util").expect("alias registered");
    assert_eq!(target.id, 20, "the alias resolves to the declaration's single identity");
    assert_eq!(target.qualified_name, "lib-pkg.util");
    assert_eq!(tree.reexport_alias_target("lib-pkg.util").map(|s| s.id), None);
    // The single-winner slot keeps the binding symbol: type-derivation
    // contexts read it where the binding itself is the correct referent.
    assert_eq!(tree.by_qualified_name("wrapper-pkg.util").map(|s| s.id), Some(10));
}

#[test]
fn reexport_alias_leads_but_keeps_same_qname_fallbacks() {
    let tree = build_reexport_alias_fixture();
    let all = tree.all_by_qualified_name("wrapper-pkg.util");
    assert_eq!(all.len(), 2, "declaration first, barrel binding kept as fallback");
    assert_eq!(all.first().unwrap().id, 20);
    assert_eq!(all.get(1).unwrap().id, 10);
    // The declaring package's own qname is untouched.
    assert_eq!(tree.by_qualified_name("lib-pkg.util").map(|s| s.id), Some(20));
}

#[test]
fn merged_value_type_pair_keeps_declared_type_off_field_slots() {
    // `declare var D: DConstructor` + `interface D` share one qname. The
    // value's declared type must not occupy the field slots — the qname slot
    // types INSTANCES of the type, and the id map cannot tell the two
    // same-qname symbols apart.
    let arena = Arc::new(TypeArena::new());
    let ctor = arena.class("DConstructor");
    let api = arena.class("ApiKind");
    let symbols = vec![
        make_symbol("D", "D", SymbolKind::Variable, None, Some(ctor), None),
        make_symbol("D", "D", SymbolKind::Interface, None, None, None),
        make_symbol("gadget", "gadget", SymbolKind::Variable, None, Some(api), None),
    ];
    let pf = make_parsed_file("src/lib.d.ts", symbols, Vec::new());
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/lib.d.ts".to_string(), "D".to_string()), 1);
    id_map.insert(("src/lib.d.ts".to_string(), "gadget".to_string()), 2);
    let tree = Compilation::build(&[pf], &id_map, Arc::clone(&arena));

    assert_eq!(tree.field_type_id("D"), None);
    assert_eq!(tree.field_type_id_of(1), None);
    // A variable whose qname no type owns keeps its declared type on both slots.
    assert_eq!(tree.field_type_id("gadget"), Some(api));
    assert_eq!(tree.field_type_id_of(2), Some(api));
}

// ---------------------------------------------------------------------------
// Cross-language external visibility (ext_lang_allowed / filter_ext_langs)
// ---------------------------------------------------------------------------

use crate::ecosystem::EcosystemId;

/// One ext value file of the given language, declaring `name` typed `ty`,
/// plus the id map entry for it.
fn ext_value_fixture(
    path: &str,
    language: &str,
    name: &str,
    ty: crate::type_checker::core::types::TypeId,
) -> (ParsedFile, HashMap<(String, String), i64>) {
    let mut pf = make_parsed_file(
        path,
        vec![make_symbol(name, name, SymbolKind::Variable, None, Some(ty), None)],
        Vec::new(),
    );
    pf.language = language.to_string();
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert((path.to_string(), name.to_string()), 1);
    (pf, id_map)
}

fn build_with_active(
    active: Vec<EcosystemId>,
    files: &[ParsedFile],
    id_map: &HashMap<(String, String), i64>,
    arena: Arc<TypeArena>,
) -> Compilation {
    let ctx = ProjectContext {
        active_ecosystems: active,
        ..Default::default()
    };
    Compilation::build_with_context(files, id_map, arena, Some(&ctx), &std::collections::HashSet::new())
}

#[test]
fn foreign_language_ext_value_is_dropped_from_by_name() {
    let arena = Arc::new(TypeArena::new());
    let str_ty = arena.class("str");
    let (pf, id_map) = ext_value_fixture(
        "ext:idx:C:/py/site-packages/fields.py",
        "python",
        "field",
        str_ty,
    );
    // cargo serves rust; pypi serves python; no active ecosystem serves both.
    let tree = build_with_active(
        vec![EcosystemId::new("cargo"), EcosystemId::new("pypi")],
        &[pf],
        &id_map,
        Arc::clone(&arena),
    );

    let rust_allowed = tree.ext_lang_allowed("rust");
    assert!(rust_allowed.is_some(), "cargo is active — rust must carry a visibility set");
    let filtered = tree.filter_ext_langs(tree.by_name("field"), rust_allowed);
    assert!(
        filtered.is_empty(),
        "a python ext value must not serve a rust receiver's bare-name probe"
    );

    let kept = tree.filter_ext_langs(tree.by_name("field"), tree.ext_lang_allowed("python"));
    assert_eq!(kept.len(), 1, "the same value stays visible to a python receiver");
}

#[test]
fn co_declared_ecosystem_languages_cross_resolve() {
    let arena = Arc::new(TypeArena::new());
    let api = arena.class("ApiClient");
    let (pf, id_map) =
        ext_value_fixture("ext:ts:some-pkg/index.d.ts", "typescript", "client", api);
    // npm declares typescript AND javascript AND vue — one ecosystem, one family.
    let tree = build_with_active(
        vec![EcosystemId::new("npm")],
        &[pf],
        &id_map,
        Arc::clone(&arena),
    );

    for receiver in ["javascript", "typescript", "vue"] {
        let allowed = tree.ext_lang_allowed(receiver);
        assert!(allowed.is_some(), "{receiver} is npm-served");
        let kept = tree.filter_ext_langs(tree.by_name("client"), allowed);
        assert_eq!(
            kept.len(),
            1,
            "a TS ext declaration must stay visible to a {receiver} receiver"
        );
    }
}

#[test]
fn unknown_receiver_language_is_unfiltered() {
    let arena = Arc::new(TypeArena::new());
    let str_ty = arena.class("str");
    let (pf, id_map) = ext_value_fixture(
        "ext:idx:C:/py/site-packages/fields.py",
        "python",
        "field",
        str_ty,
    );
    let tree = build_with_active(
        vec![EcosystemId::new("pypi")],
        &[pf],
        &id_map,
        Arc::clone(&arena),
    );

    // No active ecosystem serves markdown — no constraint, nothing filtered.
    let allowed = tree.ext_lang_allowed("markdown");
    assert!(allowed.is_none());
    let kept = tree.filter_ext_langs(tree.by_name("field"), allowed);
    assert_eq!(kept.len(), 1);
}

#[test]
fn internal_candidates_are_never_language_filtered() {
    let arena = Arc::new(TypeArena::new());
    let str_ty = arena.class("String");
    let mut pf = make_parsed_file(
        "src/lib.rs",
        vec![make_symbol("field", "field", SymbolKind::Variable, None, Some(str_ty), None)],
        Vec::new(),
    );
    pf.language = "rust".to_string();
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/lib.rs".to_string(), "field".to_string()), 1);
    let tree = build_with_active(
        vec![EcosystemId::new("cargo"), EcosystemId::new("pypi")],
        &[pf],
        &id_map,
        Arc::clone(&arena),
    );

    // The python receiver's set excludes rust, but the candidate is internal
    // (project source, not `ext:`) — mixed-language project boundaries stay legal.
    let kept = tree.filter_ext_langs(tree.by_name("field"), tree.ext_lang_allowed("python"));
    assert_eq!(kept.len(), 1);
}

#[test]
fn context_without_active_ecosystems_has_no_visibility_sets() {
    let arena = Arc::new(TypeArena::new());
    let str_ty = arena.class("str");
    let (pf, id_map) = ext_value_fixture(
        "ext:idx:C:/py/site-packages/fields.py",
        "python",
        "field",
        str_ty,
    );
    let ctx = ProjectContext::default();
    let tree = Compilation::build_with_context(
        &[pf],
        &id_map,
        Arc::clone(&arena),
        Some(&ctx),
        &std::collections::HashSet::new(),
    );
    assert!(tree.ext_lang_allowed("rust").is_none());
    let kept = tree.filter_ext_langs(tree.by_name("field"), tree.ext_lang_allowed("rust"));
    assert_eq!(kept.len(), 1);
}

/// A module-augmentation graft must land on the module's exported INTERFACE,
/// not on a same-named VALUE the module also declares. `@testing-library/jest-dom`
/// augments `declare module 'vitest' { interface Assertion extends
/// TestingLibraryMatchers {} }`, while `vitest` itself declares a value named
/// `Assertion` and re-exports the interface from `@vitest/expect`. Grafting onto
/// the value leaves every `expect(x).toBeInTheDocument()` unresolved, because the
/// receiver types as the re-exported interface.
#[test]
fn module_augmentation_grafts_onto_the_exported_interface_not_a_same_named_value() {
    let arena = Arc::new(TypeArena::new());
    let aug = make_parsed_file(
        "ext:ts:@testing-library/jest-dom/types/vitest.d.ts",
        vec![make_symbol(
            "Assertion",
            "@testing-library/jest-dom.Assertion",
            SymbolKind::Interface,
            None,
            None,
            None,
        )],
        vec![inherits_ref(0, "@testing-library/jest-dom.matchers.TestingLibraryMatchers")],
    );
    let expect_pkg = make_parsed_file(
        "ext:ts:@vitest/expect/index.d.ts",
        vec![make_symbol(
            "Assertion",
            "@vitest/expect.Assertion",
            SymbolKind::Interface,
            None,
            None,
            None,
        )],
        vec![],
    );
    // vitest's own surface: a VALUE named `Assertion` plus the re-export of the
    // interface from @vitest/expect.
    let vitest = make_parsed_file(
        "ext:ts:vitest/index.d.ts",
        vec![make_symbol(
            "Assertion",
            "vitest.Assertion",
            SymbolKind::Variable,
            None,
            None,
            None,
        )],
        vec![named_reexport("Assertion", "@vitest/expect")],
    );

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        ("ext:ts:@vitest/expect/index.d.ts".into(), "@vitest/expect.Assertion".into()),
        900,
    );
    let mut tree = Compilation::build(&[aug, expect_pkg, vitest], &id_map, Arc::clone(&arena));

    tree.apply_module_augmentations(&[(
        "vitest".to_string(),
        "Assertion".to_string(),
        "@testing-library/jest-dom.Assertion".to_string(),
    )]);

    assert_eq!(
        tree.parent_class_qnames("@vitest/expect.Assertion"),
        ["@testing-library/jest-dom.matchers.TestingLibraryMatchers"],
        "the augmentation's supertype must graft onto the interface the module \
         exports, so a matcher declared on it resolves on the receiver"
    );
}

/// A package can export both `type X = …` and the `interface X` whose members
/// the walk needs (`@testing-library/jest-dom` ships
/// `matchersStandalone.TestingLibraryMatchers` beside
/// `matchers.TestingLibraryMatchers`). A supertype named only by its bare head
/// must climb to the declaration that DECLARES members — a member-less alias
/// cannot satisfy the lookup the climb exists for.
#[test]
fn a_bare_supertype_head_climbs_to_the_member_bearing_declaration() {
    let arena = Arc::new(TypeArena::new());
    // The alias is ingested FIRST, so a first-wins pick would take it.
    let alias_file = make_parsed_file(
        "ext:ts:pkg/types/standalone.d.ts",
        vec![make_symbol(
            "Matchers",
            "pkg.standalone.Matchers",
            SymbolKind::TypeAlias,
            None,
            None,
            None,
        )],
        vec![],
    );
    let iface_file = make_parsed_file(
        "ext:ts:pkg/types/matchers.d.ts",
        vec![
            make_symbol("Matchers", "pkg.matchers.Matchers", SymbolKind::Interface, None, None, None),
            make_symbol("toBeVisible", "pkg.matchers.Matchers.toBeVisible", SymbolKind::Method, Some(0), None, None),
        ],
        vec![],
    );
    let child = make_parsed_file(
        "ext:ts:other/index.d.ts",
        vec![make_symbol("Assertion", "other.Assertion", SymbolKind::Interface, None, None, None)],
        vec![inherits_ref(0, "Matchers")],
    );

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("ext:ts:pkg/types/standalone.d.ts".into(), "pkg.standalone.Matchers".into()), 10);
    id_map.insert(("ext:ts:pkg/types/matchers.d.ts".into(), "pkg.matchers.Matchers".into()), 20);
    id_map.insert(("ext:ts:pkg/types/matchers.d.ts".into(), "pkg.matchers.Matchers.toBeVisible".into()), 21);
    id_map.insert(("ext:ts:other/index.d.ts".into(), "other.Assertion".into()), 30);

    let tree = Compilation::build(&[alias_file, iface_file, child], &id_map, Arc::clone(&arena));

    assert_eq!(
        tree.parent_class_ids(30),
        vec![20],
        "the climb must reach the interface that declares the members, not the \
         same-named member-less alias that happened to be ingested first"
    );
}

/// `useBaseQuery(Observer: typeof QueryObserver) { const observer = new Observer(…) }`
/// — `new X()` where `X` names a VALUE in scope (a constructor-typed parameter)
/// yields the constructed INSTANCE. Interning the bare `Observer` instead lets
/// an unrelated package's same-named type win the later head binding.
#[test]
fn field_init_new_through_a_constructor_valued_name_yields_the_instance() {
    use crate::indexer::resolve::engine::testkit::call_ref;
    use crate::type_checker::core::types::Type;

    let arena = Arc::new(TypeArena::new());
    let ctor = arena.intern(Type::Constructor(arena.class("QueryObserver")));
    let scope =
        make_symbol("useBaseQuery", "useBaseQuery", SymbolKind::Function, None, None, None);
    let param = make_symbol(
        "Observer",
        "useBaseQuery.Observer",
        SymbolKind::Property,
        Some(0),
        Some(ctor),
        None,
    );
    let mut local = make_symbol(
        "observer",
        "useBaseQuery.observer",
        SymbolKind::Variable,
        Some(0),
        None,
        None,
    );
    local.scope_path = Some("useBaseQuery".to_string());

    let mut r = call_ref("Observer");
    r.kind = EdgeKind::Instantiates;
    r.source_symbol_index = 2;
    let pf = make_parsed_file("src/u.ts", vec![scope, param, local], vec![r]);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/u.ts".to_string(), "useBaseQuery".to_string()), 1);
    id_map.insert(("src/u.ts".to_string(), "useBaseQuery.Observer".to_string()), 2);
    id_map.insert(("src/u.ts".to_string(), "useBaseQuery.observer".to_string()), 3);

    let mut tree = Compilation::build(std::slice::from_ref(&pf), &id_map, Arc::clone(&arena));
    tree.infer_field_init_types(std::slice::from_ref(&pf), &rustc_hash::FxHashMap::default());
    assert_eq!(
        tree.field_type_id("useBaseQuery.observer").map(|id| arena.format_type(id)).as_deref(),
        Some("QueryObserver"),
        "the instance the constructor value builds, not the bare ctor name",
    );
}
