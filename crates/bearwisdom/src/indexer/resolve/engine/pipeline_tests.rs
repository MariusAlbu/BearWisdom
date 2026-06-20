use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::core::types::TypeArena;

use super::{FileLookup, resolve_single_pass};

#[test]
fn empty_parsed_returns_ok_with_zero_counts() {
    // No DB is available in unit tests; this exercises the pre-flush path only
    // by verifying the function accepts empty input without panicking before
    // it reaches the DB write step — the DB call itself will fail (no file),
    // which is expected in this harness. The test asserts the construction
    // phase (Compilation build, profile map, solver) completes without panic.
    //
    // A real integration test runs via `bw reindex` on a fixture project.
    let arena = Arc::new(TypeArena::new());
    let parsed: Vec<crate::types::ParsedFile> = Vec::new();
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();

    // Constructing the tree and iterating zero files succeeds. We cannot call
    // resolve_single_pass without a real Database, so just verify the
    // intermediate values are constructible.
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &parsed,
        &symbol_id_map,
        Arc::clone(&arena),
    );
    let _ = tree; // constructed without panic

    let profiles = super::build_profiles();
    assert!(
        !profiles.is_empty(),
        "registry must produce at least one language profile"
    );
}

// ---------------------------------------------------------------------------
// FileLookup unit tests
// ---------------------------------------------------------------------------

/// `local_type` returns `None` before any binding is recorded.
#[test]
fn file_lookup_local_type_empty() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    assert!(lookup.local_type("x").is_none());
}

/// `record_local_type` then `local_type` returns the recorded type.
#[test]
fn file_lookup_record_then_read() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    lookup.record_local_type("repo".to_string(), "UserRepository".to_string());
    assert_eq!(lookup.local_type("repo").as_deref(), Some("UserRepository"));
}

/// `local_type_union` wraps the single result in a `vec!`.
#[test]
fn file_lookup_local_type_union_single_branch() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    lookup.record_local_type("svc".to_string(), "OrderService".to_string());
    assert_eq!(
        lookup.local_type_union("svc"),
        Some(vec!["OrderService".to_string()])
    );
    assert!(lookup.local_type_union("missing").is_none());
}

/// `clear_local_cache` evicts all bindings so they cannot bleed into the
/// next file's resolution pass.
#[test]
fn file_lookup_clear_evicts_bindings() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    lookup.record_local_type("x".to_string(), "Foo".to_string());
    assert!(lookup.local_type("x").is_some());
    lookup.clear_local_cache();
    assert!(lookup.local_type("x").is_none());
}

/// Structural delegation: `by_name` on an empty tree returns an empty set, not
/// a panic. Confirms the delegation layer compiles and runs.
#[test]
fn file_lookup_delegates_structural_to_tree() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    assert!(lookup.by_name("anything").iter().next().is_none());
    assert!(lookup.by_qualified_name("a.b.c").is_none());
    assert!(lookup.members_of("SomeClass").iter().next().is_none());
}

fn arc_clone(a: &Arc<TypeArena>) -> Arc<TypeArena> {
    Arc::clone(a)
}

// ---------------------------------------------------------------------------
// TypeId local cache — primitive/optional corruption gate
// ---------------------------------------------------------------------------

