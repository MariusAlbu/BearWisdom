// =============================================================================
// type_checker/core/members_tests.rs — Unit tests for MembersIndex.
// =============================================================================

use super::*;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::EdgeKind;
use std::sync::Arc;

fn sym(id: i64, name: &str, qname: &str, kind: &str, scope: Option<&str>) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from("test.rs"),
        scope_path: scope.map(|s| s.to_string()),
        package_id: None,
        signature: None,
    }
}

fn sym_sig(id: i64, name: &str, qname: &str, kind: &str, scope: Option<&str>, sig: &str) -> SymbolInfo {
    SymbolInfo {
        signature: Some(sig.to_string()),
        ..sym(id, name, qname, kind, scope)
    }
}

fn empty_supertypes() -> SupertypeGraph {
    SupertypeGraph::new()
}

#[test]
fn overload_selected_by_arity() {
    let mut arena = TypeArena::new();
    let svc = arena.class("Svc");
    let mut index = MembersIndex::new();
    // Two `process` overloads on the same type, distinct arities.
    index.add_direct(svc, sym_sig(1, "process", "Svc.process", "method", Some("Svc"), "process(x)"));
    index.add_direct(svc, sym_sig(2, "process", "Svc.process", "method", Some("Svc"), "process(x, y)"));
    let graph = empty_supertypes();

    let two = index
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(2))
        .expect("hit");
    assert_eq!(two.0.id, 2, "two args should pick the two-param overload");

    let one = index
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(1))
        .expect("hit");
    assert_eq!(one.0.id, 1, "one arg should pick the one-param overload");

    // Unknown arity, and an arity that matches no overload, both fall back to
    // the first declared — the pre-arity behavior.
    let unknown = index
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, None)
        .expect("hit");
    assert_eq!(unknown.0.id, 1);
    let no_match = index
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(3))
        .expect("hit");
    assert_eq!(no_match.0.id, 1);
}

#[test]
fn empty_index_returns_none() {
    let arena = TypeArena::new();
    let mut arena = arena;
    let user = arena.class("User");
    let index = MembersIndex::new();
    let graph = empty_supertypes();
    assert!(index
        .lookup(
            user,
            "anything",
            EdgeKind::Calls,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .is_none());
}

#[test]
fn direct_member_lookup_hits() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "greet", "User.greet", "method", Some("User")));

    let graph = empty_supertypes();
    let found = index
        .lookup(user, "greet", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("should hit");
    assert_eq!(found.id, 1);
    assert_eq!(found.qualified_name, "User.greet");
}

#[test]
fn miss_returns_none_when_no_member_matches() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "greet", "User.greet", "method", Some("User")));

    let graph = empty_supertypes();
    assert!(index
        .lookup(
            user,
            "missing",
            EdgeKind::Calls,
            &graph,
            &arena,
            &DEFAULT_PROFILE
        )
        .is_none());
}

#[test]
fn lookup_walks_supertypes_for_inherited_members() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let admin = arena.class("Admin");

    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "greet", "User.greet", "method", Some("User")));

    let mut graph = SupertypeGraph::new();
    graph.add_edge(admin, user);

    let found = index
        .lookup(admin, "greet", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("inherited member");
    assert_eq!(found.id, 1);
}

#[test]
fn apply_recurses_to_base() {
    let mut arena = TypeArena::new();
    let list = arena.class("List");
    let user = arena.class("User");
    let list_user = arena.intern(Type::Apply {
        base: list,
        args: vec![user],
    });

    let mut index = MembersIndex::new();
    index.add_direct(list, sym(1, "push", "List.push", "method", Some("List")));

    let graph = empty_supertypes();
    let found = index
        .lookup(
            list_user,
            "push",
            EdgeKind::Calls,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .expect("found on base of Apply");
    assert_eq!(found.id, 1);
}

#[test]
fn optional_returns_none_when_profile_disables_peel() {
    use crate::type_checker::profile::language_profile::LanguageProfile;

    let no_peel_profile = LanguageProfile {
        look_through_optional: false,
        ..crate::type_checker::profile::language_profile::DEFAULT_PROFILE
    };

    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let opt_user = arena.intern(Type::Optional(user));

    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "id", "User.id", "field", Some("User")));

    let graph = empty_supertypes();
    // Member exists on the unwrapped User but profile gate forbids peeling.
    assert!(index
        .lookup(
            opt_user,
            "id",
            EdgeKind::TypeRef,
            &graph,
            &arena,
            &no_peel_profile,
        )
        .is_none());
}

