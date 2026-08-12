use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::resolve::engine::contract::{FlowCacheLookup, SymbolLookup};
use crate::type_checker::core::types::TypeArena;
use crate::types::ParsedFile;

use super::{FileLookup, resolve_single_pass};

/// Empty plugin registry for `resolve_one_file` tests that exercise the
/// profile-driven path only — no plugin contributes synthetic imports.
fn no_plugins() -> rustc_hash::FxHashMap<&'static str, &'static dyn crate::languages::LanguagePlugin>
{
    rustc_hash::FxHashMap::default()
}

// ---------------------------------------------------------------------------
// BuiltinSkipRule drain — end-to-end through the full resolve pipeline
// ---------------------------------------------------------------------------

/// A bash script call into a real builtin (`echo`, drained by `BuiltinSkipRule`
/// via the profile's `is_bash_builtin`) lands in `unresolved_refs` with
/// `drained=1`; a call into a genuinely missing project function lands with
/// `drained=0` and still counts against the resolution rate. Both rows must
/// keep their `unresolved_refs` entry — the drain changes classification, not
/// row survival.
#[test]
fn drained_builtin_call_lands_with_flag_and_leaves_the_rate_denominator() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("script.sh"),
        "my_func() {\n  totally_missing_project_function_xyz\n  echo hello\n}\n",
    )
    .unwrap();

    let mut db = crate::db::Database::open_in_memory().unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();

    let conn = db.conn();
    let missing: (i64, i64) = conn
        .query_row(
            "SELECT drained, COUNT(*) FROM unresolved_refs \
             WHERE target_name = 'totally_missing_project_function_xyz' GROUP BY drained",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the genuine miss must have landed a row");
    assert_eq!(missing, (0, 1), "a genuine miss must not be drained");

    let builtin: (i64, i64) = conn
        .query_row(
            "SELECT drained, COUNT(*) FROM unresolved_refs \
             WHERE target_name = 'echo' GROUP BY drained",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the drained builtin call must have landed a row");
    assert_eq!(builtin, (1, 1), "a bash builtin call must be drained");

    // The rate excludes the drained row; only the genuine miss counts.
    let rb = crate::query::stats::resolution_breakdown(&db).unwrap();
    assert_eq!(rb.internal_unresolved, 1);
    assert_eq!(rb.drained_refs, 1);
}

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
    let lookup = FileLookup::new(&tree, "typescript");
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
    let lookup = FileLookup::new(&tree, "typescript");
    lookup.record_local_type("repo".to_string(), "UserRepository".to_string());
    assert_eq!(lookup.local_type("repo").as_deref(), Some("UserRepository"));
}

// ---------------------------------------------------------------------------
// build_file_context — plugin-contributed imports
// ---------------------------------------------------------------------------

/// Minimal plugin double whose `extra_wildcard_imports` always contributes
/// one synthetic entry — stands in for a real plugin's cross-file-derived
/// redirect (Elixir's one-hop `use` injection, etc.) without depending on
/// any language-specific behavior.
struct FakeInjectingPlugin;

impl crate::languages::LanguagePlugin for FakeInjectingPlugin {
    fn id(&self) -> &str {
        "fake"
    }
    fn language_ids(&self) -> &[&str] {
        &["fake"]
    }
    fn extensions(&self) -> &[&str] {
        &[]
    }
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }
    fn scope_kinds(&self) -> &[crate::parser::scope_tree::ScopeKind] {
        &[]
    }
    fn extract(&self, _source: &str, _file_path: &str, _lang_id: &str) -> crate::types::ExtractionResult {
        crate::types::ExtractionResult::default()
    }
    fn extra_wildcard_imports(
        &self,
        _state: &crate::indexer::plugin_state::PluginStateBag,
        _file: &ParsedFile,
    ) -> Vec<crate::indexer::resolve::engine::contract::ImportEntry> {
        vec![crate::indexer::resolve::engine::contract::ImportEntry {
            imported_name: "Injected".to_string(),
            module_path: Some("Some.Module".to_string()),
            alias: None,
            is_wildcard: true,
        }]
    }
}