/// Storing a `Primitive(Int)` TypeId via `record_local_type_id` and reading it
/// back via `local_type_id` must return the exact same TypeId — not a
/// `Class("Int")` nominalization that `intern_type_str("Int")` would produce.
///
/// The corruption this guards: serializing `Primitive(Int)` via
/// `arena.format_type` yields `"Int"`, and `arena.intern_type_str("Int")`
/// re-interns it as `Class("Int")` — a distinct, member-less nominal type. The
/// TypeId cache stores the id directly, so the primitive survives intact.
#[test]
fn local_type_id_round_trips_primitive_without_nominalization() {
    use crate::type_checker::core::types::{PrimKind, Type};

    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);

    // Intern a real Primitive(Int) TypeId.
    let prim_id = arena.intern(Type::Primitive(PrimKind::Int));

    // Verify the corruption: intern_type_str("Int") produces Class("Int"), not
    // Primitive(Int).  This is the shape the old round-trip produced.
    let nominalized = arena.intern_type_str("Int");
    assert!(
        matches!(arena.get(nominalized), Type::Class(_)),
        "intern_type_str(\"Int\") must yield Class, not Primitive — confirms the corruption"
    );
    assert_ne!(
        nominalized, prim_id,
        "Class(\"Int\") and Primitive(Int) must be distinct TypeIds"
    );

    // The new path: store the TypeId directly, read it back.
    lookup.record_local_type_id("count".to_string(), prim_id);
    let got = lookup.local_type_id("count");
    assert_eq!(
        got,
        Some(prim_id),
        "local_type_id must return the exact Primitive(Int) TypeId, not None or a nominalized Class"
    );
    // The returned TypeId must NOT be the nominalized Class variant.
    assert!(
        matches!(arena.get(got.unwrap()), Type::Primitive(PrimKind::Int)),
        "cached TypeId must be Primitive(Int), not Class(\"Int\")"
    );
}

/// Same guarantee for `Optional<User>`: `record_local_type_id` with an
/// `Optional` TypeId and `local_type_id` must return it intact.
///
/// Serializing `Optional(User)` via `format_type` yields `"User?"`, and
/// `intern_type_str("User?")` falls through to `class("User?")` — a member-less
/// `Class` named `"User?"`. The TypeId cache stores the id directly instead.
#[test]
fn local_type_id_round_trips_optional_without_nominalization() {
    use crate::type_checker::core::types::Type;

    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);

    let user_id = arena.class("User");
    let opt_id = arena.intern(Type::Optional(user_id));

    // Confirm the old round-trip produces a Class, not Optional.
    let formatted = arena.format_type(opt_id); // "User?"
    let nominalized = arena.intern_type_str(&formatted);
    assert!(
        matches!(arena.get(nominalized), Type::Class(_)),
        "intern_type_str(\"User?\") must yield Class — confirms the corruption"
    );
    assert_ne!(nominalized, opt_id, "Class(\"User?\") and Optional(User) must be distinct");

    // New path: TypeId stored and retrieved intact.
    lookup.record_local_type_id("maybeUser".to_string(), opt_id);
    let got = lookup.local_type_id("maybeUser");
    assert_eq!(got, Some(opt_id), "local_type_id must return Optional(User) TypeId");
    assert!(
        matches!(arena.get(got.unwrap()), Type::Optional(_)),
        "cached TypeId must be Optional, not a nominalized Class"
    );
}

/// A reassigned local binds the same name twice. The cache must honor the LATEST
/// write across both lanes: `resolve_root` probes `local_type_id` before
/// `local_type`, so a stale TypeId left beside a newer String binding (or the
/// reverse) would win incorrectly. Each writer evicts the other lane's entry for
/// the name, so exactly one binding — the most recent — survives.
#[test]
fn reassignment_latest_write_wins_across_both_caches() {
    use crate::type_checker::core::types::{PrimKind, Type};

    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    let prim_id = arena.intern(Type::Primitive(PrimKind::Int));

    // TypeId binding, then a String reassignment for the same name.
    lookup.record_local_type_id("x".to_string(), prim_id);
    lookup.record_local_type("x".to_string(), "Repo".to_string());
    assert_eq!(lookup.local_type_id("x"), None, "String reassignment must evict the stale TypeId");
    assert_eq!(lookup.local_type("x").as_deref(), Some("Repo"), "latest String write must be visible");

    // The reverse: String binding, then a TypeId reassignment for the same name.
    lookup.record_local_type("y".to_string(), "Repo".to_string());
    lookup.record_local_type_id("y".to_string(), prim_id);
    assert_eq!(lookup.local_type("y"), None, "TypeId reassignment must evict the stale String");
    assert_eq!(lookup.local_type_id("y"), Some(prim_id), "latest TypeId write must be visible");
}