#[test]
fn optional_peels_when_profile_allows() {
    // DEFAULT_PROFILE has look_through_optional = true.
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let opt_user = arena.intern(Type::Optional(user));

    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "id", "User.id", "field", Some("User")));

    let graph = empty_supertypes();
    let found = index
        .lookup(
            opt_user,
            "id",
            EdgeKind::TypeRef,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .expect("peeled Optional");
    assert_eq!(found.id, 1);
}

#[test]
fn async_wrapper_peels_through() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let promise_user = arena.intern(Type::AsyncWrapper(user));

    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "id", "User.id", "field", Some("User")));

    let graph = empty_supertypes();
    let found = index
        .lookup(
            promise_user,
            "id",
            EdgeKind::TypeRef,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .expect("peeled AsyncWrapper");
    assert_eq!(found.id, 1);
}

#[test]
fn iterator_wrapper_peels_through() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let iter_user = arena.intern(Type::Iterator(user));

    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "id", "User.id", "field", Some("User")));

    let graph = empty_supertypes();
    let found = index
        .lookup(
            iter_user,
            "id",
            EdgeKind::TypeRef,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .expect("peeled Iterator");
    assert_eq!(found.id, 1);
}

#[test]
fn union_requires_member_in_every_branch() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let admin = arena.class("Admin");
    let union = arena.intern(Type::Union(vec![user, admin]));

    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "id", "User.id", "field", Some("User")));
    // Admin missing id intentionally.
    index.add_direct(admin, sym(2, "level", "Admin.level", "field", Some("Admin")));

    let graph = empty_supertypes();
    assert!(index
        .lookup(union, "id", EdgeKind::TypeRef, &graph, &arena, &DEFAULT_PROFILE)
        .is_none());

    // Now add id to Admin too — union resolves.
    index.add_direct(admin, sym(3, "id", "Admin.id", "field", Some("Admin")));
    let found = index
        .lookup(union, "id", EdgeKind::TypeRef, &graph, &arena, &DEFAULT_PROFILE)
        .expect("union has id everywhere");
    // First branch's match wins.
    assert_eq!(found.id, 1);
}

#[test]
fn intersection_takes_first_matching_branch() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let admin = arena.class("Admin");
    let inter = arena.intern(Type::Intersection(vec![user, admin]));

    let mut index = MembersIndex::new();
    // Only Admin has level.
    index.add_direct(admin, sym(7, "level", "Admin.level", "field", Some("Admin")));

    let graph = empty_supertypes();
    let found = index
        .lookup(
            inter,
            "level",
            EdgeKind::TypeRef,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .expect("intersection picks any branch with member");
    assert_eq!(found.id, 7);
}

#[test]
fn extensions_are_consulted_after_direct() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");

    let mut index = MembersIndex::new();
    // No direct member named "extra"; extension provides it.
    index.add_extension(user, sym(11, "extra", "Ext.extra", "method", None));

    let graph = empty_supertypes();
    let found = index
        .lookup(
            user,
            "extra",
            EdgeKind::Calls,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .expect("extension surfaces");
    assert_eq!(found.id, 11);
}

#[test]
fn direct_member_wins_over_extension_with_same_name() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");

    let mut index = MembersIndex::new();
    index.add_direct(user, sym(1, "name", "User.name", "field", Some("User")));
    index.add_extension(user, sym(2, "name", "Ext.name", "method", None));

    let graph = empty_supertypes();
    let found = index
        .lookup(
            user,
            "name",
            EdgeKind::TypeRef,
            &graph,
            &arena,
            &DEFAULT_PROFILE,
        )
        .expect("hit");
    assert_eq!(found.id, 1, "direct member must win");
}

#[test]
fn function_and_tuple_and_literal_carry_no_members() {
    let mut arena = TypeArena::new();
    let int_ty = arena.primitive(PrimKind::Int);
    let fn_ty = arena.intern(Type::Function {
        params: vec![int_ty],
        return_: int_ty,
    });
    let tup_ty = arena.intern(Type::Tuple(vec![int_ty, int_ty]));
    let lit_ty = arena.intern(Type::Literal(crate::type_checker::core::types::LitValue::Str(
        "x".into(),
    )));
    let unk = arena.intern(Type::Unknown);

    let index = MembersIndex::new();
    let graph = empty_supertypes();
    for ty in [fn_ty, tup_ty, lit_ty, unk] {
        assert!(
            index
                .lookup(
                    ty,
                    "anything",
                    EdgeKind::Calls,
                    &graph,
                    &arena,
                    &DEFAULT_PROFILE,
                )
                .is_none(),
            "no members on {ty:?}"
        );
    }
}