fn blank_parsed_file(lang: &str) -> ParsedFile {
    ParsedFile {
        path: format!("f.{lang}"),
        language: lang.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

/// A plugin's `extra_wildcard_imports` entry is appended to
/// `FileContext.imports` alongside the profile-driven scan — the generic
/// seam every `LookupRule` that reads `ctx.file_ctx.imports` (wildcard
/// import, alias-module-qname, …) consults without any rule-level change.
#[test]
fn plugin_contributed_import_reaches_file_context() {
    let file = blank_parsed_file("fake");
    let bag = crate::indexer::plugin_state::PluginStateBag::new();
    let profile = &crate::languages::rust_lang::profile::RUST_PROFILE;

    let ctx = super::build_file_context(
        "fake",
        &file,
        profile,
        Some(&FakeInjectingPlugin as &dyn crate::languages::LanguagePlugin),
        Some(&bag),
    );

    assert_eq!(ctx.imports.len(), 1);
    assert_eq!(ctx.imports[0].imported_name, "Injected");
    assert_eq!(ctx.imports[0].module_path.as_deref(), Some("Some.Module"));
    assert!(ctx.imports[0].is_wildcard);
}

/// No plugin registered for the file's language (the common case — most
/// plugins carry no cross-file import state) leaves `imports` exactly as
/// the profile-driven scan produced it.
#[test]
fn no_plugin_leaves_file_context_imports_unchanged() {
    let file = blank_parsed_file("rust");
    let profile = &crate::languages::rust_lang::profile::RUST_PROFILE;

    let ctx = super::build_file_context("rust", &file, profile, None, None);

    assert!(ctx.imports.is_empty());
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
    let lookup = FileLookup::new(&tree, "typescript");
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
    let lookup = FileLookup::new(&tree, "typescript");
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
    let lookup = FileLookup::new(&tree, "typescript");
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
    let lookup = FileLookup::new(&tree, "typescript");

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
    let lookup = FileLookup::new(&tree, "typescript");

    let user_id = arena.class("User");
    let opt_id = arena.intern(Type::Optional(user_id));

    // The string round-trip also preserves the shape now — the trailing-`?`
    // nullable suffix interns as `Optional(User)` rather than nominalizing to
    // a member-less `Class("User?")`. The TypeId cache remains the identity
    // path either way.
    let formatted = arena.format_type(opt_id); // "User?"
    let reinterned = arena.intern_type_str(&formatted);
    assert_eq!(reinterned, opt_id, "intern_type_str(\"User?\") must round-trip to Optional(User)");

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
    let lookup = FileLookup::new(&tree, "typescript");
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
    let (edges, unresolved, _ref_log) = super::resolve_one_file(&pf, &tree, &profiles, &no_plugins(), None, &solver, &id_map);
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
    let (edges, _unresolved, _ref_log) = super::resolve_one_file(&pf, &tree, &profiles, &no_plugins(), None, &solver, &id_map);
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
    let (edges_a, _, _) = super::resolve_one_file(&files[0], &tree, &profiles, &no_plugins(), None, &solver, &id_map);
    assert!(
        edges_a.iter().any(|e| e.1 == mount_a),
        "package A's devtools.mount must bind A's ReactDevtoolsImpl.mount (id {mount_a}); edges={edges_a:?}"
    );
    assert!(
        !edges_a.iter().any(|e| e.1 == mount_b),
        "package A's chain must NOT bind package B's VueDevtoolsImpl.mount (id {mount_b})"
    );

    // Package B resolves to B's mount, NOT A's.
    let (edges_b, _, _) = super::resolve_one_file(&files[1], &tree, &profiles, &no_plugins(), None, &solver, &id_map);
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

    let (_edges, unresolved, _ref_log) =
        super::resolve_one_file(&pf, &tree, &profiles, &no_plugins(), None, &solver, &id_map);

    // The ref to "NonexistentApi" must be unresolved (no matching symbol in the
    // compilation tree) and the unresolved row must carry from_snippet=true.
    assert!(
        !unresolved.is_empty(),
        "NonexistentApi must be unresolved; got no unresolved rows"
    );
    let row = unresolved
        .iter()
        .find(|(_, name, _, _, _, _, _, _, _, _)| name == "NonexistentApi");
    assert!(row.is_some(), "unresolved row for NonexistentApi not found");
    let (_, _, _, _, _, _, from_snippet, _drained, _cause_symbol_id, _cause_kind) = row.unwrap();
    assert!(
        *from_snippet,
        "unresolved ref from a snippet source symbol must have from_snippet=true"
    );
}

// ---------------------------------------------------------------------------
// Await-unwrap seed tests (slice A_stdlib fix)
// ---------------------------------------------------------------------------

/// When a binding's initializer is an `await` expression
/// (`const res = await fetch(url)`), the seed records the UNWRAPPED inner type
/// (`Response`) rather than the async-wrapper (`Promise<Response>`), so a
/// subsequent member call `res.json()` walks `Response`'s members and resolves.
///
/// Exercises the `flow_binding_await` path in `resolve_one_file`'s seed block.
#[test]
fn awaited_binding_strips_promise_wrapper_at_seed() {
    use crate::type_checker::core::types::{Type, TypeArena};
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
        return_type: Option<crate::type_checker::core::types::TypeId>,
    ) -> ExtractedSymbol {
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
            scope_path: scope.map(Into::into),
            parent_index: parent,
            declared_type: None,
            return_type,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }
    }
    fn cseg(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
        ChainSegment {
            name: name.into(),
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
        }
    }
    fn eref(src: usize, target: &str, kind: EdgeKind, chain: Option<MemberChain>, byte: u32) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: src,
            target_name: target.into(),
            kind,
            line: 1,
            col: 0,
            module: None,
            chain,
            byte_offset: byte,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }
    }

    let arena = Arc::new(TypeArena::new());

    // Build `Promise<Response>` as a TypeId in the shared arena so the
    // Compilation ingests it as `fetch`'s return type.
    let response_id = arena.class("Response");
    let promise_id = arena.class("Promise");
    let promise_response_id = arena.intern(Type::Apply {
        base: promise_id,
        args: vec![response_id],
    });

    let symbols = vec![
        // 0: enclosing function `getEpisodes`
        esym("getEpisodes", "getEpisodes", SymbolKind::Function, None, None, None),
        // 1: local `res` — bound by `await fetch(url)`
        esym("res", "getEpisodes.res", SymbolKind::Variable, Some(0), Some("getEpisodes"), None),
        // 2: `Response` class
        esym("Response", "Response", SymbolKind::Class, None, None, None),
        // 3: `Response.json` method
        esym("json", "Response.json", SymbolKind::Method, Some(2), Some("Response"), None),
        // 4: `fetch` function, return type = Promise<Response>
        esym("fetch", "fetch", SymbolKind::Function, None, None, Some(promise_response_id)),
    ];

    let refs = vec![
        // ref 0: `const res = await fetch(url)` — Calls ref on `fetch`.
        // byte_offset=5 places it inside the rhs range the flow runner would
        // record; here flow_binding_lhs is set manually.
        eref(0, "fetch", EdgeKind::Calls, None, 5),
        // ref 1: `res.json()` chain — the member call we expect to resolve.
        eref(0, "json", EdgeKind::Calls, Some(MemberChain {
            segments: vec![
                cseg("res", SegmentKind::Identifier, false),
                cseg("json", SegmentKind::Property, true),
            ],
        }), 20),
    ];

    let mut flow = FlowMeta::default();
    // ref 0's LHS is symbol 1 (`res`).
    flow.flow_binding_lhs.insert(0, 1);
    // `res` was awaited — strip one async-wrapper layer at the seed.
    flow.flow_binding_await.insert(1);

    let pf = ParsedFile {
        path: "api.ts".into(),
        language: "typescript".into(),
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
        flow,
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut id_map = HashMap::new();
    id_map.insert(("api.ts".to_string(), "getEpisodes".to_string()), 1i64);
    id_map.insert(("api.ts".to_string(), "getEpisodes.res".to_string()), 2i64);
    id_map.insert(("api.ts".to_string(), "Response".to_string()), 3i64);
    id_map.insert(("api.ts".to_string(), "Response.json".to_string()), 4i64);
    id_map.insert(("api.ts".to_string(), "fetch".to_string()), 5i64);

    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        std::slice::from_ref(&pf),
        &id_map,
        Arc::clone(&arena),
    );
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();

    let (edges, _unresolved, _ref_log) = super::resolve_one_file(&pf, &tree, &profiles, &no_plugins(), None, &solver, &id_map);

    // `res.json()` must resolve to `Response.json` (id 4).
    assert!(
        edges.iter().any(|e| e.1 == 4),
        "awaited binding `res` must be typed as Response (not Promise), so res.json resolves; edges={edges:?}"
    );
}

