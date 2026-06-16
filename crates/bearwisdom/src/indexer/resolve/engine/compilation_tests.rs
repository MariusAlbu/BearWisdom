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
