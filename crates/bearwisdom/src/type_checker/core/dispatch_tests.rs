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
    field_types: HashMap<String, String>,
}

impl EmptyLookup {
    fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            locals: HashMap::new(),
            parents: HashMap::new(),
            field_types: HashMap::new(),
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

    fn with_field_type(mut self, qname: &str, ty: &str) -> Self {
        self.field_types.insert(qname.to_string(), ty.to_string());
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
    fn field_type_name(&self, qname: &str) -> Option<&str> {
        self.field_types.get(qname).map(|s| s.as_str())
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
        sym(
            10,
            "compare",
            "Comparator.compare",
            "method",
            Some("Comparator"),
        ),
    );
    members.add_direct(
        target,
        sym(
            11,
            "compare",
            "Comparator.compare",
            "method",
            Some("Comparator"),
        ),
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
        sym(
            10,
            "compare",
            "Comparator.compare",
            "method",
            Some("Comparator"),
        ),
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
    members.add_direct(
        cls,
        sym(20, "from", "Convert.from", "method", Some("Convert")),
    );
    members.add_direct(
        cls,
        sym(21, "from", "Convert.from", "method", Some("Convert")),
    );

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
    members.add_direct(
        cls,
        sym(20, "from", "Convert.from", "method", Some("Convert")),
    );
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
        resolve_arg_types(&args, &arena, &lookup, &DEFAULT_PROFILE),
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
            &lookup,
            &DEFAULT_PROFILE
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
    members.add_direct(
        target,
        sym(40, "handle", "Handler.handle", "method", Some("Handler")),
    );
    members.add_direct(
        target,
        sym(41, "handle", "Handler.handle", "method", Some("Handler")),
    );

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        40,
        SymbolTypeData {
            param_types: vec![user],
            ..Default::default()
        },
    );
    symbol_types.insert(
        41,
        SymbolTypeData {
            param_types: vec![admin],
            ..Default::default()
        },
    );

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
    assert_eq!(
        result.id, 41,
        "Admin arg should pick the Admin overload, not User"
    );
}

// =============================================================================
// Conservative typing arms for structured CallArg variants (INFER-4 slice 1c).
//
// Each arm types a structured expression by recursing `resolve_arg_type`, and
// every arm MUST yield Unknown when it cannot soundly determine the type. The
// negative tests are the soundness proof: a divergent ternary, a heterogeneous
// array, a mixed `+`, a class subscripted by a dynamic index, and an `await`
// over a non-async value all stay Unknown rather than guessing.
// =============================================================================

use crate::languages::typescript::profile::TYPESCRIPT_PROFILE;

/// Extract the first non-empty `call_args` list from a TS snippet. Mirrors the
/// extractor's real output so the resolve-side tests run on genuine
/// extract → resolve flow rather than hand-built CallArg trees.
fn ts_call_args(src: &str) -> Vec<CallArg> {
    let result = crate::languages::typescript::extract::extract(src, false);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

fn ts_resolve(args: &[CallArg], arena: &TypeArena, lookup: &EmptyLookup) -> Vec<TypeId> {
    resolve_arg_types(args, arena, lookup, &TYPESCRIPT_PROFILE)
}

// --- Ternary --------------------------------------------------------------

#[test]
fn ternary_same_string_branches_types_str() {
    // `true ? "a" : "b"` — both branches are strings → Str.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c() { f(true ? "a" : "b"); }"#);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected a Ternary arg, got {args:?}"
    );
    let str_ty = arena.primitive(PrimKind::Str);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![str_ty]);
}

#[test]
fn ternary_same_class_branches_types_that_class() {
    // `cond ? a : b` where both `a` and `b` are `User` → User.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new()
        .with_local("a", "User")
        .with_local("b", "User");
    let args = ts_call_args(r#"function c(cond, a, b) { f(cond ? a : b); }"#);
    let user = arena.class("User");
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![user]);
}