/// Regression guard: a non-awaited `Promise<T>` binding must keep its
/// `Promise` head so `.then`/`.catch` still resolve on it.
///
/// Without `flow_binding_await`, the seed records `Promise<Response>` intact,
/// and a `then` member on `Promise` resolves while `json` (on `Response`) does not.
#[test]
fn non_awaited_promise_binding_keeps_promise_head_at_seed() {
    use crate::type_checker::core::types::{Type, TypeArena};
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
        return_type: Option<crate::type_checker::core::types::TypeId>,
    ) -> ExtractedSymbol {
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
            scope_path: scope.map(Into::into),
            parent_index: parent,
            declared_type: None,
            return_type,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }
    }
    fn cseg(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
        ChainSegment {
            name: name.into(),
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
        }
    }
    fn eref(src: usize, target: &str, kind: EdgeKind, chain: Option<MemberChain>, byte: u32) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: src,
            target_name: target.into(),
            kind,
            line: 1,
            col: 0,
            module: None,
            chain,
            byte_offset: byte,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }
    }

    let arena = Arc::new(TypeArena::new());

    let response_id = arena.class("Response");
    let promise_id = arena.class("Promise");
    let promise_response_id = arena.intern(Type::Apply {
        base: promise_id,
        args: vec![response_id],
    });

    let symbols = vec![
        // 0: enclosing function
        esym("usePromise", "usePromise", SymbolKind::Function, None, None, None),
        // 1: `p` — non-awaited binding, keeps Promise head
        esym("p", "usePromise.p", SymbolKind::Variable, Some(0), Some("usePromise"), None),
        // 2: `Promise` class with `then`
        esym("Promise", "Promise", SymbolKind::Class, None, None, None),
        // 3: `Promise.then`
        esym("then", "Promise.then", SymbolKind::Method, Some(2), Some("Promise"), None),
        // 4: `fetch` returns Promise<Response>
        esym("fetch", "fetch", SymbolKind::Function, None, None, Some(promise_response_id)),
    ];

    let refs = vec![
        // ref 0: `const p = fetch(url)` — NOT awaited
        eref(0, "fetch", EdgeKind::Calls, None, 5),
        // ref 1: `p.then(...)` — should resolve to Promise.then
        eref(0, "then", EdgeKind::Calls, Some(MemberChain {
            segments: vec![
                cseg("p", SegmentKind::Identifier, false),
                cseg("then", SegmentKind::Property, true),
            ],
        }), 20),
    ];

    let mut flow = FlowMeta::default();
    // ref 0's LHS is symbol 1 (`p`).
    flow.flow_binding_lhs.insert(0, 1);
    // No flow_binding_await — this binding is NOT awaited.

    let pf = ParsedFile {
        path: "use_promise.ts".into(),
        language: "typescript".into(),
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
        flow,
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut id_map = HashMap::new();
    id_map.insert(("use_promise.ts".to_string(), "usePromise".to_string()), 1i64);
    id_map.insert(("use_promise.ts".to_string(), "usePromise.p".to_string()), 2i64);
    id_map.insert(("use_promise.ts".to_string(), "Promise".to_string()), 3i64);
    id_map.insert(("use_promise.ts".to_string(), "Promise.then".to_string()), 4i64);
    id_map.insert(("use_promise.ts".to_string(), "fetch".to_string()), 5i64);

    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        std::slice::from_ref(&pf),
        &id_map,
        Arc::clone(&arena),
    );
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();

    let (edges, _unresolved, _ref_log) = super::resolve_one_file(&pf, &tree, &profiles, &no_plugins(), None, &solver, &id_map);

    // `p.then()` must resolve to `Promise.then` (id 4) — Promise head is preserved.
    assert!(
        edges.iter().any(|e| e.1 == 4),
        "non-awaited binding `p` must keep Promise head so p.then resolves to Promise.then; edges={edges:?}"
    );
}

