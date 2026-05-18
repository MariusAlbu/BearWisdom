// =============================================================================
// type_checker/core/chain_tests.rs — Unit + gate tests for ChainWalker.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolInfo};
use crate::type_checker::alias::AliasIndex;
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::{SymbolTypeData, SymbolTypeMap};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{
    AliasTarget, ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain,
    SegmentKind, SymbolKind, Visibility,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Synthetic fixture helpers
// ---------------------------------------------------------------------------

fn seg(name: &str, kind: SegmentKind) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: "test".to_string(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        type_arg_ids: Vec::new(),
    }
}

fn sym_info(id: i64, name: &str, qname: &str, kind: &str, scope: Option<&str>) -> SymbolInfo {
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

fn dummy_extracted_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn dummy_source_symbol(name: &str, scope: Option<&str>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
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

fn file_ctx() -> FileContext {
    FileContext {
        file_path: "test.rs".to_string(),
        language: "rust".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    }
}

struct EmptyLookup {
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
    types: rustc_hash::FxHashMap<String, Vec<SymbolInfo>>,
    locals: rustc_hash::FxHashMap<String, String>,
}

impl EmptyLookup {
    fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            types: Default::default(),
            locals: Default::default(),
        }
    }
    fn with_type(mut self, name: &str, qname: &str) -> Self {
        let info = sym_info(1, name, qname, "class", None);
        self.types.entry(name.to_string()).or_default().push(info);
        self
    }
    fn with_local(mut self, name: &str, qname: &str) -> Self {
        self.locals.insert(name.to_string(), qname.to_string());
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
    fn types_by_name(&self, name: &str) -> &[SymbolInfo] {
        self.types.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn local_type(&self, name: &str) -> Option<String> {
        self.locals.get(name).cloned()
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

// ---------------------------------------------------------------------------
// Unit tests against synthetic fixtures
// ---------------------------------------------------------------------------

#[test]
fn empty_chain_returns_none() {
    let mut arena = TypeArena::new();
    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let symbol_types = SymbolTypeMap::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: Vec::new(),
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("x");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(walker.walk(&chain, &ref_ctx, &fc).is_none());
}

#[test]
fn single_segment_chain_resolves_to_self_yielding_type() {
    // Chain = [User]. User class is registered in SymbolTypeMap with
    // self-yield → resolution targets User's sym id and TypeId equals
    // arena.class("User").
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        42,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 42);

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![seg("User", SegmentKind::TypeAccess)],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("User");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker.walk(&chain, &ref_ctx, &fc).expect("root resolves");
    assert_eq!(result.target_symbol_id, 42);
    assert_eq!(result.resolved_yield_type, user_ty);
}

#[test]
fn two_segment_chain_walks_field_through_class() {
    // User { name: string; }; chain = User.name → method.declared_type
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        100,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 100);
    symbol_types.insert(
        101,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(101, "name", "User.name", "field", Some("User")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_type("User", "User");

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("User", SegmentKind::TypeAccess),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker.walk(&chain, &ref_ctx, &fc).expect("name resolves");
    assert_eq!(result.target_symbol_id, 101);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn three_segment_chain_walks_method_then_field() {
    // class Repo { get(): User; } chain = Repo.get().name
    let mut arena = TypeArena::new();
    let repo_ty = arena.class("Repo");
    let user_ty = arena.class("User");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(repo_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(repo_ty, 1);
    symbol_types.insert(
        2,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 2);
    symbol_types.insert(
        3,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.insert(
        4,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(repo_ty, sym_info(3, "get", "Repo.get", "method", Some("Repo")));
    members.add_direct(user_ty, sym_info(4, "name", "User.name", "field", Some("User")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_type("Repo", "Repo");

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("Repo", SegmentKind::TypeAccess),
            seg("get", SegmentKind::Property),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker.walk(&chain, &ref_ctx, &fc).expect("chain resolves");
    assert_eq!(result.target_symbol_id, 4);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn generic_apply_substitutes_yield_type() {
    // class Repo<T> { first(): T; }
    // const r: Repo<User>; r.first() → yields User (not T).
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let repo_ty = arena.class("Repo");
    let user_ty = arena.class("User");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let generic_t = arena.intern(Type::Generic { param: t_param });
    let apply_ty = arena.intern(Type::Apply {
        base: repo_ty,
        args: vec![user_ty],
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(repo_ty),
            generic_params: vec![t_param],
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(repo_ty, 1);
    symbol_types.insert(
        2,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 2);
    symbol_types.insert(
        3,
        SymbolTypeData {
            return_type: Some(generic_t),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        repo_ty,
        sym_info(3, "first", "Repo.first", "method", Some("Repo")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    // Use a custom root resolver that returns the prebuilt Apply directly.
    struct ApplyRoot {
        ty: TypeId,
    }
    impl RootResolver for ApplyRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &mut TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("r", SegmentKind::Identifier),
            seg("first", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("first");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &ApplyRoot { ty: apply_ty })
        .expect("apply chain resolves");
    assert_eq!(result.target_symbol_id, 3);
    // Generic T must have been substituted to User.
    assert_eq!(result.resolved_yield_type, user_ty);
}

#[test]
fn chain_misses_when_member_not_found() {
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 1);
    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("User", SegmentKind::TypeAccess),
            seg("missing", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("missing");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(walker.walk(&chain, &ref_ctx, &fc).is_none());
}

#[test]
fn self_ref_root_resolves_through_enclosing_scope() {
    // method body: this.name → SelfRef root → enclosing class is User.
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 1);
    symbol_types.insert(
        2,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(2, "name", "User.name", "field", Some("User")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("this", SegmentKind::SelfRef),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("greet", Some("User"));
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: vec!["User".to_string()],
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker.walk(&chain, &ref_ctx, &fc).expect("self-ref resolves");
    assert_eq!(result.target_symbol_id, 2);
    assert_eq!(result.resolved_yield_type, str_ty);
}

// ---------------------------------------------------------------------------
// Gate test — real TS extraction → chain walker → expected target
// ---------------------------------------------------------------------------

#[test]
fn identifier_root_resolves_local_variable_via_lookup() {
    // const u: User = ...;  chain = u.name → `u` is a local of type User.
    // DefaultRootResolver must consult lookup.local_type before treating
    // the identifier as a class.
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 1);
    symbol_types.insert(
        2,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(2, "name", "User.name", "field", Some("User")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_local("u", "User");

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("u", SegmentKind::Identifier),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("local variable's type bridges to its members");
    assert_eq!(result.target_symbol_id, 2);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn identifier_root_local_wins_over_same_named_global_type() {
    // A local `User` of type Admin shadows the global User class. The
    // walker must resolve to Admin's members, not User's.
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let admin_ty = arena.class("Admin");
    let int_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Int);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 1);
    symbol_types.insert(
        2,
        SymbolTypeData {
            return_type: Some(admin_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(admin_ty, 2);
    symbol_types.insert(
        3,
        SymbolTypeData {
            declared_type: Some(int_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        admin_ty,
        sym_info(3, "level", "Admin.level", "field", Some("Admin")),
    );
    // User does NOT have `level`.

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("User", "User")
        .with_local("User", "Admin");

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("User", SegmentKind::Identifier),
            seg("level", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("level");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("local shadowing must route through Admin");
    assert_eq!(result.target_symbol_id, 3);
    assert_eq!(result.resolved_yield_type, int_ty);
}

#[test]
fn chain_expands_alias_before_member_lookup() {
    // type UserAlias = User;  chain = UserAlias.name → walks through the
    // alias to User, then looks up `name` on User.
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let alias_ty = arena.class("UserAlias");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 1);
    symbol_types.insert(
        2,
        SymbolTypeData {
            return_type: Some(alias_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(alias_ty, 2);
    symbol_types.insert(
        3,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(3, "name", "User.name", "field", Some("User")),
    );

    let supertypes = SupertypeGraph::new();
    let mut aliases = AliasIndex::default();
    aliases.insert(
        alias_ty,
        AliasTarget::Application {
            root: "User".to_string(),
            args: Vec::new(),
        },
    );
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("UserAlias", SegmentKind::TypeAccess),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("alias expands and chain resolves");
    assert_eq!(result.target_symbol_id, 3);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn construction_segment_yields_self_for_type_defining_kind() {
    // class Foo { x: string; }  chain = new Foo().x → root is Foo,
    // intermediate Construction segment yields Foo (self), final segment
    // looks up x.
    let mut arena = TypeArena::new();
    let foo_ty = arena.class("Foo");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(foo_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(foo_ty, 1);
    symbol_types.insert(
        2,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(foo_ty, sym_info(2, "x", "Foo.x", "field", Some("Foo")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("Foo", SegmentKind::Construction),
            seg("x", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("x");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("construction segment yields self, x resolves");
    assert_eq!(result.target_symbol_id, 2);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn gate_real_ts_two_segment_chain_walks_field() {
    use crate::languages::typescript::extract;

    let source = r#"
export class User {
    name: string = "";
}
"#;
    let extraction = extract::extract(source, false);
    let pf = crate::types::ParsedFile {
        path: "src/u.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: source.len() as u64,
        line_count: source.lines().count() as u32,
        mtime: None,
        package_id: None,
        symbols: extraction.symbols,
        refs: extraction.refs,
        routes: extraction.routes,
        db_sets: extraction.db_sets,
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(source.to_string()),
        has_errors: extraction.has_errors,
        flow: Default::default(),
        demand_contributions: extraction.demand_contributions,
        alias_targets: extraction.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut sym_ids = crate::type_checker::core::SymbolIdMap::default();
    for (idx, _) in pf.symbols.iter().enumerate() {
        sym_ids.insert((pf.path.clone(), idx), idx as i64 + 1);
    }

    let mut arena = TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );

    // Add a synthetic `declared_type = Str` for the `name` field so the
    // chain walker has somewhere to land. Real Phase 5 TS migration
    // populates this from the extractor; for this Phase 4 gate test we
    // splice it in manually to prove the walker correctly threads the
    // declared_type through SymbolTypeMap.
    let name_idx = pf
        .symbols
        .iter()
        .position(|s| s.name == "name" && matches!(s.kind, SymbolKind::Property | SymbolKind::Field))
        .expect("extractor must emit `name` field/property");
    let name_sym_id = sym_ids[&(pf.path.clone(), name_idx)];

    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);
    let mut symbol_types = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );
    symbol_types.insert(
        name_sym_id,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_type("User", "User");

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("User", SegmentKind::TypeAccess),
            seg("name", SegmentKind::Property),
        ],
    };
    let source_sym = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();

    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("real-extraction User.name must resolve");
    assert_eq!(result.target_symbol_id, name_sym_id);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn gate_real_go_struct_method_chain_walks() {
    // Go: type Repo struct {} ; func (r *Repo) Get() *User { ... }
    //     type User struct { Name string }
    // chain: Repo.Get().Name → method yield User → field Name → string.
    // The gate proves the chain walker works against a non-TS extractor.
    use crate::languages::go::extract;

    let source = r#"
package main

type User struct {
    Name string
}

type Repo struct {}

func (r *Repo) Get() *User {
    return &User{Name: "x"}
}
"#;
    let extraction = extract::extract(source);
    let pf = crate::types::ParsedFile {
        path: "main.go".to_string(),
        language: "go".to_string(),
        content_hash: String::new(),
        size: source.len() as u64,
        line_count: source.lines().count() as u32,
        mtime: None,
        package_id: None,
        symbols: extraction.symbols,
        refs: extraction.refs,
        routes: extraction.routes,
        db_sets: extraction.db_sets,
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(source.to_string()),
        has_errors: extraction.has_errors,
        flow: Default::default(),
        demand_contributions: extraction.demand_contributions,
        alias_targets: extraction.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut sym_ids = crate::type_checker::core::SymbolIdMap::default();
    for (idx, _) in pf.symbols.iter().enumerate() {
        sym_ids.insert((pf.path.clone(), idx), idx as i64 + 1);
    }

    let mut arena = TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );

    // Pull the qnames the extractor emitted for Repo / User / Get / Name.
    let user_qname = pf
        .symbols
        .iter()
        .find(|s| s.name == "User" && s.kind == SymbolKind::Struct)
        .map(|s| s.qualified_name.clone())
        .expect("User struct extracted");
    let repo_qname = pf
        .symbols
        .iter()
        .find(|s| s.name == "Repo" && s.kind == SymbolKind::Struct)
        .map(|s| s.qualified_name.clone())
        .expect("Repo struct extracted");
    let user_ty = arena.class(&user_qname);
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );
    // Splice in `Get` return type and `Name` declared type, as Phase 5+
    // extractor migration will eventually surface these directly.
    let get_idx = pf
        .symbols
        .iter()
        .position(|s| s.name == "Get" && s.kind == SymbolKind::Method)
        .expect("Get method extracted");
    let get_id = sym_ids[&(pf.path.clone(), get_idx)];
    symbol_types.insert(
        get_id,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    let name_idx = pf
        .symbols
        .iter()
        .position(|s| s.name == "Name" && matches!(s.kind, SymbolKind::Property | SymbolKind::Field))
        .expect("Name field extracted");
    let name_id = sym_ids[&(pf.path.clone(), name_idx)];
    symbol_types.insert(
        name_id,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_type("Repo", &repo_qname);

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg(&repo_qname, SegmentKind::TypeAccess),
            seg("Get", SegmentKind::Property),
            seg("Name", SegmentKind::Property),
        ],
    };
    let source_sym = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("Name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("Repo.Get().Name resolves end-to-end on real Go extraction");
    assert_eq!(result.target_symbol_id, name_id);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn gate_real_ts_method_call_chain_walks_three_segments() {
    // class Repo { get(): User; }   class User { name: string; }
    // chain: Repo.get().name → method yield User → field name → string.
    use crate::languages::typescript::extract;

    let source = r#"
export class User {
    name: string = "";
}
export class Repo {
    get(): User { return new User(); }
}
"#;
    let extraction = extract::extract(source, false);
    let pf = crate::types::ParsedFile {
        path: "src/repo.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: source.len() as u64,
        line_count: source.lines().count() as u32,
        mtime: None,
        package_id: None,
        symbols: extraction.symbols,
        refs: extraction.refs,
        routes: extraction.routes,
        db_sets: extraction.db_sets,
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(source.to_string()),
        has_errors: extraction.has_errors,
        flow: Default::default(),
        demand_contributions: extraction.demand_contributions,
        alias_targets: extraction.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut sym_ids = crate::type_checker::core::SymbolIdMap::default();
    for (idx, _) in pf.symbols.iter().enumerate() {
        sym_ids.insert((pf.path.clone(), idx), idx as i64 + 1);
    }

    let mut arena = TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );

    let user_ty = arena.class("User");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );
    // Phase 5+ extractor will fill these; for Phase 4's gate splice in the
    // canonical return/declared types so the walker has type info to thread.
    let get_idx = pf
        .symbols
        .iter()
        .position(|s| s.name == "get" && s.kind == SymbolKind::Method)
        .expect("`get` method extracted");
    let get_id = sym_ids[&(pf.path.clone(), get_idx)];
    symbol_types.insert(
        get_id,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    let name_idx = pf
        .symbols
        .iter()
        .position(|s| s.name == "name" && matches!(s.kind, SymbolKind::Property | SymbolKind::Field))
        .expect("`name` field extracted");
    let name_id = sym_ids[&(pf.path.clone(), name_idx)];
    symbol_types.insert(
        name_id,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_type("Repo", "Repo");

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("Repo", SegmentKind::TypeAccess),
            seg("get", SegmentKind::Property),
            seg("name", SegmentKind::Property),
        ],
    };
    let source_sym = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("Repo.get().name must resolve end-to-end");
    assert_eq!(result.target_symbol_id, name_id);
    assert_eq!(result.resolved_yield_type, str_ty);
}
