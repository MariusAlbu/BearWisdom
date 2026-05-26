// =============================================================================
// type_checker/core/dispatch_tests.rs — Unit tests for method dispatch.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::SymbolInfo;
use crate::type_checker::core::symbol_types::SymbolTypeData;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena};
use crate::type_checker::profile::language_profile::{
    DispatchAxis, LanguageProfile, DEFAULT_PROFILE,
};
use crate::types::{AliasTarget, CallArg};
use std::collections::HashMap;
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
    locals: HashMap<String, String>,
    parents: HashMap<String, String>,
}

impl EmptyLookup {
    fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            locals: HashMap::new(),
            parents: HashMap::new(),
        }
    }

    fn with_local(mut self, name: &str, ty: &str) -> Self {
        self.locals.insert(name.to_string(), ty.to_string());
        self
    }

    fn with_parent(mut self, child: &str, parent: &str) -> Self {
        self.parents.insert(child.to_string(), parent.to_string());
        self
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
    fn local_type(&self, name: &str) -> Option<String> {
        self.locals.get(name).cloned()
    }
    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.parents.get(class_qname).map(|s| s.as_str())
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
fn multi_arg_dispatch_falls_back_to_receiver_when_no_signature_matches() {
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
    // No arg-type match → fall back to receiver dispatch (the sole candidate)
    // so the call still resolves rather than missing.
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
    assert_eq!(result.id, 10);
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

#[test]
fn resolve_arg_types_classifies_literals() {
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let str_ty = arena.primitive(PrimKind::Str);
    let int_ty = arena.primitive(PrimKind::Int);
    let float_ty = arena.primitive(PrimKind::Float);
    let bool_ty = arena.primitive(PrimKind::Bool);
    let unknown = arena.intern(Type::Unknown);

    let args = vec![
        CallArg::StringLit("x".into()),
        CallArg::TemplateLit("/a/{}".into()),
        CallArg::Literal("42".into()),
        CallArg::Literal("3.14".into()),
        CallArg::Literal("true".into()),
        CallArg::Literal("null".into()),
        CallArg::Other,
    ];
    assert_eq!(
        resolve_arg_types(&args, &arena, &lookup),
        vec![str_ty, str_ty, int_ty, float_ty, bool_ty, unknown, unknown]
    );
}

#[test]
fn resolve_arg_types_chases_ident_local_type() {
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("u", "User");
    let user = arena.class("User");
    let unknown = arena.intern(Type::Unknown);

    // Known local → its declared type; unknown local → Unknown.
    assert_eq!(
        resolve_arg_types(
            &[CallArg::Ident("u".into()), CallArg::Ident("mystery".into())],
            &arena,
            &lookup
        ),
        vec![user, unknown]
    );
}

#[test]
fn multi_arg_dispatch_picks_most_specific_overload() {
    // handle(User) and handle(Admin) where Admin <: User. An Admin argument is
    // assignable to both; the most specific (Admin) overload wins.
    let mut arena = TypeArena::new();
    let target = arena.class("Handler");
    let user = arena.class("User");
    let admin = arena.class("Admin");

    let mut members = MembersIndex::new();
    members.add_direct(target, sym(40, "handle", "Handler.handle", "method", Some("Handler")));
    members.add_direct(target, sym(41, "handle", "Handler.handle", "method", Some("Handler")));

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(40, SymbolTypeData { param_types: vec![user], ..Default::default() });
    symbol_types.insert(41, SymbolTypeData { param_types: vec![admin], ..Default::default() });

    let supertypes = SupertypeGraph::new();
    let lookup = EmptyLookup::new().with_parent("Admin", "User");
    let profile = LanguageProfile {
        dispatch_axis: DispatchAxis::MultiArg,
        ..DEFAULT_PROFILE
    };

    let query = DispatchQuery {
        method_name: "handle",
        receiver: target,
        arg_types: &[admin],
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
    .expect("most-specific dispatch hits");
    assert_eq!(result.id, 41, "Admin arg should pick the Admin overload, not User");
}
