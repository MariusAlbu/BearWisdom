// =============================================================================
// type_checker/core/pattern_tests.rs — Unit tests for destructuring binding.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::SymbolInfo;
use crate::type_checker::core::symbol_types::SymbolTypeData;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena};
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};
use std::sync::Arc;

fn sym(id: i64, name: &str, qname: &str, kind: &str, scope: Option<&str>) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from("x.ts"),
        scope_path: scope.map(|s| s.to_string()),
        package_id: None,
        signature: None,
    }
}

#[test]
fn identifier_binds_value_unchanged() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let symbol_types = SymbolTypeMap::new();

    let out = bind(
        &Pattern::Identifier("u".into()),
        user,
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &DEFAULT_PROFILE,
    );
    assert_eq!(out, vec![("u".to_string(), user)]);
}

#[test]
fn rest_binds_value_unchanged() {
    let mut arena = TypeArena::new();
    let list = arena.class("List");
    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let symbol_types = SymbolTypeMap::new();

    let out = bind(
        &Pattern::Rest("xs".into()),
        list,
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &DEFAULT_PROFILE,
    );
    assert_eq!(out, vec![("xs".to_string(), list)]);
}

#[test]
fn object_destructure_resolves_each_prop_through_member_lookup() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let str_ty = arena.primitive(PrimKind::Str);
    let int_ty = arena.primitive(PrimKind::Int);

    let mut members = MembersIndex::new();
    members.add_direct(user, sym(1, "name", "User.name", "field", Some("User")));
    members.add_direct(user, sym(2, "age", "User.age", "field", Some("User")));

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );
    symbol_types.insert(
        2,
        SymbolTypeData {
            declared_type: Some(int_ty),
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();

    let pattern = Pattern::Object(vec![
        ("name".into(), Pattern::Identifier("n".into())),
        ("age".into(), Pattern::Identifier("a".into())),
    ]);
    let out = bind(
        &pattern,
        user,
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &DEFAULT_PROFILE,
    );
    assert_eq!(
        out,
        vec![("n".to_string(), str_ty), ("a".to_string(), int_ty)]
    );
}

#[test]
fn object_destructure_unknown_prop_falls_back_to_unknown_typeid() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let members = MembersIndex::new(); // empty — no `name` registered
    let supertypes = SupertypeGraph::new();
    let symbol_types = SymbolTypeMap::new();

    let pattern = Pattern::Object(vec![(
        "missing".into(),
        Pattern::Identifier("m".into()),
    )]);
    let out = bind(
        &pattern,
        user,
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &DEFAULT_PROFILE,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "m");
    assert_eq!(arena.get(out[0].1), &Type::Unknown);
}

#[test]
fn array_destructure_peels_iterator_when_profile_allows() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let list = arena.class("List");
    let list_user = arena.intern(Type::Apply {
        base: list,
        args: vec![user],
    });
    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let symbol_types = SymbolTypeMap::new();

    let profile = LanguageProfile {
        iterator_method: Some("next"),
        ..DEFAULT_PROFILE
    };

    let pattern = Pattern::Array(vec![
        Pattern::Identifier("head".into()),
        Pattern::Identifier("second".into()),
    ]);
    let out = bind(
        &pattern,
        list_user,
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &profile,
    );
    assert_eq!(
        out,
        vec![
            ("head".to_string(), user),
            ("second".to_string(), user),
        ]
    );
}

#[test]
fn array_destructure_rest_at_tail_keeps_original_value_type() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let list = arena.class("List");
    let list_user = arena.intern(Type::Apply {
        base: list,
        args: vec![user],
    });

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = LanguageProfile {
        iterator_method: Some("next"),
        ..DEFAULT_PROFILE
    };

    let pattern = Pattern::Array(vec![
        Pattern::Identifier("head".into()),
        Pattern::Rest("rest".into()),
    ]);
    let out = bind(
        &pattern,
        list_user,
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &profile,
    );
    // head -> element (User), rest -> the whole list type.
    assert_eq!(out[0], ("head".to_string(), user));
    assert_eq!(out[1].0, "rest");
    assert_eq!(out[1].1, list_user);
}

#[test]
fn nested_object_in_array_destructures_correctly() {
    // const [{ name }, ...rest] = users;  // users: List<User>
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let str_ty = arena.primitive(PrimKind::Str);
    let list = arena.class("List");
    let list_user = arena.intern(Type::Apply {
        base: list,
        args: vec![user],
    });

    let mut members = MembersIndex::new();
    members.add_direct(user, sym(1, "name", "User.name", "field", Some("User")));
    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );
    let supertypes = SupertypeGraph::new();
    let profile = LanguageProfile {
        iterator_method: Some("next"),
        ..DEFAULT_PROFILE
    };

    let pattern = Pattern::Array(vec![
        Pattern::Object(vec![(
            "name".into(),
            Pattern::Identifier("n".into()),
        )]),
        Pattern::Rest("rest".into()),
    ]);
    let out = bind(
        &pattern,
        list_user,
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &profile,
    );
    assert_eq!(out[0], ("n".to_string(), str_ty));
    assert_eq!(out[1].1, list_user);
}