// ---------------------------------------------------------------------------
// First-uncaptured-type cause — root-cause cascade attribution
// ---------------------------------------------------------------------------

/// `const logger = createScopedLogger()` where `createScopedLogger`'s own
/// return type is never captured: the call itself resolves (an edge to the
/// factory), but nothing seeds `logger`'s forward-inferred type, so N later
/// `logger.<member>()` refs are ALL unresolved. Every one of them must carry
/// `cause_kind='uncaptured_return'` with `cause_symbol_id` pointing at the
/// factory — the CLAUDE.md worked example: many surface misses, one upstream
/// cause.
#[test]
fn member_refs_on_uncaptured_call_root_all_blame_the_initializer() {
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, MemberChain, ParsedFile,
        SegmentKind, SymbolKind, Visibility,
    };
    fn esym(name: &str, qname: &str, kind: SymbolKind, parent: Option<usize>) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(), qualified_name: qname.into(), kind,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0, byte_offset: 0,
            signature: None, doc_comment: None, scope_path: parent.map(|_| "caller".into()),
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
        esym("caller", "caller", SymbolKind::Function, None),                    // 0
        esym("logger", "caller.logger", SymbolKind::Variable, Some(0)),          // 1
        esym("createScopedLogger", "createScopedLogger", SymbolKind::Function, None), // 2
    ];
    let refs = vec![
        // `const logger = createScopedLogger()` — Calls, flow-bound to `logger`.
        // No return_type on the factory symbol above, so nothing seeds `logger`.
        eref(0, "createScopedLogger", EdgeKind::Calls, None),
        // `logger.info()`
        eref(0, "info", EdgeKind::Calls, Some(MemberChain {
            segments: vec![
                cseg("logger", SegmentKind::Identifier, false),
                cseg("info", SegmentKind::Property, true),
            ],
        })),
        // `logger.warn()`
        eref(0, "warn", EdgeKind::Calls, Some(MemberChain {
            segments: vec![
                cseg("logger", SegmentKind::Identifier, false),
                cseg("warn", SegmentKind::Property, true),
            ],
        })),
    ];
    let mut flow = FlowMeta::default();
    flow.flow_binding_lhs.insert(0, 1); // ref 0's LHS is symbol 1 (`logger`)
    let pf = ParsedFile {
        path: "logger.ts".into(), language: "typescript".into(), content_hash: String::new(),
        size: 0, line_count: 0, mtime: None, package_id: None, symbols, refs,
        routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
        has_errors: false, flow, demand_contributions: Vec::new(),
        alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
    };
    let mut id_map = HashMap::new();
    id_map.insert(("logger.ts".to_string(), "caller".to_string()), 1i64);
    id_map.insert(("logger.ts".to_string(), "caller.logger".to_string()), 2i64);
    id_map.insert(("logger.ts".to_string(), "createScopedLogger".to_string()), 3i64);
    let arena = Arc::new(TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        std::slice::from_ref(&pf),
        &id_map,
        Arc::clone(&arena),
    );
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();
    let (edges, unresolved, _ref_log) =
        super::resolve_one_file(&pf, &tree, &profiles, &no_plugins(), None, &solver, &id_map);

    // The factory call itself resolves.
    assert!(
        edges.iter().any(|e| e.1 == 3),
        "createScopedLogger() call must resolve; edges={edges:?}"
    );
    // Both member refs on `logger` are unresolved, and BOTH blame the factory
    // (id 3) as the uncaptured-return cause — not `logger` itself (id 2).
    assert_eq!(unresolved.len(), 2, "info and warn must both be unresolved; got {unresolved:?}");
    for row in &unresolved {
        let (_, target_name, _, _, _, _, _, _, cause_symbol_id, cause_kind) = row;
        assert_eq!(*cause_symbol_id, Some(3), "{target_name} must blame the factory, not the binding");
        assert_eq!(*cause_kind, Some("uncaptured_return"));
    }
}