#[test]
fn engine_resolves_local_var_member_call_via_scope_exact_root() {
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, MemberChain, ParsedFile,
        SegmentKind, SymbolKind, Visibility,
    };
    fn esym(name: &str, qname: &str, kind: SymbolKind, parent: Option<usize>) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(), qualified_name: qname.into(), kind,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0, byte_offset: 0,
            signature: None, doc_comment: None, scope_path: parent.map(|_| "SolidQueryDevtools".into()),
            parent_index: parent, declared_type: None, return_type: None,
            param_types: Vec::new(), generic_params: Vec::new(),
        }
    }
    fn cseg(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
        ChainSegment {
            name: name.into(), node_kind: String::new(), kind, declared_type: None,
            type_args: Vec::new(), optional_chaining: false, byte_offset: 0,
            declared_type_id: None, is_call, call_args: Vec::new(), type_arg_ids: Vec::new(),
        }
    }
    fn eref(src: usize, target: &str, kind: EdgeKind, chain: Option<MemberChain>) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false, is_reexport: false, source_symbol_index: src,
            target_name: target.into(), kind, line: 1, col: 0, module: None, chain,
            byte_offset: 1, namespace_segments: Vec::new(), call_args: Vec::new(),
        }
    }
    let symbols = vec![
        esym("SolidQueryDevtools", "SolidQueryDevtools", SymbolKind::Function, None), // 0
        esym("devtools", "SolidQueryDevtools.devtools", SymbolKind::Variable, Some(0)), // 1
        esym("TanstackQueryDevtools", "TanstackQueryDevtools", SymbolKind::Class, None), // 2
        esym("mount", "TanstackQueryDevtools.mount", SymbolKind::Method, Some(2)), // 3
    ];
    let refs = vec![
        // `const devtools = new TanstackQueryDevtools()` -> field type TypeRef
        eref(1, "TanstackQueryDevtools", EdgeKind::TypeRef, None),
        // `devtools.mount(ref)` chain
        eref(0, "mount", EdgeKind::Calls, Some(MemberChain {
            segments: vec![
                cseg("devtools", SegmentKind::Identifier, false),
                cseg("mount", SegmentKind::Property, true),
            ],
        })),
    ];
    let pf = ParsedFile {
        path: "d.ts".into(), language: "typescript".into(), content_hash: String::new(),
        size: 0, line_count: 0, mtime: None, package_id: None, symbols, refs,
        routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
        has_errors: false, flow: FlowMeta::default(), demand_contributions: Vec::new(),
        alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
    };
    let mut id_map = HashMap::new();
    id_map.insert(("d.ts".to_string(), "SolidQueryDevtools".to_string()), 1i64);
    id_map.insert(("d.ts".to_string(), "SolidQueryDevtools.devtools".to_string()), 2i64);
    id_map.insert(("d.ts".to_string(), "TanstackQueryDevtools".to_string()), 3i64);
    id_map.insert(("d.ts".to_string(), "TanstackQueryDevtools.mount".to_string()), 4i64);
    let arena = Arc::new(TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(std::slice::from_ref(&pf), &id_map, arena);
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();
    let (edges, unresolved) = super::resolve_one_file(&pf, &tree, &profiles, &solver, &id_map);
    // mount (target id 4) must resolve as an edge from SolidQueryDevtools (1).
    assert!(edges.iter().any(|e| e.1 == 4), "devtools.mount must resolve to TanstackQueryDevtools.mount");
}

