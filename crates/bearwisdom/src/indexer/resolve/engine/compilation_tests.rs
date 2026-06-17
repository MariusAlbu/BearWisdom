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

#[test]
fn return_type_name_for_find() {
    let (tree, _) = build_fixture();
    assert_eq!(
        tree.return_type_name("Repo.find"),
        Some("User"),
        "return_type_name(Repo.find) should be Some(\"User\")"
    );
}

#[test]
fn field_type_name_for_db() {
    let (tree, _) = build_fixture();
    assert_eq!(
        tree.field_type_name("Repo.db"),
        Some("Database"),
        "field_type_name(Repo.db) should be Some(\"Database\")"
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
        tree.return_type_name("Svc.getUser"),
        Some("User"),
        "return_type_name should be derived from the TypeRef ref"
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
        tree.field_type_name("App.config"),
        Some("Config"),
        "field_type_name should be derived from the TypeRef ref"
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
        tree.return_type_name("load"),
        Some("User"),
        "return_type_name should be derived from the signature string"
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
        tree.return_type_name("fetch"),
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
        tree.return_type_name("useQueryClient"),
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
        tree.return_type_name("useQueryClient"),
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
        tree.return_type_name("useQueryClient"),
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
    let c = |q: &str, t: &str| (q.to_string(), t.to_string(), None, Vec::<String>::new());

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
    let folded: Vec<&str> = mixed.iter().map(|(q, _, _, _)| q.as_str()).collect();
    assert_eq!(folded, vec!["b"], "only the agreeing qname `b` is folded");
}

/// End-to-end: once the inferred return lands, a call-root chain
/// `useQueryClient().clear()` resolves through the chain walker against the
/// `Compilation`, binding the leaf to `QueryClient.clear`.
#[test]
fn inferred_return_lets_call_root_chain_resolve() {
    use crate::indexer::resolve::engine::chain::bind_member_access;
    use crate::indexer::resolve::engine::testkit::{call_ref, ref_ctx, source_symbol};
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

    let resolved = bind_member_access(&rc, &tree).map(|res| res.target_symbol_id);
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