#[test]
fn ternary_divergent_branches_types_unknown() {
    // `cond ? 1 : "x"` — Int vs Str → Unknown (do NOT pick a branch).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c(cond) { f(cond ? 1 : "x"); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn ternary_one_branch_unknown_types_unknown() {
    // `cond ? a : "x"` where `a` is untyped → one branch Unknown → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c(cond, a) { f(cond ? a : "x"); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

// --- ArrayLiteral ---------------------------------------------------------

#[test]
fn array_literal_homogeneous_class_types_array_of_that_class() {
    // `[u1, u2]` where both are `User` → Apply<Array, [User]>.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new()
        .with_local("u1", "User")
        .with_local("u2", "User");
    let args = ts_call_args(r#"function c(u1, u2) { f([u1, u2]); }"#);
    let user = arena.class("User");
    let arr = arena.intern(Type::Apply {
        base: arena.class("Array"),
        args: vec![user],
    });
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![arr]);
}

#[test]
fn array_literal_heterogeneous_types_unknown() {
    // `[1, "x"]` — Int and Str → Unknown (do NOT guess an element type).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c() { f([1, "x"]); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn array_literal_with_unknown_element_types_unknown() {
    // `[u1, mystery]` — one element untyped → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("u1", "User");
    let args = ts_call_args(r#"function c(u1, mystery) { f([u1, mystery]); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn array_literal_empty_types_unknown() {
    // `[]` — no element to type → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(
        ts_resolve(
            &[CallArg::ArrayLiteral { elements: vec![] }],
            &arena,
            &lookup
        ),
        vec![unknown]
    );
}

// --- Await ----------------------------------------------------------------

// The TS extractor's `await_expression` arm reads the awaited node via the
// `value` field, which this grammar version doesn't expose, so `await x`
// currently extracts as `Await { expr: Other }` (an extraction-side gap owned
// by the extract slice). These resolve-side tests feed the structured
// `Await { expr: Ident(_) }` directly so they exercise the typing arm itself.
fn await_arg(inner: CallArg) -> CallArg {
    CallArg::Await {
        expr: Box::new(inner),
    }
}

#[test]
fn await_promise_local_unwraps_inner() {
    // `await p` where `p: Promise<User>` → User (Promise is a TS async wrapper).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("p", "Promise<User>");
    let user = arena.class("User");
    let args = vec![await_arg(CallArg::Ident("p".into()))];
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![user]);
}