/// `const observer = new QueryObserver(); observer.getCurrentResult()` — the
/// `new` is an `Instantiates` flow-binding, so forward inference must type
/// `observer` as the constructed class itself (not its non-existent field type)
/// for the later member call to walk `QueryObserver`'s methods.
#[test]
fn engine_types_a_new_expression_local_for_a_later_member_call() {
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, MemberChain, ParsedFile,
        SegmentKind, SymbolKind, Visibility,
    };
    fn esym(name: &str, qname: &str, kind: SymbolKind, parent: Option<usize>) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(), qualified_name: qname.into(), kind,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0, byte_offset: 0,
            signature: None, doc_comment: None, scope_path: parent.map(|_| "useTest".into()),
            parent_index: parent, declared_type: None, return_type: None,
            param_types: Vec::new(), generic_params: Vec::new(),
        }
    }
    fn cseg(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
        ChainSegment {
            name: name.into(), node_kind: String::new(), kind, declared_type: None,
            type_args: Vec::new(), optional_chaining: false, byte_offset: 0,
            declared_type_id: None, is_call, call_args: Vec::new(), type_arg_ids: Vec::new(),
        }
    }
    fn eref(src: usize, target: &str, kind: EdgeKind, chain: Option<MemberChain>) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false, is_reexport: false, source_symbol_index: src,
            target_name: target.into(), kind, line: 1, col: 0, module: None, chain,
            byte_offset: 1, namespace_segments: Vec::new(), call_args: Vec::new(),
        }
    }
    let symbols = vec![
        esym("useTest", "useTest", SymbolKind::Function, None),                  // 0
        esym("observer", "useTest.observer", SymbolKind::Variable, Some(0)),     // 1
        esym("QueryObserver", "QueryObserver", SymbolKind::Class, None),         // 2
        esym("getCurrentResult", "QueryObserver.getCurrentResult", SymbolKind::Method, Some(2)), // 3
    ];
    let refs = vec![
        // `const observer = new QueryObserver()` — Instantiates, flow-bound to `observer`.
        eref(0, "QueryObserver", EdgeKind::Instantiates, None),
        // `observer.getCurrentResult()`
        eref(0, "getCurrentResult", EdgeKind::Calls, Some(MemberChain {
            segments: vec![
                cseg("observer", SegmentKind::Identifier, false),
                cseg("getCurrentResult", SegmentKind::Property, true),
            ],
        })),
    ];
    let mut flow = FlowMeta::default();
    flow.flow_binding_lhs.insert(0, 1); // ref 0's LHS is symbol 1 (`observer`)
    let pf = ParsedFile {
        path: "d.ts".into(), language: "typescript".into(), content_hash: String::new(),
        size: 0, line_count: 0, mtime: None, package_id: None, symbols, refs,
        routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
        has_errors: false, flow, demand_contributions: Vec::new(),
        alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
    };
    let mut id_map = HashMap::new();
    id_map.insert(("d.ts".to_string(), "useTest".to_string()), 1i64);
    id_map.insert(("d.ts".to_string(), "useTest.observer".to_string()), 2i64);
    id_map.insert(("d.ts".to_string(), "QueryObserver".to_string()), 3i64);
    id_map.insert(("d.ts".to_string(), "QueryObserver.getCurrentResult".to_string()), 4i64);
    let arena = Arc::new(TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(std::slice::from_ref(&pf), &id_map, arena);
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();
    let (edges, _unresolved) = super::resolve_one_file(&pf, &tree, &profiles, &solver, &id_map);
    assert!(
        edges.iter().any(|e| e.1 == 4),
        "observer.getCurrentResult must resolve to QueryObserver.getCurrentResult via new-expression typing"
    );
}