/// A rename import ref (`use m::Orig as Bound;`) carries the module's original
/// declared name as a single-segment chain. `build_file_context` must key the
/// entry on the ORIGINAL name — that is what the module's files declare — with
/// the locally bound name as the alias; a plain import keeps the flat shape.
#[test]
fn rename_import_ref_splits_original_name_and_alias() {
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, FlowMeta, MemberChain, ParsedFile, SegmentKind,
    };
    fn import_ref(target: &str, module: &str, original: Option<&str>) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: 0,
            target_name: target.into(),
            kind: EdgeKind::Imports,
            line: 0,
            col: 0,
            module: Some(module.into()),
            chain: original.map(|orig| MemberChain {
                segments: vec![ChainSegment {
                    name: orig.into(),
                    node_kind: "use_as_original".into(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 0,
                    declared_type_id: None,
                    is_call: false,
                    call_args: Vec::new(),
                    type_arg_ids: Vec::new(),
                }],
            }),
            byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }
    }
    let pf = ParsedFile {
        path: "src/main.rs".into(),
        language: "rust".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: vec![
            import_ref("JsonValue", "ser_x", Some("Value")),
            import_ref("Deserializer", "ser_x", None),
        ],
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
    };
    let fc = super::build_file_context(
        "rust",
        &pf,
        &crate::languages::rust_lang::profile::RUST_PROFILE,
        None,
        None,
    );
    let renamed = fc
        .imports
        .iter()
        .find(|i| i.alias.as_deref() == Some("JsonValue"))
        .expect("rename import must land an aliased entry");
    assert_eq!(renamed.imported_name, "Value");
    assert_eq!(renamed.module_path.as_deref(), Some("ser_x"));
    let plain = fc
        .imports
        .iter()
        .find(|i| i.imported_name == "Deserializer")
        .expect("plain import must keep the flat shape");
    assert_eq!(plain.alias, None);
}
/// A rename import whose ORIGINAL name equals a local struct's name (`use
/// ext_pkg::Widget as ExternalWidgetTrait;` next to `pub struct Widget`), with
/// the external declaration materialized and a later crate-rooted import of
/// the same name (`use super::Widget` in a nested mod). Bare `Widget` refs
/// must keep binding the LOCAL struct: the rename binds only its alias, so
/// neither the workspace-package specifier pick nor the same-file yield may
/// treat the original name as imported.
#[test]
fn rename_import_original_name_does_not_shadow_local_struct() {
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, MemberChain, ParsedFile,
        SegmentKind, SymbolKind, Visibility,
    };
    fn esym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(), qualified_name: qname.into(), kind,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0, byte_offset: 0,
            signature: None, doc_comment: None, scope_path: None,
            parent_index: None, declared_type: None, return_type: None,
            param_types: Vec::new(), generic_params: Vec::new(),
        }
    }
    fn import_ref(target: &str, module: &str, original: Option<&str>, line: u32) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false, is_reexport: false, source_symbol_index: 0,
            target_name: target.into(), kind: EdgeKind::Imports,
            line, col: 0, module: Some(module.into()),
            chain: original.map(|orig| MemberChain { segments: vec![ChainSegment {
                name: orig.into(), node_kind: "use_as_original".into(),
                kind: SegmentKind::Identifier, declared_type: None, type_args: Vec::new(),
                optional_chaining: false, byte_offset: 0, declared_type_id: None,
                is_call: false, call_args: Vec::new(), type_arg_ids: Vec::new(),
            }]}),
            byte_offset: 0, namespace_segments: Vec::new(), call_args: Vec::new(),
        }
    }
    let type_ref = ExtractedRef {
        is_import_binding: false, is_reexport: false, source_symbol_index: 1,
        target_name: "Widget".into(), kind: EdgeKind::TypeRef,
        line: 10, col: 0, module: None, chain: None,
        byte_offset: 100, namespace_segments: Vec::new(), call_args: Vec::new(),
    };
    let mut pf = ParsedFile {
        path: "widgets/src/widget.rs".into(), language: "rust".into(),
        content_hash: String::new(), size: 0, line_count: 0, mtime: None, package_id: Some(1),
        symbols: vec![
            esym("Widget", "Widget", SymbolKind::Struct),
            esym("caller", "caller", SymbolKind::Function),
        ],
        refs: vec![
            import_ref("ExternalWidgetTrait", "ext_pkg", Some("Widget"), 3),
            type_ref,
            import_ref("Widget", "crate", None, 200),
        ],
        routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
        has_errors: false, flow: FlowMeta::default(), demand_contributions: Vec::new(),
        alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
    };
    pf.symbols[0].visibility = Some(Visibility::Public);
    let ext_pf = ParsedFile {
        path: "ext:rust:ext_pkg/src/lib.rs".into(), language: "rust".into(),
        content_hash: String::new(), size: 0, line_count: 0, mtime: None, package_id: None,
        symbols: vec![esym("Widget", "Widget", SymbolKind::Interface)],
        refs: Vec::new(),
        routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
        has_errors: false, flow: FlowMeta::default(), demand_contributions: Vec::new(),
        alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
    };
    let mut id_map = HashMap::new();
    id_map.insert(("widgets/src/widget.rs".to_string(), "Widget".to_string()), 1i64);
    id_map.insert(("widgets/src/widget.rs".to_string(), "caller".to_string()), 2i64);
    id_map.insert(("ext:rust:ext_pkg/src/lib.rs".to_string(), "Widget".to_string()), 3i64);
    let arena = Arc::new(TypeArena::new());
    let files = [pf, ext_pf];
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(&files, &id_map, arena);
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();
    let (edges, unresolved, _log) = super::resolve_one_file(&files[0], &tree, &profiles, &no_plugins(), None, &solver, &id_map);
    assert!(
        edges.iter().any(|e| e.1 == 1),
        "bare TypeRef must bind the local struct; edges={edges:?} unresolved={unresolved:?}"
    );
    assert!(
        !edges.iter().any(|e| e.1 == 3),
        "the rename's original name must not divert the bind to the external declaration"
    );
}

