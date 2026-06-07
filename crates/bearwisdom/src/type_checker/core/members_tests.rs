// =============================================================================
// type_checker/core/members_tests.rs — Unit tests for MembersIndex.
// =============================================================================

use super::*;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeData;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{AliasTarget, EdgeKind};
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
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(2), None)
        .expect("hit");
    assert_eq!(two.0.id, 2, "two args should pick the two-param overload");

    let one = index
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(1), None)
        .expect("hit");
    assert_eq!(one.0.id, 1, "one arg should pick the one-param overload");

    // Unknown arity, and an arity that matches no overload, both fall back to
    // the first declared — the pre-arity behavior.
    let unknown = index
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, None, None)
        .expect("hit");
    assert_eq!(unknown.0.id, 1);
    let no_match = index
        .lookup_with_binding(svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(3), None)
        .expect("hit");
    assert_eq!(no_match.0.id, 1);
}

/// Minimal `SymbolLookup` for `ArgTypes` — never consulted when the argument
/// and parameter types are primitives (disjointness decides without an
/// inheritance walk).
struct NullLookup {
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl NullLookup {
    fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }
}

impl SymbolLookup for NullLookup {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
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
    fn alias_target(&self, _: &str) -> Option<&AliasTarget> {
        None
    }
    fn parent_class_qname(&self, _: &str) -> Option<&str> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

#[test]
fn overload_selected_by_arg_type() {
    let mut arena = TypeArena::new();
    let svc = arena.class("Svc");
    let int_ty = arena.primitive(PrimKind::Int);
    let str_ty = arena.primitive(PrimKind::Str);
    let mut index = MembersIndex::new();
    // Two same-arity `process` overloads, distinguished only by parameter type.
    index.add_direct(svc, sym_sig(1, "process", "Svc.process", "method", Some("Svc"), "process(x)"));
    index.add_direct(svc, sym_sig(2, "process", "Svc.process", "method", Some("Svc"), "process(x)"));
    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(1, SymbolTypeData { param_types: vec![int_ty], ..Default::default() });
    symbol_types.insert(2, SymbolTypeData { param_types: vec![str_ty], ..Default::default() });
    let graph = empty_supertypes();
    let lookup = NullLookup::new();

    // A string argument picks the Str overload (id 2), not the first-declared.
    let str_args = [str_ty];
    let by_str = index
        .lookup_with_binding(
            svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(1),
            Some(ArgTypes { arg_types: &str_args, symbol_types: &symbol_types, lookup: &lookup }),
        )
        .expect("hit");
    assert_eq!(by_str.0.id, 2, "string arg should pick the Str overload");

    // An int argument picks the Int overload (id 1).
    let int_args = [int_ty];
    let by_int = index
        .lookup_with_binding(
            svc, "process", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE, Some(1),
            Some(ArgTypes { arg_types: &int_args, symbol_types: &symbol_types, lookup: &lookup }),
        )
        .expect("hit");
    assert_eq!(by_int.0.id, 1, "int arg should pick the Int overload");
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
fn scope_less_extension_keyed_under_receiver_for_opted_in_language() {
    // A Kotlin top-level extension (`fun String.shout()`) has no scope_path —
    // the receiver is folded into the signature as a leading `this String`
    // parameter. It must register as an extension member of `String` even
    // though it owns no enclosing type, so a `s.shout()` chain resolves.
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

    let ext = ExtractedSymbol {
        name: "shout".to_string(),
        qualified_name: "shout".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: Some("fun shout(this String): String".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    };
    let pf = ParsedFile {
        path: "Ext.kt".to_string(),
        language: "kotlin".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![ext],
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
    sym_ids.insert(("Ext.kt".to_string(), 0), 11);

    let arena = TypeArena::new();
    let index = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &arena,
    );

    let string_ty = arena.class("String");
    assert!(
        index.extensions_of(string_ty).iter().any(|m| m.id == 11),
        "scope-less Kotlin extension must register against its receiver type"
    );
}

#[test]
fn scope_less_extension_ignored_for_non_opted_in_language() {
    // The same scope-less `this`-signature shape in a language that does NOT
    // use the convention (Rust) must not register as an extension — the gate is
    // per-language.
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

    let ext = ExtractedSymbol {
        name: "shout".to_string(),
        qualified_name: "shout".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: Some("fn shout(this String): String".to_string()),
        doc_comment: None,
        scope_path: None,
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
        symbols: vec![ext],
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
    sym_ids.insert(("lib.rs".to_string(), 0), 12);

    let arena = TypeArena::new();
    let index = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &arena,
    );

    let string_ty = arena.class("String");
    assert!(
        index.extensions_of(string_ty).is_empty(),
        "non-opted-in language must not register a `this`-signature as an extension"
    );
}

/// A `ParsedFile` carrying the given symbols and refs, defaulting the rest.
fn parsed(path: &str, language: &str, symbols: Vec<crate::types::ExtractedSymbol>, refs: Vec<crate::types::ExtractedRef>) -> crate::types::ParsedFile {
    crate::types::ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
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
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn ex_sym(name: &str, qname: &str, kind: crate::types::SymbolKind, scope: Option<&str>) -> crate::types::ExtractedSymbol {
    crate::types::ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(crate::types::Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope.map(|s| s.to_string()),
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn ex_ref(source_idx: usize, target: &str, kind: EdgeKind) -> crate::types::ExtractedRef {
    crate::types::ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

#[test]
fn external_trait_default_method_is_reachable_from_implementing_type() {
    // A project type `Dog` implements an EXTERNAL trait `Greet` that declares a
    // default method `hello` with a body. The trait + its default method live in
    // an `ext:` ParsedFile (the dep crate walked as plain Rust source). The
    // C->Greet supertype edge forms via the impl-container reroute, but until the
    // ext: skip is kind-gated, `hello`'s body symbol is dropped at build time so
    // the chain walk finds no member. After the gate it is reachable.
    use crate::types::SymbolKind;

    // ext: dep file — Greet trait (idx 0) + its default method hello (idx 1).
    let ext_file = parsed(
        "ext:rust:greet/lib.rs",
        "rust",
        vec![
            ex_sym("Greet", "Greet", SymbolKind::Trait, None),
            ex_sym("hello", "Greet.hello", SymbolKind::Function, Some("Greet")),
        ],
        Vec::new(),
    );

    // Internal file — impl-container Namespace (idx 0) implementing Greet for Dog,
    // plus the Dog struct (idx 1). The container emits an Implements ref to Greet
    // and a self-TypeRef naming the implementing type Dog (mirrors extract_impl).
    let app_file = parsed(
        "app.rs",
        "rust",
        vec![
            ex_sym("<impl Dog@1>", "<impl Dog@1>", SymbolKind::Namespace, None),
            ex_sym("Dog", "Dog", SymbolKind::Struct, None),
        ],
        vec![
            ex_ref(0, "Greet", EdgeKind::Implements),
            ex_ref(0, "Dog", EdgeKind::TypeRef),
        ],
    );

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("ext:rust:greet/lib.rs".to_string(), 0), 100); // Greet
    sym_ids.insert(("ext:rust:greet/lib.rs".to_string(), 1), 101); // hello
    sym_ids.insert(("app.rs".to_string(), 0), 200); // impl container
    sym_ids.insert(("app.rs".to_string(), 1), 201); // Dog

    let arena = TypeArena::new();
    let slice = vec![ext_file, app_file];
    let members = MembersIndex::build_from_parsed_files(&slice, &sym_ids, &arena);

    let lookup = NullLookup::new();
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(&slice, &arena, &DEFAULT_PROFILE, &members, &symbol_types, &lookup);

    let dog = arena.class("Dog");
    let found = members
        .lookup(dog, "hello", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("external trait default method must be reachable from the implementing type");
    assert_eq!(found.id, 101, "resolves to the trait's default-method body symbol");
}

#[test]
fn external_supertrait_default_method_is_reachable_transitively() {
    // Sub-case (b) of fork #4 — transitive ext:→ext: supertrait chains. An
    // external trait `Derived: Base` extends another external trait `Base`. Base
    // declares a default `base_method`; Derived declares a default
    // `derived_method`. A project type `Dog` implements `Derived` only. The
    // supertrait edge `Derived → Base` forms in build_explicit from Derived's own
    // `Inherits` ref (source is the Trait symbol, NOT an impl-container, so the
    // reroute does not fire), and `Dog → Derived` forms via the impl-container
    // reroute. Both Base's and Derived's default bodies live in ext: files and are
    // admitted by the per-symbol trait gate. The chain walk
    // `Dog → Derived → Base` must reach BOTH defaults — the inner hop is the
    // transitive ext:→ext: case the single-hop fork left unexercised.
    use crate::types::SymbolKind;

    // ext: dep crate A — Base trait (idx 0) + default base_method (idx 1).
    let ext_base = parsed(
        "ext:rust:base-crate/lib.rs",
        "rust",
        vec![
            ex_sym("Base", "Base", SymbolKind::Trait, None),
            ex_sym("base_method", "Base.base_method", SymbolKind::Function, Some("Base")),
        ],
        Vec::new(),
    );
    // ext: dep crate B (a DIFFERENT file) — Derived trait (idx 0) + default
    // derived_method (idx 1); Derived inherits Base from crate A. The per-file
    // trait gate must admit each crate's defaults independently, and the
    // cross-file supertrait `Inherits` edge must still form so the walk spans
    // both ext: files.
    let ext_derived = parsed(
        "ext:rust:derived-crate/lib.rs",
        "rust",
        vec![
            ex_sym("Derived", "Derived", SymbolKind::Trait, None),
            ex_sym("derived_method", "Derived.derived_method", SymbolKind::Function, Some("Derived")),
        ],
        vec![ex_ref(0, "Base", EdgeKind::Inherits)],
    );

    // Internal file — impl-container Namespace (idx 0) implementing Derived for
    // Dog, plus the Dog struct (idx 1).
    let app_file = parsed(
        "app.rs",
        "rust",
        vec![
            ex_sym("<impl Dog@1>", "<impl Dog@1>", SymbolKind::Namespace, None),
            ex_sym("Dog", "Dog", SymbolKind::Struct, None),
        ],
        vec![
            ex_ref(0, "Derived", EdgeKind::Implements),
            ex_ref(0, "Dog", EdgeKind::TypeRef),
        ],
    );

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("ext:rust:base-crate/lib.rs".to_string(), 0), 100); // Base
    sym_ids.insert(("ext:rust:base-crate/lib.rs".to_string(), 1), 101); // base_method
    sym_ids.insert(("ext:rust:derived-crate/lib.rs".to_string(), 0), 102); // Derived
    sym_ids.insert(("ext:rust:derived-crate/lib.rs".to_string(), 1), 103); // derived_method
    sym_ids.insert(("app.rs".to_string(), 0), 200); // impl container
    sym_ids.insert(("app.rs".to_string(), 1), 201); // Dog

    let arena = TypeArena::new();
    let slice = vec![ext_base, ext_derived, app_file];
    let members = MembersIndex::build_from_parsed_files(&slice, &sym_ids, &arena);

    let lookup = NullLookup::new();
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(&slice, &arena, &DEFAULT_PROFILE, &members, &symbol_types, &lookup);

    let dog = arena.class("Dog");
    // The directly-implemented trait's default resolves (single hop).
    let derived = members
        .lookup(dog, "derived_method", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("directly-implemented trait's default method must be reachable");
    assert_eq!(derived.id, 103);
    // The SUPERTRAIT's default resolves through the transitive ext:→ext: hop.
    let base = members
        .lookup(dog, "base_method", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("external supertrait default method must be reachable transitively");
    assert_eq!(base.id, 101, "resolves to the supertrait's default-method body symbol");
}

#[test]
fn external_non_trait_member_still_skipped() {
    // The write-storm guard: an external Class `Curl` with a Method `perform`
    // (the common case — a non-trait external type with body methods) must NOT
    // be admitted. Only trait/interface-owned external members ride the relaxed
    // skip; everything else stays lookup-only via SymbolIndex.
    use crate::types::SymbolKind;

    let ext_file = parsed(
        "ext:rust:curl/lib.rs",
        "rust",
        vec![
            ex_sym("Curl", "Curl", SymbolKind::Class, None),
            ex_sym("perform", "Curl.perform", SymbolKind::Method, Some("Curl")),
        ],
        Vec::new(),
    );

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("ext:rust:curl/lib.rs".to_string(), 0), 300);
    sym_ids.insert(("ext:rust:curl/lib.rs".to_string(), 1), 301);

    let arena = TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&ext_file),
        &sym_ids,
        &arena,
    );

    let curl = arena.class("Curl");
    assert!(
        members.direct_of(curl).is_empty(),
        "non-trait external member must stay skipped (write-storm guard)"
    );
}

/// A `SymbolLookup` whose `types_by_name` returns a fixed candidate pool, so a
/// test can drive `resolve_target_qname`'s ambiguity tightening end-to-end
/// through `SupertypeGraph::build`. Everything else mirrors `NullLookup`.
struct TypePoolLookup {
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
    by_short: FxHashMap<String, Vec<SymbolInfo>>,
}

impl TypePoolLookup {
    fn new(pool: Vec<SymbolInfo>) -> Self {
        let mut by_short: FxHashMap<String, Vec<SymbolInfo>> = FxHashMap::default();
        for s in pool {
            by_short.entry(s.name.clone()).or_default().push(s);
        }
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            by_short,
        }
    }
}

impl SymbolLookup for TypePoolLookup {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_short.get(name).map(|v| v.as_slice()).unwrap_or(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
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
    fn alias_target(&self, _: &str) -> Option<&AliasTarget> {
        None
    }
    fn parent_class_qname(&self, _: &str) -> Option<&str> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

#[test]
fn external_trait_default_method_reachable_when_short_name_collides_with_struct() {
    // Sub-case (a) of fork #4 — qname alignment. The external trait's qname is
    // MODULE-PREFIXED (`mycrate.Greet`) and its default-method body keys under
    // that prefixed scope. The impl site writes the bare name `Greet`. A decoy
    // struct also named `Greet` lives in the type pool, so the impl-site name
    // resolves to TWO type-like candidates. Without kind-aware tightening
    // `resolve_target_qname` falls back to the bare `Greet`, the supertype edge
    // keys `class("Greet")`, and the walk misses the member keyed under
    // `class("mycrate.Greet")`. The Implements edge wants a Trait/Interface
    // parent, so the unique trait among the pool must win.
    use crate::types::SymbolKind;

    let ext_file = parsed(
        "ext:rust:mycrate/lib.rs",
        "rust",
        vec![
            ex_sym("Greet", "mycrate.Greet", SymbolKind::Trait, None),
            ex_sym("hello", "mycrate.Greet.hello", SymbolKind::Function, Some("mycrate.Greet")),
        ],
        Vec::new(),
    );

    let app_file = parsed(
        "app.rs",
        "rust",
        vec![
            ex_sym("<impl Dog@1>", "<impl Dog@1>", SymbolKind::Namespace, None),
            ex_sym("Dog", "Dog", SymbolKind::Struct, None),
        ],
        vec![
            ex_ref(0, "Greet", EdgeKind::Implements),
            ex_ref(0, "Dog", EdgeKind::TypeRef),
        ],
    );

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("ext:rust:mycrate/lib.rs".to_string(), 0), 100); // Greet trait
    sym_ids.insert(("ext:rust:mycrate/lib.rs".to_string(), 1), 101); // hello
    sym_ids.insert(("app.rs".to_string(), 0), 200); // impl container
    sym_ids.insert(("app.rs".to_string(), 1), 201); // Dog

    let arena = TypeArena::new();
    let slice = vec![ext_file, app_file];
    let members = MembersIndex::build_from_parsed_files(&slice, &sym_ids, &arena);

    // The type pool has the prefixed trait AND a same-short-named decoy struct,
    // so the bare impl-site name `Greet` is ambiguous by short name.
    let lookup = TypePoolLookup::new(vec![
        sym(100, "Greet", "mycrate.Greet", "trait", None),
        sym(999, "Greet", "other.Greet", "struct", None),
    ]);
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(&slice, &arena, &DEFAULT_PROFILE, &members, &symbol_types, &lookup);

    let dog = arena.class("Dog");
    let found = members
        .lookup(dog, "hello", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("Implements edge must resolve to the unique trait despite the same-named struct");
    assert_eq!(found.id, 101, "resolves to the trait's default-method body symbol");
}

#[test]
fn external_trait_short_name_collides_with_another_trait_declines() {
    // Soundness boundary for sub-case (a): when the bare impl-site name maps to
    // TWO same-short-named TRAITS (two deps each declaring a `Greet` trait), the
    // kind preference can't disambiguate which one the impl meant — that needs
    // import-scope (BIND-2), not member keying. The edge falls back to the bare
    // name, so the member keyed under the prefixed trait qname is NOT reached:
    // a decline to None, never a coincidental bind to the wrong trait's default.
    use crate::types::SymbolKind;

    let ext_file = parsed(
        "ext:rust:mycrate/lib.rs",
        "rust",
        vec![
            ex_sym("Greet", "mycrate.Greet", SymbolKind::Trait, None),
            ex_sym("hello", "mycrate.Greet.hello", SymbolKind::Function, Some("mycrate.Greet")),
        ],
        Vec::new(),
    );

    let app_file = parsed(
        "app.rs",
        "rust",
        vec![
            ex_sym("<impl Dog@1>", "<impl Dog@1>", SymbolKind::Namespace, None),
            ex_sym("Dog", "Dog", SymbolKind::Struct, None),
        ],
        vec![
            ex_ref(0, "Greet", EdgeKind::Implements),
            ex_ref(0, "Dog", EdgeKind::TypeRef),
        ],
    );

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("ext:rust:mycrate/lib.rs".to_string(), 0), 100);
    sym_ids.insert(("ext:rust:mycrate/lib.rs".to_string(), 1), 101);
    sym_ids.insert(("app.rs".to_string(), 0), 200);
    sym_ids.insert(("app.rs".to_string(), 1), 201);

    let arena = TypeArena::new();
    let slice = vec![ext_file, app_file];
    let members = MembersIndex::build_from_parsed_files(&slice, &sym_ids, &arena);

    // Two distinct traits share the short name `Greet` — irreducibly ambiguous.
    let lookup = TypePoolLookup::new(vec![
        sym(100, "Greet", "mycrate.Greet", "trait", None),
        sym(998, "Greet", "other.Greet", "trait", None),
    ]);
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(&slice, &arena, &DEFAULT_PROFILE, &members, &symbol_types, &lookup);

    let dog = arena.class("Dog");
    assert!(
        members
            .lookup(dog, "hello", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
            .is_none(),
        "two same-named traits are ambiguous — decline, do not bind a wrong trait's default"
    );
}

#[test]
fn this_extension_target_recognises_simple_signature() {
    assert_eq!(
        super::this_extension_target("string MyExt(this string s, int x)"),
        Some("string")
    );
}

#[test]
fn this_extension_target_handles_generic_receiver() {
    assert_eq!(
        super::this_extension_target(
            "T MyExtAsync<T>(this IServiceCollection<T> services, Action a)"
        ),
        Some("IServiceCollection")
    );
}

#[test]
fn this_extension_target_returns_none_for_regular_method() {
    assert_eq!(
        super::this_extension_target("int Add(int a, int b)"),
        None
    );
}

#[test]
fn this_extension_target_returns_none_for_empty_params() {
    assert_eq!(
        super::this_extension_target("void DoWork()"),
        None
    );
}

#[test]
fn this_extension_target_handles_extra_whitespace() {
    assert_eq!(
        super::this_extension_target("Result<T> Try<T>( this  IObservable<T> source )"),
        Some("IObservable")
    );
}

#[test]
fn this_extension_target_only_first_param_counts() {
    // `this` on a non-first parameter is invalid C# but should not match.
    assert_eq!(
        super::this_extension_target("void Bind(IDictionary d, this string key)"),
        None
    );
}

#[test]
fn project_struct_structurally_satisfies_external_interface() {
    // EXT-2 interface side, end to end: a project struct that carries an external
    // interface's shape (Go `io.Reader`) gains the structural supertype edge. The
    // external interface's members are admitted by the Trait/Interface admission
    // gate, so `build_structural` enumerates the interface shape and INFER-5's
    // sound check links the project struct. Distinct from the Implements-reroute
    // path: here there is NO nominal edge, only structural satisfaction.
    use crate::types::SymbolKind;

    // ext: dep — Reader interface (idx 0) + its Read method (idx 1).
    let ext_file = parsed(
        "ext:go:io/io.go",
        "go",
        vec![
            ex_sym("Reader", "io.Reader", SymbolKind::Interface, None),
            ex_sym("Read", "io.Reader.Read", SymbolKind::Method, Some("io.Reader")),
        ],
        Vec::new(),
    );
    // internal — MyReader struct (idx 0) + a Read method (idx 1) with the same shape.
    let app_file = parsed(
        "app.go",
        "go",
        vec![
            ex_sym("MyReader", "MyReader", SymbolKind::Struct, None),
            ex_sym("Read", "MyReader.Read", SymbolKind::Method, Some("MyReader")),
        ],
        Vec::new(),
    );

    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("ext:go:io/io.go".to_string(), 0), 100); // io.Reader
    sym_ids.insert(("ext:go:io/io.go".to_string(), 1), 101); // io.Reader.Read
    sym_ids.insert(("app.go".to_string(), 0), 200); // MyReader
    sym_ids.insert(("app.go".to_string(), 1), 201); // MyReader.Read

    let arena = TypeArena::new();
    let slice = vec![ext_file, app_file];
    let members = MembersIndex::build_from_parsed_files(&slice, &sym_ids, &arena);

    // Matching `(ByteSlice) -> Int` shapes so INFER-5's sound check accepts.
    let byte_slice = arena.class("ByteSlice");
    let int_ty = arena.primitive(PrimKind::Int);
    let mut symbol_types = SymbolTypeMap::new();
    let method_shape = || SymbolTypeData {
        declared_type: None,
        return_type: Some(int_ty),
        param_types: vec![byte_slice],
        generic_params: Vec::new(),
    };
    symbol_types.insert(101, method_shape());
    symbol_types.insert(201, method_shape());

    let lookup = NullLookup::new();
    let profile = crate::type_checker::profile::language_profile::LanguageProfile {
        supertype_discovery:
            crate::type_checker::profile::language_profile::SupertypeDiscovery::Structural,
        ..DEFAULT_PROFILE
    };
    let graph = SupertypeGraph::build(&slice, &arena, &profile, &members, &symbol_types, &lookup);

    let my_reader = arena.class("MyReader");
    let reader = arena.class("io.Reader");
    assert!(
        graph.parents_of(my_reader).contains(&reader),
        "MyReader structurally satisfies the external io.Reader interface"
    );
}