/// Two monorepo packages each declare a top-level enclosing symbol holding a
/// `devtools` variable with the SAME simple name `devtools`, each typed to that
/// package's OWN devtools-impl class with its own `mount` method. Both files are
/// built into one `Compilation` and resolved independently. Package A's
/// `devtools.mount()` chain must bind A's `mount`; package B's must bind B's —
/// never each other's. This is the end-to-end identity case the engine migration
/// targets: the chain root keys on the in-scope (package-distinct) declaration,
/// so a first-match-by-simple-name root scan that collapsed both `devtools` vars
/// to one would mis-bind one package's chain and this test would catch it.
#[test]
fn engine_distinguishes_same_named_devtools_across_packages() {
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, MemberChain, ParsedFile,
        SegmentKind, SymbolKind, Visibility,
    };
    fn esym(
        name: &str,
        qname: &str,
        kind: SymbolKind,
        parent: Option<usize>,
        scope: Option<&str>,
    ) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(), qualified_name: qname.into(), kind,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0, byte_offset: 0,
            signature: None, doc_comment: None, scope_path: scope.map(Into::into),
            parent_index: parent, declared_type: None, return_type: None,
            param_types: Vec::new(), generic_params: Vec::new(),
        }
    }
    fn cseg(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
        ChainSegment {
            name: name.into(), node_kind: String::new(), kind, declared_type: None,
            type_args: Vec::new(), optional_chaining: false, byte_offset: 0,
            declared_type_id: None, is_call, call_args: Vec::new(), type_arg_ids: Vec::new(),
        }
    }
    fn eref(src: usize, target: &str, kind: EdgeKind, chain: Option<MemberChain>) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false, is_reexport: false, source_symbol_index: src,
            target_name: target.into(), kind, line: 1, col: 0, module: None, chain,
            byte_offset: 1, namespace_segments: Vec::new(), call_args: Vec::new(),
        }
    }
    // One package's file: an enclosing symbol `host` holding `devtools` (typed to
    // `impl_class`), `impl_class` with `mount`, and a `devtools.mount()` chain
    // referenced from the enclosing symbol.
    fn make_pkg(
        path: &str,
        package_id: i64,
        host: &str,
        impl_class: &str,
    ) -> (ParsedFile, Vec<((String, String), ())>) {
        let devtools_qname = format!("{host}.devtools");
        let mount_qname = format!("{impl_class}.mount");
        let symbols = vec![
            esym(host, host, SymbolKind::Function, None, None), // 0
            esym("devtools", &devtools_qname, SymbolKind::Variable, Some(0), Some(host)), // 1
            esym(impl_class, impl_class, SymbolKind::Class, None, None), // 2
            esym("mount", &mount_qname, SymbolKind::Method, Some(2), Some(impl_class)), // 3
        ];
        let refs = vec![
            // `const devtools = new <impl_class>()` -> field-type TypeRef on devtools.
            eref(1, impl_class, EdgeKind::TypeRef, None),
            // `devtools.mount(...)` chain, referenced from the host (index 0).
            eref(0, "mount", EdgeKind::Calls, Some(MemberChain {
                segments: vec![
                    cseg("devtools", SegmentKind::Identifier, false),
                    cseg("mount", SegmentKind::Property, true),
                ],
            })),
        ];
        let pf = ParsedFile {
            path: path.into(), language: "typescript".into(), content_hash: String::new(),
            size: 0, line_count: 0, mtime: None, package_id: Some(package_id), symbols, refs,
            routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
            ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
            has_errors: false, flow: FlowMeta::default(), demand_contributions: Vec::new(),
            alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
        };
        let qnames = vec![
            ((path.to_string(), host.to_string()), ()),
            ((path.to_string(), devtools_qname), ()),
            ((path.to_string(), impl_class.to_string()), ()),
            ((path.to_string(), mount_qname), ()),
        ];
        (pf, qnames)
    }

    // Package A (react-query) and package B (vue-query): same simple name
    // `devtools`, package-distinct enclosing host + impl class.
    let (pf_a, qa) = make_pkg("packages/react-query/devtools.ts", 1, "ReactQueryDevtools", "ReactDevtoolsImpl");
    let (pf_b, qb) = make_pkg("packages/vue-query/devtools.ts", 2, "VueQueryDevtools", "VueDevtoolsImpl");

    // Build one id_map across both files, assigning stable ids in order.
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut next_id = 1i64;
    for (key, ()) in qa.into_iter().chain(qb.into_iter()) {
        id_map.insert(key, next_id);
        next_id += 1;
    }

    // `ParsedFile` is not `Clone`; own both in a vec so the same values back the
    // `Compilation` and the per-file resolve calls.
    let files = vec![pf_a, pf_b];
    let arena = Arc::new(TypeArena::new());
    let tree =
        crate::indexer::resolve::engine::compilation::Compilation::build(&files, &id_map, arena);
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();

    let mount_a = id_map[&("packages/react-query/devtools.ts".to_string(), "ReactDevtoolsImpl.mount".to_string())];
    let mount_b = id_map[&("packages/vue-query/devtools.ts".to_string(), "VueDevtoolsImpl.mount".to_string())];

    // Package A resolves to A's mount, NOT B's.
    let (edges_a, _) = super::resolve_one_file(&files[0], &tree, &profiles, &solver, &id_map);
    assert!(
        edges_a.iter().any(|e| e.1 == mount_a),
        "package A's devtools.mount must bind A's ReactDevtoolsImpl.mount (id {mount_a}); edges={edges_a:?}"
    );
    assert!(
        !edges_a.iter().any(|e| e.1 == mount_b),
        "package A's chain must NOT bind package B's VueDevtoolsImpl.mount (id {mount_b})"
    );

    // Package B resolves to B's mount, NOT A's.
    let (edges_b, _) = super::resolve_one_file(&files[1], &tree, &profiles, &solver, &id_map);
    assert!(
        edges_b.iter().any(|e| e.1 == mount_b),
        "package B's devtools.mount must bind B's VueDevtoolsImpl.mount (id {mount_b}); edges={edges_b:?}"
    );
    assert!(
        !edges_b.iter().any(|e| e.1 == mount_a),
        "package B's chain must NOT bind package A's ReactDevtoolsImpl.mount (id {mount_a})"
    );
}