// ---------------------------------------------------------------------------
// Cross-language external visibility through the per-file lookup
// ---------------------------------------------------------------------------

/// A minimal ext value file of the given language declaring one typed variable.
fn ext_value_file(
    path: &str,
    language: &str,
    name: &str,
    ty: crate::type_checker::core::types::TypeId,
) -> crate::types::ParsedFile {
    crate::types::ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![crate::types::ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: crate::types::SymbolKind::Variable,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            byte_offset: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            declared_type: Some(ty),
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

/// The per-file lookup drops ext candidates whose language shares no active
/// ecosystem with the resolving file's language, and keeps co-declared ones:
/// a python ext value never serves a rust receiver, while a TS ext value
/// still serves a javascript receiver (both npm-declared).
#[test]
fn file_lookup_by_name_respects_cross_language_ext_visibility() {
    use crate::ecosystem::EcosystemId;
    use crate::indexer::resolve::engine::compilation::Compilation;

    let arena = Arc::new(TypeArena::new());
    let str_ty = arena.class("str");
    let api_ty = arena.class("ApiClient");
    let py = ext_value_file("ext:idx:C:/py/site-packages/fields.py", "python", "field", str_ty);
    let ts = ext_value_file("ext:ts:some-pkg/index.d.ts", "typescript", "client", api_ty);

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert((py.path.clone(), "field".to_string()), 1);
    id_map.insert((ts.path.clone(), "client".to_string()), 2);

    let ctx = crate::indexer::project_context::ProjectContext {
        active_ecosystems: vec![
            EcosystemId::new("cargo"),
            EcosystemId::new("pypi"),
            EcosystemId::new("npm"),
        ],
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &[py, ts],
        &id_map,
        Arc::clone(&arena),
        Some(&ctx),
        &std::collections::HashSet::new(),
    );

    let rust_lookup = FileLookup::new(&tree, "rust");
    assert!(
        rust_lookup.by_name("field").is_empty(),
        "python ext value must not reach a rust file's by-name probe"
    );

    let js_lookup = FileLookup::new(&tree, "javascript");
    assert_eq!(
        js_lookup.by_name("client").len(),
        1,
        "TS ext value must still serve a javascript file"
    );
    assert!(
        js_lookup.by_name("field").is_empty(),
        "python ext value must not reach a javascript file either"
    );

    let py_lookup = FileLookup::new(&tree, "python");
    assert_eq!(py_lookup.by_name("field").len(), 1, "python keeps its own ext surface");
}

/// A plain namespace import (`using System.Linq;`) becomes a WILDCARD entry
/// when the profile opts in — the C# shape. Named binding imports never do.
#[test]
fn namespace_import_entry_is_a_wildcard_under_the_profile_flag() {
    use crate::types::{EdgeKind, ExtractedRef, FlowMeta, ParsedFile};
    fn using_ref(ns: &str) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: 0,
            target_name: ns.into(),
            kind: EdgeKind::Imports,
            line: 0,
            col: 0,
            module: Some(ns.into()),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }
    }
    let pf = ParsedFile {
        path: "src/Program.cs".into(),
        language: "csharp".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: vec![using_ref("System.Linq")],
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
    };
    let fc = super::build_file_context(
        "csharp",
        &pf,
        &crate::languages::csharp::profile::CSHARP_PROFILE,
        None,
        None,
    );
    let entry = fc
        .imports
        .iter()
        .find(|i| i.imported_name == "System.Linq")
        .expect("using directive must land an import entry");
    assert_eq!(entry.module_path.as_deref(), Some("System.Linq"));
    assert!(entry.is_wildcard, "a plain using opens the namespace as a wildcard");
}