#[test]
fn await_non_promise_local_types_unknown() {
    // `await n` where `n: Box<User>` — Box is NOT an async wrapper → Unknown
    // (NOT the wrapped Box<User>, NOT the inner User).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("n", "Box<User>");
    let unknown = arena.intern(Type::Unknown);
    let args = vec![await_arg(CallArg::Ident("n".into()))];
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn await_untyped_local_types_unknown() {
    // `await x` where `x` is untyped → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let unknown = arena.intern(Type::Unknown);
    let args = vec![await_arg(CallArg::Ident("x".into()))];
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn await_structural_async_wrapper_unwraps_inner() {
    // A structural `AsyncWrapper(User)` (engine-canonical async shape) → User,
    // via `unwrap_await`. Covers the wrapper form that isn't a nominal Apply.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let user = arena.class("User");
    let wrapped = arena.intern(Type::AsyncWrapper(user));
    // Drive the arm through a local typed as the wrapper. `intern_type_str`
    // can't mint an AsyncWrapper from a string, so assert unwrap_async directly.
    assert_eq!(_test_unwrap_async(wrapped, &arena), user);
    let _ = &lookup;
}

// --- Spread ---------------------------------------------------------------

#[test]
fn spread_array_local_contributes_element_type() {
    // `...xs` where `xs: Array<User>` → User (element type to dispatch).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("xs", "Array<User>");
    let args = ts_call_args(r#"function c(xs) { f(...xs); }"#);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Spread { .. })),
        "expected a Spread arg, got {args:?}"
    );
    let user = arena.class("User");
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![user]);
}

#[test]
fn spread_map_local_types_unknown() {
    // `...m` where `m: Map<string, User>` — a Map spreads to entry tuples, not
    // its first type arg → Unknown (peeling args[0] would be unsound).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("m", "Map<string, User>");
    let args = ts_call_args(r#"function c(m) { f(...m); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn spread_untyped_local_types_unknown() {
    // `...xs` where `xs` is untyped → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c(xs) { f(...xs); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

// --- IndexAccess ----------------------------------------------------------

#[test]
fn index_access_array_yields_element_type() {
    // `arr[0]` where `arr: Array<User>` → User (index value irrelevant).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("arr", "Array<User>");
    let args = ts_call_args(r#"function c(arr) { f(arr[0]); }"#);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected an IndexAccess arg, got {args:?}"
    );
    let user = arena.class("User");
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![user]);
}

#[test]
fn index_access_map_yields_value_type() {
    // `m[k]` where `m: Map<string, User>` → User (the value type V).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("m", "Map<string, User>");
    let args = ts_call_args(r#"function c(m, k) { f(m[k]); }"#);
    let user = arena.class("User");
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![user]);
}

#[test]
fn index_access_class_string_literal_key_yields_field_type() {
    // `user["name"]` where `user: User` and `User.name: string` → Str.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new()
        .with_local("user", "User")
        .with_field_type("User.name", "string");
    let args = ts_call_args(r#"function c(user) { f(user["name"]); }"#);
    // `string` interns as Class("string"); the TS primitive_mapping maps it to
    // Str at compare time, but the resolved arg type is the nominal Class form.
    let expected = arena.intern_type_str("string");
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![expected]);
}

#[test]
fn index_access_class_dynamic_index_types_unknown() {
    // `user[idx]` where `user: User` and `idx` is a non-literal → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new().with_local("user", "User");
    let args = ts_call_args(r#"function c(user, idx) { f(user[idx]); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn index_access_tuple_integer_literal_yields_element() {
    // A tuple `[User, string]` indexed by integer literal 0 → User. Exercised
    // through the type-directed index step against a hand-built Type::Tuple,
    // since `intern_type_str` cannot produce a Tuple from a string local.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let user = arena.class("User");
    let str_ty = arena.intern_type_str("string");
    let tuple = arena.intern(Type::Tuple(vec![user, str_ty]));
    let idx0 = _test_index_into(tuple, &CallArg::Literal("0".into()), &arena, &lookup);
    let idx1 = _test_index_into(tuple, &CallArg::Literal("1".into()), &arena, &lookup);
    assert_eq!(idx0, user, "tuple[0] should be User");
    assert_eq!(idx1, str_ty, "tuple[1] should be string");
}

#[test]
fn index_access_tuple_out_of_range_types_unknown() {
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let user = arena.class("User");
    let tuple = arena.intern(Type::Tuple(vec![user]));
    let unknown = arena.intern(Type::Unknown);
    let oob = _test_index_into(tuple, &CallArg::Literal("5".into()), &arena, &lookup);
    assert_eq!(oob, unknown, "tuple index out of range should be Unknown");
}

#[test]
fn index_access_tuple_non_literal_index_types_unknown() {
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let user = arena.class("User");
    let tuple = arena.intern(Type::Tuple(vec![user]));
    let unknown = arena.intern(Type::Unknown);
    let dyn_idx = _test_index_into(tuple, &CallArg::Ident("i".into()), &arena, &lookup);
    assert_eq!(
        dyn_idx, unknown,
        "tuple with dynamic index should be Unknown"
    );
}

// --- Binary ---------------------------------------------------------------

#[test]
fn binary_comparison_types_bool() {
    // `a > b` with untyped operands → Bool regardless (comparison operator).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c(a, b) { f(a > b); }"#);
    let bool_ty = arena.primitive(PrimKind::Bool);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![bool_ty]);
}

#[test]
fn binary_logical_same_typed_operands_joins_to_operand_type() {
    // `a && b` where both locals are `boolean` → the common operand type.
    // Both operands resolve to Class("boolean") via intern_type_str, which is
    // the same TypeId on both sides, so the join returns that type.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new()
        .with_local("a", "boolean")
        .with_local("b", "boolean");
    let args = ts_call_args(r#"function c(a, b) { f(a && b); }"#);
    // Expected: the interned Class("boolean") — same TypeId as intern_type_str("boolean").
    let expected = arena.intern_type_str("boolean");
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![expected]);
}

#[test]
fn binary_logical_differing_typed_operands_yields_unknown() {
    // `s || n` where `s: string` and `n: number` → Unknown (operand types differ).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new()
        .with_local("s", "string")
        .with_local("n", "number");
    let args = ts_call_args(r#"function c(s, n) { f(s || n); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn binary_logical_untyped_operands_yields_unknown() {
    // `a && b` with untyped locals → operands are Unknown → result Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c(a, b) { f(a && b); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn binary_numeric_addition_types_number() {
    // `1 + 2` — both integer literals → Int (numeric).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c() { f(1 + 2); }"#);
    let int_ty = arena.primitive(PrimKind::Int);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![int_ty]);
}

#[test]
fn binary_string_concat_types_str() {
    // `"a" + "b"` — both strings → Str.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c() { f("a" + "b"); }"#);
    let str_ty = arena.primitive(PrimKind::Str);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![str_ty]);
}

#[test]
fn binary_arithmetic_subtraction_numeric_locals_types_number() {
    // `x - y` where both `x` and `y` are `number` → Float (numeric).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new()
        .with_local("x", "number")
        .with_local("y", "number");
    let args = ts_call_args(r#"function c(x, y) { f(x - y); }"#);
    let float_ty = arena.primitive(PrimKind::Float);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![float_ty]);
}

#[test]
fn binary_plus_mixed_string_number_types_unknown() {
    // `1 + "x"` — Int + Str → Unknown (never assume + is numeric on a mix).
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c() { f(1 + "x"); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn binary_plus_unknown_operands_types_unknown() {
    // `x + y` with untyped operands → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c(x, y) { f(x + y); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn binary_arithmetic_unknown_operand_types_unknown() {
    // `x * 2` where `x` is untyped → one operand non-numeric → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c(x) { f(x * 2); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}

#[test]
fn binary_bitwise_integer_literals_types_int() {
    // `1 & 2` — both integer literals → Int.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c() { f(1 & 2); }"#);
    let int_ty = arena.primitive(PrimKind::Int);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![int_ty]);
}

#[test]
fn binary_bitwise_float_operand_types_unknown() {
    // `1.5 & 2` — a float operand is not an integer → Unknown.
    let arena = TypeArena::new();
    let lookup = EmptyLookup::new();
    let args = ts_call_args(r#"function c() { f(1.5 & 2); }"#);
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(ts_resolve(&args, &arena, &lookup), vec![unknown]);
}
