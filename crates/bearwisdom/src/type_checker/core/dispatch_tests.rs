// =============================================================================
// type_checker/core/dispatch_tests.rs — Unit tests for method dispatch.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::SymbolInfo;
use crate::type_checker::core::symbol_types::SymbolTypeData;
use crate::type_checker::core::types::{PrimKind, TypeArena};
use crate::type_checker::profile::language_profile::{
    DispatchAxis, LanguageProfile, DEFAULT_PROFILE,
};
use crate::types::AliasTarget;
use std::sync::Arc;

fn sym(id: i64, name: &str, qname: &str, kind: &str, scope: Option<&str>) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from("x.rs"),
        scope_path: scope.map(|s| s.to_string()),
        package_id: None,
        signature: None,
    }
}

struct EmptyLookup {
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl EmptyLookup {
    fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }
}

impl SymbolLookup for EmptyLookup {
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
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

#[test]
fn receiver_dispatch_delegates_to_members_lookup() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let mut members = MembersIndex::new();
    members.add_direct(user, sym(1, "greet", "User.greet", "method", Some("User")));
    let supertypes = SupertypeGraph::new();
    let symbol_types = SymbolTypeMap::new();
    let lookup = EmptyLookup::new();

    let query = DispatchQuery {
        method_name: "greet",
        receiver: user,
        arg_types: &[],
        expected_return: None,
        kind_filter: EdgeKind::Calls,
    };
    let result = select_method(
        &query,
        &members,
        &supertypes,
        &symbol_types,
        &arena,
        &DEFAULT_PROFILE,
        &lookup,
    )
    .expect("receiver dispatch hits");
    assert_eq!(result.id, 1);
}

#[test]
fn multi_arg_dispatch_picks_signature_matching_arg_types() {
    // Two methods named `compare`: one takes (Int, Int), another (Str, Str).
    // Call with (Int, Int) picks the first.
    let mut arena = TypeArena::new();
    let target = arena.class("Comparator");
    let int_ty = arena.primitive(PrimKind::Int);
    let str_ty = arena.primitive(PrimKind::Str);

    let mut members = MembersIndex::new();
    members.add_direct(
        target,
        sym(10, "compare", "Comparator.compare", "method", Some("Comparator")),
    );
    members.add_direct(
        target,
        sym(11, "compare", "Comparator.compare", "method", Some("Comparator")),
    );

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        10,
        SymbolTypeData {
            param_types: vec![int_ty, int_ty],
            ..Default::default()
        },
    );
    symbol_types.insert(
        11,
        SymbolTypeData {
            param_types: vec![str_ty, str_ty],
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();
    let lookup = EmptyLookup::new();
    let profile = LanguageProfile {
        dispatch_axis: DispatchAxis::MultiArg,
        ..DEFAULT_PROFILE
    };

    let query = DispatchQuery {
        method_name: "compare",
        receiver: target,
        arg_types: &[int_ty, int_ty],
        expected_return: None,
        kind_filter: EdgeKind::Calls,
    };
    let result = select_method(
        &query,
        &members,
        &supertypes,
        &symbol_types,
        &arena,
        &profile,
        &lookup,
    )
    .expect("multi-arg dispatch picks Int signature");
    assert_eq!(result.id, 10);

    // Call with (Str, Str) picks the second.
    let query = DispatchQuery {
        method_name: "compare",
        receiver: target,
        arg_types: &[str_ty, str_ty],
        expected_return: None,
        kind_filter: EdgeKind::Calls,
    };
    let result = select_method(
        &query,
        &members,
        &supertypes,
        &symbol_types,
        &arena,
        &profile,
        &lookup,
    )
    .expect("multi-arg dispatch picks Str signature");
    assert_eq!(result.id, 11);
}

#[test]
fn multi_arg_dispatch_misses_when_no_signature_matches() {
    let mut arena = TypeArena::new();
    let target = arena.class("Comparator");
    let int_ty = arena.primitive(PrimKind::Int);
    let str_ty = arena.primitive(PrimKind::Str);

    let mut members = MembersIndex::new();
    members.add_direct(
        target,
        sym(10, "compare", "Comparator.compare", "method", Some("Comparator")),
    );

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        10,
        SymbolTypeData {
            param_types: vec![int_ty, int_ty],
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();
    let lookup = EmptyLookup::new();
    let profile = LanguageProfile {
        dispatch_axis: DispatchAxis::MultiArg,
        ..DEFAULT_PROFILE
    };

    let query = DispatchQuery {
        method_name: "compare",
        receiver: target,
        // String args against the only candidate's Int params → No.
        arg_types: &[str_ty, str_ty],
        expected_return: None,
        kind_filter: EdgeKind::Calls,
    };
    assert!(select_method(
        &query,
        &members,
        &supertypes,
        &symbol_types,
        &arena,
        &profile,
        &lookup,
    )
    .is_none());
}

#[test]
fn return_type_dispatch_picks_candidate_with_matching_return() {
    let mut arena = TypeArena::new();
    let cls = arena.class("Convert");
    let int_ty = arena.primitive(PrimKind::Int);
    let str_ty = arena.primitive(PrimKind::Str);

    let mut members = MembersIndex::new();
    members.add_direct(cls, sym(20, "from", "Convert.from", "method", Some("Convert")));
    members.add_direct(cls, sym(21, "from", "Convert.from", "method", Some("Convert")));

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        20,
        SymbolTypeData {
            return_type: Some(int_ty),
            ..Default::default()
        },
    );
    symbol_types.insert(
        21,
        SymbolTypeData {
            return_type: Some(str_ty),
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();
    let lookup = EmptyLookup::new();
    let profile = LanguageProfile {
        dispatch_axis: DispatchAxis::ReturnType,
        ..DEFAULT_PROFILE
    };

    let query = DispatchQuery {
        method_name: "from",
        receiver: cls,
        arg_types: &[],
        expected_return: Some(str_ty),
        kind_filter: EdgeKind::Calls,
    };
    let result = select_method(
        &query,
        &members,
        &supertypes,
        &symbol_types,
        &arena,
        &profile,
        &lookup,
    )
    .expect("return-type dispatch hits Str variant");
    assert_eq!(result.id, 21);
}

#[test]
fn return_type_dispatch_falls_back_to_receiver_when_no_expected_return() {
    let mut arena = TypeArena::new();
    let cls = arena.class("Convert");
    let int_ty = arena.primitive(PrimKind::Int);

    let mut members = MembersIndex::new();
    members.add_direct(cls, sym(20, "from", "Convert.from", "method", Some("Convert")));
    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        20,
        SymbolTypeData {
            return_type: Some(int_ty),
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();
    let lookup = EmptyLookup::new();
    let profile = LanguageProfile {
        dispatch_axis: DispatchAxis::ReturnType,
        ..DEFAULT_PROFILE
    };

    let query = DispatchQuery {
        method_name: "from",
        receiver: cls,
        arg_types: &[],
        expected_return: None, // no signal — fall back
        kind_filter: EdgeKind::Calls,
    };
    let result = select_method(
        &query,
        &members,
        &supertypes,
        &symbol_types,
        &arena,
        &profile,
        &lookup,
    )
    .expect("falls back to receiver dispatch");
    assert_eq!(result.id, 20);
}