// ---------------------------------------------------------------------------
// Ref-site dedup — duplicate emissions collapse before resolution
// ---------------------------------------------------------------------------

/// Extractors can emit the same ref node more than once (double-visited
/// constructs, per-node-kind coverage double-emits). `resolve_one_file` must
/// process each ref SITE exactly once: emissions sharing (source symbol, kind,
/// target, line, byte offset) collapse to one row, while two same-name refs on
/// the same line at distinct byte offsets stay separate rows.
#[test]
fn duplicate_ref_emissions_collapse_to_one_row_per_site() {
    use crate::types::{
        EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
    };
    fn esym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(), qualified_name: qname.into(), kind,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0, byte_offset: 0,
            signature: None, doc_comment: None, scope_path: None,
            parent_index: None, declared_type: None, return_type: None,
            param_types: Vec::new(), generic_params: Vec::new(),
        }
    }
    fn eref(target: &str, line: u32, byte: u32) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false, is_reexport: false, source_symbol_index: 0,
            target_name: target.into(), kind: EdgeKind::Calls, line, col: 0,
            module: None, chain: None, byte_offset: byte,
            namespace_segments: Vec::new(), call_args: Vec::new(),
        }
    }
    let pf = ParsedFile {
        path: "dup.ts".into(), language: "typescript".into(), content_hash: String::new(),
        size: 0, line_count: 0, mtime: None, package_id: None,
        symbols: vec![esym("f", "f", SymbolKind::Function)],
        refs: vec![
            // The same node emitted three times — one row survives.
            eref("missingA", 1, 10),
            eref("missingA", 1, 10),
            eref("missingA", 1, 10),
            // Two distinct same-name sites on one line — both survive in-memory.
            eref("missingB", 1, 30),
            eref("missingB", 1, 40),
        ],
        routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
        has_errors: false, flow: FlowMeta::default(), demand_contributions: Vec::new(),
        alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
    };
    let mut id_map = HashMap::new();
    id_map.insert(("dup.ts".to_string(), "f".to_string()), 1i64);
    let arena = Arc::new(TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        std::slice::from_ref(&pf),
        &id_map,
        arena,
    );
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();
    let (_edges, unresolved, ref_log) =
        super::resolve_one_file(&pf, &tree, &profiles, &no_plugins(), None, &solver, &id_map);

    let count_a = unresolved.iter().filter(|(_, n, ..)| n == "missingA").count();
    let count_b = unresolved.iter().filter(|(_, n, ..)| n == "missingB").count();
    assert_eq!(count_a, 1, "triple emission of one site must land one row; unresolved={unresolved:?}");
    assert_eq!(count_b, 2, "distinct byte offsets on one line are separate sites");
    // The resolution log sees each SITE once too — not each emission.
    assert_eq!(ref_log.len(), 3, "ref_log must carry one row per distinct site; got {ref_log:?}");
}