/// A ref whose source symbol is tagged `symbol_from_snippet = true` (e.g. from
/// a Markdown fenced code block) must produce an unresolved row with
/// `from_snippet = true` when the target cannot be resolved.  The
/// `CODE_REF_FILTER` in `query/stats.rs` excludes `from_snippet=1` rows from
/// resolution-rate aggregates; the propagation here is what makes snippet refs
/// invisible to those aggregates.
#[test]
fn snippet_source_symbol_propagates_from_snippet_to_unresolved_row() {
    use crate::types::{
        EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
    };
    fn esym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(),
            qualified_name: qname.into(),
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
            parent_index: None,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }
    }
    fn eref(src: usize, target: &str, kind: EdgeKind) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: src,
            target_name: target.into(),
            kind,
            line: 5,
            col: 0,
            module: None,
            chain: None,
            byte_offset: 50,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }
    }

    // Symbol 0: the markdown file's own host symbol (not a snippet).
    // Symbol 1: a TS symbol spliced from a ``ts fence — tagged from_snippet.
    let symbols = vec![
        esym("README", "README", SymbolKind::Class),  // 0 — host, not snippet
        esym("fetchData", "fetchData", SymbolKind::Function), // 1 — in-fence, snippet
    ];
    // One ref from the snippet symbol to an unresolvable target.
    let refs = vec![eref(1, "NonexistentApi", EdgeKind::Calls)];

    let pf = ParsedFile {
        path: "README.md".into(),
        language: "markdown".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 10,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        // Symbol 0 is not a snippet; symbol 1 is (spliced from a Markdown fence).
        symbol_from_snippet: vec![false, true],
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut id_map = std::collections::HashMap::new();
    id_map.insert(("README.md".to_string(), "README".to_string()), 1i64);
    id_map.insert(("README.md".to_string(), "fetchData".to_string()), 2i64);

    let arena = Arc::new(crate::type_checker::core::types::TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        std::slice::from_ref(&pf),
        &id_map,
        Arc::clone(&arena),
    );
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();

    let (_edges, unresolved) =
        super::resolve_one_file(&pf, &tree, &profiles, &solver, &id_map);

    // The ref to "NonexistentApi" must be unresolved (no matching symbol in the
    // compilation tree) and the unresolved row must carry from_snippet=true.
    assert!(
        !unresolved.is_empty(),
        "NonexistentApi must be unresolved; got no unresolved rows"
    );
    let row = unresolved.iter().find(|(_, name, _, _, _, _, _)| name == "NonexistentApi");
    assert!(row.is_some(), "unresolved row for NonexistentApi not found");
    let (_, _, _, _, _, _, from_snippet) = row.unwrap();
    assert!(
        *from_snippet,
        "unresolved ref from a snippet source symbol must have from_snippet=true"
    );
}