#[test]
fn restrictive_kind_table_filters_incompatible_kinds() {
    // Construct a profile that only accepts Method as a target for Calls.
    // A field with matching name must be skipped.
    use crate::type_checker::profile::language_profile::LanguageProfile;
    use crate::types::SymbolKind;

    const STRICT_TABLE: crate::type_checker::profile::language_profile::KindTable =
        &[(EdgeKind::Calls, &[SymbolKind::Method])];

    let strict_profile = LanguageProfile {
        kind_compatible_table: STRICT_TABLE,
        ..crate::type_checker::profile::language_profile::DEFAULT_PROFILE
    };

    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let mut index = MembersIndex::new();
    // Field named `greet` (wrong kind for Calls) AND method named `greet`
    // declared in that order. Restrictive table must skip the field.
    index.add_direct(user, sym(1, "greet", "User.greet", "field", Some("User")));
    index.add_direct(user, sym(2, "greet", "User.greet", "method", Some("User")));

    let graph = empty_supertypes();
    let found = index
        .lookup(user, "greet", EdgeKind::Calls, &graph, &arena, &strict_profile)
        .expect("method should resolve despite field shadowing the name");
    assert_eq!(found.id, 2, "field skipped by kind filter; method wins");
}

#[test]
fn direct_keys_iterates_registered_types() {
    let mut arena = TypeArena::new();
    let a = arena.class("A");
    let b = arena.class("B");
    let c = arena.class("C");

    let mut index = MembersIndex::new();
    index.add_direct(a, sym(1, "x", "A.x", "field", Some("A")));
    index.add_direct(b, sym(2, "x", "B.x", "field", Some("B")));

    let mut keys: Vec<TypeId> = index.direct_keys().collect();
    keys.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(keys, expected);
    assert!(!keys.contains(&c));
}

#[test]
fn generic_type_members_keyed_under_both_bare_and_parameterized() {
    // A method of a generic type carries the impl's type params in its
    // scope_path (`IndexWriter<D>`). It must be reachable both from a `self`
    // receiver that keeps the params and from a receiver normalized to the
    // bare base. build_from_parsed_files dual-keys it.
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

    let method = ExtractedSymbol {
        name: "add_document".to_string(),
        qualified_name: "IndexWriter<D>.add_document".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: Some("IndexWriter<D>".to_string()),
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    };
    let pf = ParsedFile {
        path: "lib.rs".to_string(),
        language: "rust".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![method],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };
    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("lib.rs".to_string(), 0), 7);

    let arena = TypeArena::new();
    let index = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &arena,
    );

    let bare = arena.class("IndexWriter");
    let parameterized = arena.class("IndexWriter<D>");
    assert!(
        index.direct_of(bare).iter().any(|m| m.id == 7),
        "member must be reachable from the bare base type"
    );
    assert!(
        index.direct_of(parameterized).iter().any(|m| m.id == 7),
        "member must stay reachable from the parameterized self type"
    );
}

#[test]
fn non_generic_type_members_keyed_once() {
    // A non-generic type's scope equals its bare name — no duplicate key.
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

    let method = ExtractedSymbol {
        name: "schema".to_string(),
        qualified_name: "Searcher.schema".to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: Some("Searcher".to_string()),
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    };
    let pf = ParsedFile {
        path: "lib.rs".to_string(),
        language: "rust".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![method],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };
    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("lib.rs".to_string(), 0), 9);

    let arena = TypeArena::new();
    let index = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &arena,
    );

    let searcher = arena.class("Searcher");
    assert_eq!(
        index.direct_of(searcher).iter().filter(|m| m.id == 9).count(),
        1,
        "non-generic member is keyed exactly once"
    );
}

#[test]
fn csharp_extension_target_recognises_simple_signature() {
    assert_eq!(
        super::csharp_extension_target("string MyExt(this string s, int x)"),
        Some("string")
    );
}

#[test]
fn csharp_extension_target_handles_generic_receiver() {
    assert_eq!(
        super::csharp_extension_target(
            "T MyExtAsync<T>(this IServiceCollection<T> services, Action a)"
        ),
        Some("IServiceCollection")
    );
}

#[test]
fn csharp_extension_target_returns_none_for_regular_method() {
    assert_eq!(
        super::csharp_extension_target("int Add(int a, int b)"),
        None
    );
}

#[test]
fn csharp_extension_target_returns_none_for_empty_params() {
    assert_eq!(
        super::csharp_extension_target("void DoWork()"),
        None
    );
}

#[test]
fn csharp_extension_target_handles_extra_whitespace() {
    assert_eq!(
        super::csharp_extension_target("Result<T> Try<T>( this  IObservable<T> source )"),
        Some("IObservable")
    );
}

#[test]
fn csharp_extension_target_only_first_param_counts() {
    // `this` on a non-first parameter is invalid C# but should not match.
    assert_eq!(
        super::csharp_extension_target("void Bind(IDictionary d, this string key)"),
        None
    );
}