/// A dotted-FQN import (`import java.util.Map` → target `Map`, module
/// `java.util.Map`; `import com.foo.*` → target `*`, module `com.foo`) must
/// surface its module path in the file context: the import rungs bind a bare
/// name by matching the imported declaration's file path against that module,
/// and the wildcard entry feeds the open-namespace set the chain root anchors
/// through.
#[test]
fn fqn_import_refs_carry_module_path_into_file_context() {
    use crate::types::{EdgeKind, ExtractedRef, FlowMeta, ParsedFile};
    fn import_ref(target: &str, module: &str) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: 0,
            target_name: target.into(),
            kind: EdgeKind::Imports,
            line: 0,
            col: 0,
            module: Some(module.into()),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }
    }
    let pf = ParsedFile {
        path: "src/main/java/org/demo/OrdersService.java".into(),
        language: "java".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: vec![
            import_ref("Map", "java.util.Map"),
            import_ref("*", "com.foo"),
        ],
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
    };
    let fc = super::build_file_context(
        "java",
        &pf,
        &crate::languages::java::profile::JAVA_PROFILE,
        None,
        None,
    );
    let exact = fc
        .imports
        .iter()
        .find(|i| i.imported_name == "Map")
        .expect("exact import must land an entry");
    assert_eq!(exact.module_path.as_deref(), Some("java.util.Map"));
    assert!(!exact.is_wildcard, "an exact import is not a wildcard");
    let wildcard = fc
        .imports
        .iter()
        .find(|i| i.imported_name == "*")
        .expect("wildcard import must land an entry");
    assert_eq!(wildcard.module_path.as_deref(), Some("com.foo"));
    assert!(wildcard.is_wildcard, "a `*` target is a wildcard entry");
}
