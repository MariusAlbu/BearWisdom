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
    AliasTarget, CallArg, ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain,
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
        is_call: false,
        call_args: Vec::new(),
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

fn sym_info_sig(
    id: i64,
    name: &str,
    qname: &str,
    kind: &str,
    scope: Option<&str>,
    sig: &str,
) -> SymbolInfo {
    SymbolInfo {
        signature: Some(sig.to_string()),
        ..sym_info(id, name, qname, kind, scope)
    }
}

fn dummy_extracted_ref(target: &str) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
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
    local_unions: rustc_hash::FxHashMap<String, Vec<String>>,
    field_types: rustc_hash::FxHashMap<String, String>,
    return_types: rustc_hash::FxHashMap<String, String>,
    by_qname: rustc_hash::FxHashMap<String, SymbolInfo>,
    parents: rustc_hash::FxHashMap<String, String>,
    generic_param_type_ids: rustc_hash::FxHashMap<String, Vec<TypeId>>,
    discriminants: rustc_hash::FxHashMap<String, (String, String, bool)>,
    recorded_locals: std::cell::RefCell<Vec<(String, String)>>,
}

impl EmptyLookup {
    fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            types: Default::default(),
            locals: Default::default(),
            local_unions: Default::default(),
            field_types: Default::default(),
            return_types: Default::default(),
            by_qname: Default::default(),
            parents: Default::default(),
            generic_param_type_ids: Default::default(),
            discriminants: Default::default(),
            recorded_locals: Default::default(),
        }
    }
    fn with_discriminant(mut self, name: &str, prop: &str, literal: &str) -> Self {
        self.discriminants
            .insert(name.to_string(), (prop.to_string(), literal.to_string(), false));
        self
    }
    fn with_negated_discriminant(mut self, name: &str, prop: &str, literal: &str) -> Self {
        self.discriminants
            .insert(name.to_string(), (prop.to_string(), literal.to_string(), true));
        self
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
    fn with_local_union(mut self, name: &str, branches: &[&str]) -> Self {
        self.local_unions
            .insert(name.to_string(), branches.iter().map(|s| s.to_string()).collect());
        self
    }
    fn with_field_type(mut self, qname: &str, type_name: &str) -> Self {
        self.field_types
            .insert(qname.to_string(), type_name.to_string());
        self
    }
    fn with_external_type(mut self, short_name: &str, ext_qname: &str) -> Self {
        let mut info = sym_info(99, short_name, ext_qname, "class", None);
        info.file_path = Arc::from("ext:test");
        self.types
            .entry(short_name.to_string())
            .or_default()
            .push(info);
        self
    }
    fn with_qname_symbol(mut self, qname: &str, info: SymbolInfo) -> Self {
        self.by_qname.insert(qname.to_string(), info);
        self
    }
    fn with_return_type(mut self, qname: &str, type_name: &str) -> Self {
        self.return_types
            .insert(qname.to_string(), type_name.to_string());
        self
    }
    fn with_parent(mut self, child: &str, parent: &str) -> Self {
        self.parents.insert(child.to_string(), parent.to_string());
        self
    }
    fn with_generic_param_type_ids(mut self, scope: &str, ids: Vec<TypeId>) -> Self {
        self.generic_param_type_ids.insert(scope.to_string(), ids);
        self
    }
}

impl SymbolLookup for EmptyLookup {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.get(qname)
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
    fn local_type_union(&self, name: &str) -> Option<Vec<String>> {
        if let Some(u) = self.local_unions.get(name) {
            return Some(u.clone());
        }
        self.locals.get(name).cloned().map(|s| vec![s])
    }
    fn local_discriminant(&self, name: &str) -> Option<(String, String, bool)> {
        self.discriminants.get(name).cloned()
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
    fn return_type_name(&self, qname: &str) -> Option<&str> {
        self.return_types.get(qname).map(|s| s.as_str())
    }
    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.parents.get(class_qname).map(|s| s.as_str())
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_param_type_ids(&self, type_name: &str) -> Option<&[TypeId]> {
        self.generic_param_type_ids
            .get(type_name)
            .map(|v| v.as_slice())
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
    fn record_local_type(&self, name: String, type_name: String) {
        self.recorded_locals.borrow_mut().push((name, type_name));
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
            _arena: &TypeArena,
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
fn turbofish_binds_method_own_generic() {
    // class Repo { find<U>(): U }  — repo.find<User>().name resolves `name` on
    // User. The return is stored param-blind as Class("U") (the production
    // shape from intern_type_str); the turbofish binds U and the primary-path
    // rebind canonicalizes Class("U") → Generic(U) so substitution fires.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let repo_ty = arena.class("Repo");
    let user_ty = arena.class("User");
    let class_u = arena.class("U");
    let u_param = arena.intern_generic(GenericParamData {
        name: "U".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_u = arena.intern(Type::Generic { param: u_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_u), // nominal Class("U"), the prod shape
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(repo_ty, sym_info(1, "find", "Repo.find", "method", Some("Repo")));
    members.add_direct(user_ty, sym_info(2, "name", "User.name", "property", Some("User")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_generic_param_type_ids("Repo.find", vec![gen_u]);

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
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
            seg("repo", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                type_args: vec!["User".to_string()],
                ..seg("find", SegmentKind::Property)
            },
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
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: repo_ty })
        .expect("name resolves on the turbofish-bound return type");
    assert_eq!(result.target_symbol_id, 2);
}

// ---------------------------------------------------------------------------
// INFER-8 — argument-driven generic inference at a terminal call
// ---------------------------------------------------------------------------

// A minimal root resolver that returns a fixed prebuilt type.
struct InferFixedRoot {
    ty: TypeId,
}
impl RootResolver for InferFixedRoot {
    fn resolve(
        &self,
        _seg: &ChainSegment,
        _ref_ctx: &RefContext,
        _file_ctx: &FileContext,
        _arena: &TypeArena,
        _lookup: &dyn SymbolLookup,
    ) -> Option<TypeId> {
        Some(self.ty)
    }
}

#[test]
fn arg_driven_generic_binds_terminal_yield() {
    // class Box { wrap<T>(x: T): T }  — box.wrap(u) with a local `u: User` and
    // no turbofish binds T from the argument, so the call yields User. The
    // return + param are stored param-blind (Class("T"), the prod shape);
    // `generic_param_type_ids` supplies the canonical Generic(T).
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let box_ty = arena.class("Box");
    let user_ty = arena.class("User");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),    // param-blind Class("T")
            param_types: vec![class_t],    // x: T, also param-blind
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(box_ty, sym_info(1, "wrap", "Box.wrap", "method", Some("Box")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("Box.wrap", vec![gen_t])
        .with_local("u", "User");

    let walker = ChainWalker::new(
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
            seg("box", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("wrap", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Ident("u".to_string())],
        ..dummy_extracted_ref("wrap")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: box_ty })
        .expect("wrap resolves");
    assert_eq!(result.target_symbol_id, 1);
    // T was inferred from the argument's type → yield is User, not unbound T.
    assert_eq!(result.resolved_yield_type, user_ty);
}

#[test]
fn arg_driven_generic_inferred_through_array_arg() {
    // class Svc { firstOf<T>(xs: Array<T>): T }  — svc.firstOf(items) with a
    // local `items: Array<User>` recurses into the Array application to bind
    // T → User, so the call yields User.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let svc_ty = arena.class("Svc");
    let user_ty = arena.class("User");
    let class_t = arena.class("T");
    let array_cls = arena.class("Array");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });
    // Declared param `xs: Array<T>`, param-blind: Apply{Array, [Class("T")]}.
    let array_of_t = arena.intern(Type::Apply {
        base: array_cls,
        args: vec![class_t],
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![array_of_t],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        svc_ty,
        sym_info(1, "firstOf", "Svc.firstOf", "method", Some("Svc")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("Svc.firstOf", vec![gen_t])
        .with_local("items", "Array<User>");

    let walker = ChainWalker::new(
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
            seg("svc", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("firstOf", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Ident("items".to_string())],
        ..dummy_extracted_ref("firstOf")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: svc_ty })
        .expect("firstOf resolves");
    assert_eq!(result.target_symbol_id, 1);
    assert_eq!(result.resolved_yield_type, user_ty);
}

#[test]
fn turbofish_overrides_arg_driven_inference() {
    // box.wrap<Admin>(u) with `u: User` — the explicit turbofish binds T=Admin
    // and arg-driven inference is gated off, so the call yields Admin not User.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let box_ty = arena.class("Box");
    let admin_ty = arena.class("Admin");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(box_ty, sym_info(1, "wrap", "Box.wrap", "method", Some("Box")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("Box.wrap", vec![gen_t])
        .with_local("u", "User");

    let walker = ChainWalker::new(
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
            seg("box", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                type_args: vec!["Admin".to_string()],
                ..seg("wrap", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Ident("u".to_string())],
        ..dummy_extracted_ref("wrap")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: box_ty })
        .expect("wrap resolves");
    assert_eq!(result.target_symbol_id, 1);
    assert_eq!(result.resolved_yield_type, admin_ty);
}

#[test]
fn arg_driven_no_inference_when_arg_untyped() {
    // box.wrap(mystery) where `mystery` has no known local type — the argument
    // resolves to Unknown, so T stays unbound and the call yields the generic
    // T unchanged (byte-identical to the pre-inference behavior).
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let box_ty = arena.class("Box");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(box_ty, sym_info(1, "wrap", "Box.wrap", "method", Some("Box")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // No local type for `mystery` → argument types to Unknown.
    let lookup = EmptyLookup::new().with_generic_param_type_ids("Box.wrap", vec![gen_t]);

    let walker = ChainWalker::new(
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
            seg("box", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("wrap", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Ident("mystery".to_string())],
        ..dummy_extracted_ref("wrap")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: box_ty })
        .expect("wrap resolves");
    assert_eq!(result.target_symbol_id, 1);
    // T was not inferable → the yield is the unbound generic, not a guess.
    assert_eq!(result.resolved_yield_type, gen_t);
}

#[test]
fn arg_driven_infers_owner_param_when_receiver_unbound() {
    // class Repository<T> { findOne(filter: T): T }  — the method declares no
    // generics of its own; T is the OWNING type's parameter. With a raw
    // `Repository` receiver (no type args), the receiver leaves T unbound, so
    // the argument's type fills it: repo.findOne(u) with `u: User` yields User.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let repo_ty = arena.class("Repository");
    let user_ty = arena.class("User");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        repo_ty,
        sym_info(1, "findOne", "Repository.findOne", "method", Some("Repository")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // T is declared on the OWNER (`Repository`), not on the method.
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("Repository", vec![gen_t])
        .with_local("u", "User");

    let walker = ChainWalker::new(
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
            seg("repo", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("findOne", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Ident("u".to_string())],
        ..dummy_extracted_ref("findOne")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: repo_ty })
        .expect("findOne resolves");
    assert_eq!(result.target_symbol_id, 1);
    assert_eq!(result.resolved_yield_type, user_ty);
}

#[test]
fn receiver_binding_wins_over_arg_driven_inference() {
    // Same Repository<T> { findOne(filter: T): T }, but the receiver is
    // `Repository<Account>` — the receiver binds T → Account first, so an
    // argument of a different type does NOT clobber it: findOne(u) with
    // `u: User` still yields Account.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let repo_ty = arena.class("Repository");
    let account_ty = arena.class("Account");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });
    let repo_of_account = arena.intern(Type::Apply {
        base: repo_ty,
        args: vec![account_ty],
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        repo_ty,
        sym_info(1, "findOne", "Repository.findOne", "method", Some("Repository")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("Repository", vec![gen_t])
        .with_local("u", "User");

    let walker = ChainWalker::new(
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
            seg("repo", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("findOne", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Ident("u".to_string())],
        ..dummy_extracted_ref("findOne")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: repo_of_account })
        .expect("findOne resolves");
    assert_eq!(result.target_symbol_id, 1);
    // Receiver pinned T → Account; the User argument must not override it.
    assert_eq!(result.resolved_yield_type, account_ty);
}

#[test]
fn arg_driven_generic_binds_mid_chain_yield() {
    // class Repo<T> { find(x: T): T }  — `repo.find(user).name` in one
    // expression. `find` is INVOKED mid-chain (not the terminal segment), so
    // its T must bind from the segment's own arg `user: User`. Then `find`
    // yields User and the terminal `name` field resolves on User. The terminal
    // segment is the field `name` (NOT a call), so the terminal G2 cannot fire;
    // only the per-segment mid-chain bind can supply T.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let repo_ty = arena.class("Repo");
    let user_ty = arena.class("User");
    let string_ty = arena.class("String");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        repo_ty,
        sym_info(1, "find", "Repo.find", "method", Some("Repo")),
    );
    members.add_direct(
        user_ty,
        sym_info(2, "name", "User.name", "field", Some("User")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // T is declared on the OWNER (`Repo`); `name`'s field type is String.
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("Repo", vec![gen_t])
        .with_field_type("User.name", "String")
        .with_local("user", "User");

    let walker = ChainWalker::new(
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
            seg("repo", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: vec![CallArg::Ident("user".to_string())],
                ..seg("find", SegmentKind::Property)
            },
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    // Terminal ref targets the field `name` with no args — so terminal G2 stays
    // off (it requires a non-empty `call_args`); only the mid-chain bind on
    // `find` can supply T.
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: repo_ty })
        .expect("name resolves through mid-chain find");
    assert_eq!(result.target_symbol_id, 2);
    assert_eq!(result.resolved_yield_type, string_ty);
}

#[test]
fn mid_chain_receiver_binding_wins() {
    // Same Repo<T> { find(x: T): T }, receiver is `Repo<Account>`. The receiver
    // binds T → Account first, so a mid-chain arg of a DIFFERENT type does not
    // clobber it: `repo.find(user).name` with `repo: Repo<Account>` and
    // `user: User` resolves `name` on Account, not User.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let repo_ty = arena.class("Repo");
    let account_ty = arena.class("Account");
    let user_ty = arena.class("User");
    let string_ty = arena.class("String");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });
    let repo_of_account = arena.intern(Type::Apply {
        base: repo_ty,
        args: vec![account_ty],
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        repo_ty,
        sym_info(1, "find", "Repo.find", "method", Some("Repo")),
    );
    // `name` exists on BOTH Account and User; the receiver binding decides which
    // one the chain lands on. Only the Account field carries id 3.
    members.add_direct(
        account_ty,
        sym_info(3, "name", "Account.name", "field", Some("Account")),
    );
    members.add_direct(
        user_ty,
        sym_info(2, "name", "User.name", "field", Some("User")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("Repo", vec![gen_t])
        .with_field_type("Account.name", "String")
        .with_field_type("User.name", "String")
        .with_local("user", "User");

    let walker = ChainWalker::new(
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
            seg("repo", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: vec![CallArg::Ident("user".to_string())],
                ..seg("find", SegmentKind::Property)
            },
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
        .walk_with_root(&chain, &ref_ctx, &fc, &InferFixedRoot { ty: repo_of_account })
        .expect("name resolves on Account");
    // Receiver pinned T → Account; the mid-chain User arg must not override it.
    assert_eq!(result.target_symbol_id, 3);
    assert_eq!(result.resolved_yield_type, string_ty);
}

#[test]
fn bare_call_binds_generic_return_from_arg() {
    // function makeThing<T>(x: T): T — a chain-less `makeThing(u)` with a
    // local `u: User` binds T from the argument, so the bare call yields User
    // instead of the unbindable bare parameter. The return + param are stored
    // param-blind (Class("T")); `generic_param_type_ids` supplies Generic(T).
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("makeThing", vec![gen_t])
        .with_local("u", "User");

    let walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let sym = sym_info(1, "makeThing", "makeThing", "function", None);
    let yielded = walker
        .infer_bare_call_yield(&sym, &[CallArg::Ident("u".to_string())])
        .expect("generic bare call yields the bound return");
    assert_eq!(yielded, user_ty);
}

#[test]
fn bare_call_binds_generic_through_array_arg() {
    // function firstOf<T>(xs: Array<T>): T — `firstOf(items)` with a local
    // `items: Array<User>` recurses into the Array application to bind T → User.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let class_t = arena.class("T");
    let array_cls = arena.class("Array");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });
    let array_of_t = arena.intern(Type::Apply {
        base: array_cls,
        args: vec![class_t],
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![array_of_t],
            ..Default::default()
        },
    );

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_generic_param_type_ids("firstOf", vec![gen_t])
        .with_local("items", "Array<User>");

    let walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let sym = sym_info(1, "firstOf", "firstOf", "function", None);
    let yielded = walker
        .infer_bare_call_yield(&sym, &[CallArg::Ident("items".to_string())])
        .expect("array-generic bare call yields the element type");
    assert_eq!(yielded, user_ty);
}

#[test]
fn lambda_param_seeded_from_callback_signature() {
    // interface Array<T> { map<U>(cb: (v: T) => U): U[] }
    // const arr: User[]; arr.map(x => ...) — the receiver binds T→User via
    // bind_apply_args, so map's callback param `(v: T)` substitutes to User and
    // the lambda's own param `x` is seeded `User` in the forward cache.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");
    let class_t = arena.class("T");
    let array_cls = arena.class("Array");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });
    // map's declared callback type: `(v: T) => U`, param-blind Class("T").
    let class_u = arena.class("U");
    let cb_fn = arena.intern(Type::Function {
        params: vec![class_t],
        return_: class_u,
    });
    // Receiver `Array<User>` so bind_apply_args binds T→User.
    let array_of_user = arena.intern(Type::Apply {
        base: array_cls,
        args: vec![user_ty],
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        7,
        SymbolTypeData {
            param_types: vec![cb_fn],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        array_cls,
        sym_info(7, "map", "Array.map", "method", Some("Array")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // `Array`'s generic param T is the canonical Generic(T); the receiver's
    // Apply args bind it, and owner_param_type_map(Array.map) rebinds the
    // callback's Class("T") to it.
    let lookup = EmptyLookup::new().with_generic_param_type_ids("Array", vec![gen_t]);

    struct ApplyRoot {
        ty: TypeId,
    }
    impl RootResolver for ApplyRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
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
            seg("arr", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("map", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Lambda { params: vec!["x".to_string()] }],
        ..dummy_extracted_ref("map")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let _ = walker.walk_with_root(&chain, &ref_ctx, &fc, &ApplyRoot { ty: array_of_user });

    let recorded = lookup.recorded_locals.borrow();
    assert!(
        recorded
            .iter()
            .any(|(n, t)| n == "x" && t == "User"),
        "expected lambda param x seeded as User, got: {recorded:?}"
    );
}

#[test]
fn lambda_param_unbound_receiver_seeds_nothing() {
    // Raw `Array` receiver (no element type): T binds nothing, so the callback
    // param stays Generic — the seed must skip it rather than record bare T.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let class_t = arena.class("T");
    let array_cls = arena.class("Array");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });
    let class_u = arena.class("U");
    let cb_fn = arena.intern(Type::Function {
        params: vec![class_t],
        return_: class_u,
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        7,
        SymbolTypeData {
            param_types: vec![cb_fn],
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        array_cls,
        sym_info(7, "map", "Array.map", "method", Some("Array")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_generic_param_type_ids("Array", vec![gen_t]);

    struct BareRoot {
        ty: TypeId,
    }
    impl RootResolver for BareRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
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
            seg("arr", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("map", SegmentKind::Property)
            },
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        call_args: vec![CallArg::Lambda { params: vec!["x".to_string()] }],
        ..dummy_extracted_ref("map")
    };
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let _ = walker.walk_with_root(&chain, &ref_ctx, &fc, &BareRoot { ty: array_cls });

    assert!(
        lookup.recorded_locals.borrow().is_empty(),
        "unbound receiver must seed nothing, got: {:?}",
        lookup.recorded_locals.borrow()
    );
}

#[test]
fn bare_call_non_generic_returns_none() {
    // function build(x): User — no generic params, so the bare-call inference
    // declines; the concrete return flows through the loop's return-type
    // fallback instead of being duplicated here.
    let mut arena = TypeArena::new();
    let user_ty = arena.class("User");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(user_ty),
            param_types: vec![user_ty],
            ..Default::default()
        },
    );

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_local("u", "User");

    let walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let sym = sym_info(1, "build", "build", "function", None);
    assert_eq!(
        walker.infer_bare_call_yield(&sym, &[CallArg::Ident("u".to_string())]),
        None
    );
}

#[test]
fn bare_call_untyped_arg_returns_none() {
    // Generic function, but the argument has no known type, so T can't bind —
    // the inference declines rather than guessing or recording bare T.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let class_t = arena.class("T");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(class_t),
            param_types: vec![class_t],
            ..Default::default()
        },
    );

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // `mystery` has no registered local type → the arg resolves to Unknown.
    let lookup = EmptyLookup::new().with_generic_param_type_ids("makeThing", vec![gen_t]);

    let walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let sym = sym_info(1, "makeThing", "makeThing", "function", None);
    assert_eq!(
        walker.infer_bare_call_yield(&sym, &[CallArg::Ident("mystery".to_string())]),
        None
    );
}

#[test]
fn bare_call_no_args_returns_none() {
    // A zero-argument call can bind nothing — decline.
    let mut arena = TypeArena::new();
    let symbol_types = SymbolTypeMap::new();
    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();
    let walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let sym = sym_info(1, "makeThing", "makeThing", "function", None);
    assert_eq!(walker.infer_bare_call_yield(&sym, &[]), None);
}

#[test]
fn cast_segment_adopts_asserted_type() {
    // (x as Admin).ban() — the cast asserts Admin, so `ban` resolves on Admin
    // regardless of x's own (here irrelevant) type.
    let mut arena = TypeArena::new();
    let unknown_ty = arena.class("Whatever");
    let admin_ty = arena.class("Admin");

    let symbol_types = SymbolTypeMap::new();
    let mut members = MembersIndex::new();
    members.add_direct(admin_ty, sym_info(5, "ban", "Admin.ban", "method", Some("Admin")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
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
            ChainSegment {
                declared_type: Some("Admin".to_string()),
                ..seg("x", SegmentKind::Identifier)
            },
            seg("ban", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("ban");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: unknown_ty })
        .expect("ban resolves on the asserted Admin type");
    assert_eq!(result.target_symbol_id, 5);
}

#[test]
fn opaque_existential_root_resolves_protocol_extension_default() {
    // A receiver declared `some Greet` / `any Greet` dispatches members through
    // the protocol `Greet`, where a protocol extension files its default `hello`
    // (the part-(b) scope_path filing). The opaque/existential keyword prefix is
    // peeled at intern time, so the chain root types as `Greet` and the member
    // walk finds the default.
    for opaque in ["some Greet", "any Greet"] {
        let mut arena = TypeArena::new();
        let irrelevant = arena.class("Whatever");
        let greet_ty = arena.class("Greet");

        let symbol_types = SymbolTypeMap::new();
        let mut members = MembersIndex::new();
        members.add_direct(
            greet_ty,
            sym_info(7, "hello", "Greet.hello", "method", Some("Greet")),
        );

        let supertypes = SupertypeGraph::new();
        let aliases = AliasIndex::default();
        let lookup = EmptyLookup::new();

        struct FixedRoot {
            ty: TypeId,
        }
        impl RootResolver for FixedRoot {
            fn resolve(
                &self,
                _seg: &ChainSegment,
                _ref_ctx: &RefContext,
                _file_ctx: &FileContext,
                _arena: &TypeArena,
                _lookup: &dyn SymbolLookup,
            ) -> Option<TypeId> {
                Some(self.ty)
            }
        }

        let walker = ChainWalker::new(
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
                ChainSegment {
                    declared_type: Some(opaque.to_string()),
                    ..seg("g", SegmentKind::Identifier)
                },
                seg("hello", SegmentKind::Property),
            ],
        };
        let source = dummy_source_symbol("caller", None);
        let r = dummy_extracted_ref("hello");
        let ref_ctx = RefContext {
            extracted_ref: &r,
            source_symbol: &source,
            scope_chain: Vec::new(),
            file_package_id: None,
        };
        let fc = file_ctx();
        let result = walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: irrelevant })
            .unwrap_or_else(|| panic!("hello resolves on the peeled `{opaque}` receiver"));
        assert_eq!(result.target_symbol_id, 7, "{opaque} dispatches `hello` through Greet");
    }
}

#[test]
fn parameter_typed_receiver_resolves_protocol_extension_default() {
    // The real Swift path for an opaque/existential parameter receiver:
    // `func use(g: some Greet) { g.hello() }`. The extractor emits `g` as a
    // Parameter scoped under `use` whose declared type is captured (keyword
    // already peeled to `Greet`), so the chain root resolves through
    // `field_type("use.g") = "Greet"` (scope-chain walk in the root resolver),
    // not through a segment-level `declared_type`. `hello` is filed under
    // `Greet` (the part-(b) protocol-extension default). This is the seam the
    // segment-injected opaque test could not exercise.
    let mut arena = TypeArena::new();
    let greet_ty = arena.class("Greet");

    let symbol_types = SymbolTypeMap::new();
    let mut members = MembersIndex::new();
    members.add_direct(
        greet_ty,
        sym_info(7, "hello", "Greet.hello", "method", Some("Greet")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // The parameter `g` files under `use.g`; its captured declared type is the
    // peeled constraint `Greet`. No segment-level declared_type is set.
    let lookup = EmptyLookup::new().with_field_type("use.g", "Greet");

    let walker = ChainWalker::new(
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
            seg("g", SegmentKind::Identifier),
            seg("hello", SegmentKind::Property),
        ],
    };
    // The source symbol is the enclosing function `use`; its qname seeds the
    // scope chain the root resolver walks for `{scope}.g`.
    let source = dummy_source_symbol("use", None);
    let r = dummy_extracted_ref("hello");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: vec!["use".to_string()],
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("hello resolves on the parameter-typed `some Greet` receiver");
    assert_eq!(
        result.target_symbol_id, 7,
        "parameter `g: some Greet` dispatches `hello` through Greet"
    );
}

#[test]
fn generic_arg_substitutes_through_inheritance() {
    // class Repository<T> { find_one(): T }
    // class UserRepo: Repository<User> {}
    // let r: UserRepo; r.find_one() → yields User (T bound through the
    // `extends Repository<User>` edge), NOT the unbound generic T.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let repository_ty = arena.class("Repository");
    let user_repo_ty = arena.class("UserRepo");
    let user_ty = arena.class("User");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let generic_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    // Repository<T> — generic, self-yields, declares T.
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(repository_ty),
            generic_params: vec![t_param],
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(repository_ty, 1);
    // UserRepo — non-generic subclass.
    symbol_types.insert(
        2,
        SymbolTypeData {
            return_type: Some(user_repo_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_repo_ty, 2);
    // User — leaf type.
    symbol_types.insert(
        3,
        SymbolTypeData {
            return_type: Some(user_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_ty, 3);
    // find_one(): T — declared on Repository.
    symbol_types.insert(
        4,
        SymbolTypeData {
            return_type: Some(generic_t),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        repository_ty,
        sym_info(4, "find_one", "Repository.find_one", "method", Some("Repository")),
    );

    // UserRepo extends Repository<User> — the edge carries the bound arg.
    let mut supertypes = SupertypeGraph::new();
    supertypes.add_edge_generic(user_repo_ty, repository_ty, vec![user_ty]);

    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("find_one", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("find_one");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: user_repo_ty })
        .expect("inherited generic method resolves");
    assert_eq!(result.target_symbol_id, 4);
    // T must be bound to User through the inheritance edge.
    assert_eq!(result.resolved_yield_type, user_ty);
}

#[test]
fn generic_arg_composes_through_multi_level_inheritance() {
    // class C<T> { item(): T }
    // class B<T> extends C<T> {}        // B→C edge arg = Generic(B::T)  (G2 output)
    // class A    extends B<X> {}        // A→B edge arg = X (concrete)
    // let a: A; a.item() → X — the B::T → X binding must compose across the
    // B→C hop (G3), not surface the unbound Generic(B::T).
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let a_ty = arena.class("A");
    let b_ty = arena.class("B");
    let c_ty = arena.class("C");
    let x_ty = arena.class("X");
    let b_t = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 3,
        bound: None,
    });
    let c_t = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 4,
        bound: None,
    });
    let generic_bt = arena.intern(Type::Generic { param: b_t });
    let generic_ct = arena.intern(Type::Generic { param: c_t });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(2, SymbolTypeData { return_type: Some(a_ty), ..Default::default() });
    symbol_types.mark_self_yielding(a_ty, 2);
    symbol_types.insert(
        3,
        SymbolTypeData { return_type: Some(b_ty), generic_params: vec![b_t], ..Default::default() },
    );
    symbol_types.mark_self_yielding(b_ty, 3);
    symbol_types.insert(
        4,
        SymbolTypeData { return_type: Some(c_ty), generic_params: vec![c_t], ..Default::default() },
    );
    symbol_types.mark_self_yielding(c_ty, 4);
    symbol_types.insert(1, SymbolTypeData { return_type: Some(generic_ct), ..Default::default() });

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(1, "item", "C.item", "method", Some("C")));

    // A → B<X> (concrete) ; B → C<T> (B's own param, as G2 emits) ; B declares T.
    let mut supertypes = SupertypeGraph::new();
    supertypes.add_edge_generic(a_ty, b_ty, vec![x_ty]);
    supertypes.add_edge_generic(b_ty, c_ty, vec![generic_bt]);
    supertypes.record_node_params(b_ty, vec![b_t]);

    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("a", SegmentKind::Identifier),
            seg("item", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("item");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: a_ty })
        .expect("multi-level inherited generic resolves");
    assert_eq!(result.target_symbol_id, 1);
    // C::T composes through B::T → X across both hops.
    assert_eq!(result.resolved_yield_type, x_ty);
}

#[test]
fn bare_param_receiver_types_as_generic_then_resolves_via_bound() {
    // fn f<T: Animal>(t: T) { t.name() }
    // The receiver `t` is declared as the bare generic param `T`. The default
    // root resolver must type it as the canonical Type::Generic (carrying the
    // bound), NOT a class literally named "T", so the member walk reaches
    // Animal.name() through the bound.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let animal_ty = arena.class("Animal");
    let string_ty = arena.class("String");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: Some(animal_ty),
    });
    let generic_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(string_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        animal_ty,
        sym_info(1, "name", "Animal.name", "method", Some("Animal")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // `t`'s declared type is "T"; scope `f` declares T → Type::Generic{T}.
    let lookup = EmptyLookup::new()
        .with_field_type("f.t", "T")
        .with_generic_param_type_ids("f", vec![generic_t]);

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
            seg("t", SegmentKind::Identifier),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("name");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: vec!["f".to_string()],
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("bare generic receiver resolves via bound");
    assert_eq!(result.target_symbol_id, 1);
    assert_eq!(result.resolved_yield_type, string_ty);
}

#[test]
fn f_bounded_param_resolves_member_on_generic_bound() {
    // fn f<T: Comparable<T>>(x: T) { x.compareTo(...) }
    // T's bound is the generic application Comparable<T>; member lookup must
    // follow it to Comparable.compareTo (the inner self-ref T stays nominal —
    // it doesn't affect resolving compareTo on the base).
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let comparable_ty = arena.class("Comparable");
    let int_ty = arena.class("int");
    // bound = Comparable<T> (inner T nominal, as gap-B's intern_type_str emits).
    let inner_t = arena.class("T");
    let bound = arena.intern(Type::Apply {
        base: comparable_ty,
        args: vec![inner_t],
    });
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: Some(bound),
    });
    let generic_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(1, SymbolTypeData { return_type: Some(int_ty), ..Default::default() });

    let mut members = MembersIndex::new();
    members.add_direct(
        comparable_ty,
        sym_info(1, "compareTo", "Comparable.compareTo", "method", Some("Comparable")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("x", SegmentKind::Identifier),
            seg("compareTo", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("compareTo");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: generic_t })
        .expect("f-bounded member resolves via generic bound");
    assert_eq!(result.target_symbol_id, 1);
}

#[test]
fn called_function_typed_field_yields_return_type() {
    // obj.handler().name where handler: () => User. Calling the function-typed
    // field yields User, so `name` resolves on User. Without call-yield,
    // `name` would be looked up on the function value and miss.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let obj = arena.class("Obj");
    let user = arena.class("User");
    let handler_fn = arena.intern(Type::Function {
        params: vec![],
        return_: user,
    });

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        1,
        SymbolTypeData {
            declared_type: Some(handler_fn),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(obj, sym_info(1, "handler", "Obj.handler", "property", Some("Obj")));
    members.add_direct(user, sym_info(2, "name", "User.name", "property", Some("User")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("obj", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("handler", SegmentKind::Property)
            },
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
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: obj })
        .expect("name resolves on the called handler's return type");
    assert_eq!(result.target_symbol_id, 2);
}

#[test]
fn called_function_typed_root_yields_return_type() {
    // f().name where f: () => User. Calling the function-typed root yields User,
    // so `name` resolves on User — the root-call form of closure call-yield.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let make_fn = arena.intern(Type::Function {
        params: vec![],
        return_: user,
    });

    let symbol_types = SymbolTypeMap::new();
    let mut members = MembersIndex::new();
    members.add_direct(user, sym_info(2, "name", "User.name", "property", Some("User")));

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
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
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("f", SegmentKind::Identifier)
            },
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
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: make_fn })
        .expect("name resolves on the called function root's return type");
    assert_eq!(result.target_symbol_id, 2);
}

#[test]
fn discriminated_union_narrows_to_matching_branch() {
    // type Shape = Circle | Square; if (s.kind === "circle") { s.radius }
    // The active discriminant guard selects Circle, whose `radius` resolves.
    // Without narrowing, `radius` is absent from the Square branch, so the union
    // arm would miss — a successful resolution proves branch selection fired.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let circle = arena.class("Circle");
    let square = arena.class("Square");
    let union = arena.intern(Type::Union(vec![circle, square]));

    let mut members = MembersIndex::new();
    members.add_direct(
        circle,
        sym_info_sig(1, "kind", "Circle.kind", "property", Some("Circle"), "\"circle\""),
    );
    members.add_direct(
        circle,
        sym_info(2, "radius", "Circle.radius", "property", Some("Circle")),
    );
    members.add_direct(
        square,
        sym_info_sig(3, "kind", "Square.kind", "property", Some("Square"), "\"square\""),
    );
    members.add_direct(
        square,
        sym_info(4, "side", "Square.side", "property", Some("Square")),
    );

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_discriminant("s", "kind", "\"circle\"");

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("s", SegmentKind::Identifier),
            seg("radius", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("radius");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: union })
        .expect("radius resolves on the narrowed Circle branch");
    assert_eq!(result.target_symbol_id, 2);
}

#[test]
fn discriminated_union_negate_excludes_branch() {
    // s: Circle | Square; an early-exit guard `if (s.kind !== "circle") return;`
    // excludes Circle, narrowing `s` to Square — where `side` lives. Resolving
    // `side` proves the Circle branch was dropped by the negated discriminant.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let circle = arena.class("Circle");
    let square = arena.class("Square");
    let union = arena.intern(Type::Union(vec![circle, square]));

    let mut members = MembersIndex::new();
    members.add_direct(
        circle,
        sym_info_sig(1, "kind", "Circle.kind", "property", Some("Circle"), "\"circle\""),
    );
    members.add_direct(
        circle,
        sym_info(2, "radius", "Circle.radius", "property", Some("Circle")),
    );
    members.add_direct(
        square,
        sym_info_sig(3, "kind", "Square.kind", "property", Some("Square"), "\"square\""),
    );
    members.add_direct(
        square,
        sym_info(4, "side", "Square.side", "property", Some("Square")),
    );

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_negated_discriminant("s", "kind", "\"circle\"");

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("s", SegmentKind::Identifier),
            seg("side", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("side");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: union })
        .expect("side resolves on the non-excluded Square branch");
    assert_eq!(result.target_symbol_id, 4);
}

#[test]
fn anonymous_union_intersection_narrows_to_branch() {
    // An anonymous discriminated union is represented as Intersection([Circle,
    // Square]). A guard `s.kind === "circle"` narrows it to Circle: `radius`
    // resolves but `side` (Square-only) MISSES — proving branch precision, not
    // the any-branch lookup an un-narrowed Intersection would give.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let circle = arena.class("S\u{1}0");
    let square = arena.class("S\u{1}1");
    let isect = arena.intern(Type::Intersection(vec![circle, square]));

    let mut members = MembersIndex::new();
    members.add_direct(
        circle,
        sym_info_sig(1, "kind", "S\u{1}0.kind", "property", Some("S\u{1}0"), "\"circle\""),
    );
    members.add_direct(
        circle,
        sym_info(2, "radius", "S\u{1}0.radius", "property", Some("S\u{1}0")),
    );
    members.add_direct(
        square,
        sym_info_sig(3, "kind", "S\u{1}1.kind", "property", Some("S\u{1}1"), "\"square\""),
    );
    members.add_direct(
        square,
        sym_info(4, "side", "S\u{1}1.side", "property", Some("S\u{1}1")),
    );

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_discriminant("s", "kind", "\"circle\"");

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let source = dummy_source_symbol("caller", None);
    let fc = file_ctx();

    // `s.radius` resolves on the narrowed Circle branch.
    let radius_ref = dummy_extracted_ref("radius");
    let radius_ctx = RefContext {
        extracted_ref: &radius_ref,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let radius_chain = MemberChain {
        segments: vec![
            seg("s", SegmentKind::Identifier),
            seg("radius", SegmentKind::Property),
        ],
    };
    let radius = walker
        .walk_with_root(&radius_chain, &radius_ctx, &fc, &FixedRoot { ty: isect })
        .expect("radius resolves on the narrowed Circle branch");
    assert_eq!(radius.target_symbol_id, 2);

    // `s.side` (Square-only) must MISS — the guard narrowed `s` to Circle.
    let side_ref = dummy_extracted_ref("side");
    let side_ctx = RefContext {
        extracted_ref: &side_ref,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let side_chain = MemberChain {
        segments: vec![
            seg("s", SegmentKind::Identifier),
            seg("side", SegmentKind::Property),
        ],
    };
    assert!(
        walker
            .walk_with_root(&side_chain, &side_ctx, &fc, &FixedRoot { ty: isect })
            .is_none(),
        "side must not resolve — narrowed to Circle, which has no `side`"
    );
}

#[test]
fn generic_param_resolves_member_via_bound() {
    // fn f<T: Animal>(t: T) { t.name() }
    // `t` is a bare generic parameter; member lookup must follow the
    // declared upper bound (Animal) to find `name()`.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let animal_ty = arena.class("Animal");
    let string_ty = arena.class("String");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: Some(animal_ty),
    });
    let generic_t = arena.intern(Type::Generic { param: t_param });

    let mut symbol_types = SymbolTypeMap::new();
    // name(): String — declared on Animal.
    symbol_types.insert(
        1,
        SymbolTypeData {
            return_type: Some(string_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        animal_ty,
        sym_info(1, "name", "Animal.name", "method", Some("Animal")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("t", SegmentKind::Identifier),
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
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: generic_t })
        .expect("bounded generic member resolves via bound");
    assert_eq!(result.target_symbol_id, 1);
    assert_eq!(result.resolved_yield_type, string_ty);
}

#[test]
fn string_map_yield_preserves_generic_args() {
    // v.iter() where iter()'s return type is only known as the string
    // "Box<User>" (no SymbolTypeMap TypeId entry). The yield must decompose
    // to Apply{Box,[User]}, not collapse to the bare class Box — otherwise a
    // following `.get()` loses the User element binding.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let vec_ty = arena.class("Vec");
    let box_ty = arena.class("Box");
    let user_ty = arena.class("User");
    let expected = arena.intern(Type::Apply {
        base: box_ty,
        args: vec![user_ty],
    });

    // iter has no SymbolTypeMap entry → yield falls to the string map.
    let symbol_types = SymbolTypeMap::new();

    let mut members = MembersIndex::new();
    members.add_direct(
        vec_ty,
        sym_info(1, "iter", "Vec.iter", "method", Some("Vec")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_return_type("Vec.iter", "Box<User>");

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
            seg("v", SegmentKind::Identifier),
            seg("iter", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("iter");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: vec_ty })
        .expect("string-yield chain resolves");
    assert_eq!(result.target_symbol_id, 1);
    assert_eq!(result.resolved_yield_type, expected);
}

#[test]
fn substitution_fires_from_canonical_params_and_string_yield() {
    // class Repo<T> { first(): T }   const r: Repo<User>;  r.first() → User.
    // Production shape: T's params live ONLY in generic_param_type_ids (not
    // symbol_types), and first's return is known ONLY as the string "T" (no
    // SymbolTypeMap TypeId). Exercises U1 (binder falls back to canonical
    // params) + U2a (string yield rebinds "T" → Generic before substitute).
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

    // No symbol_types entry for Repo (params come from the canonical source)
    // and none for first (its yield must come from the string map).
    let symbol_types = SymbolTypeMap::new();

    let mut members = MembersIndex::new();
    members.add_direct(
        repo_ty,
        sym_info(3, "first", "Repo.first", "method", Some("Repo")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_return_type("Repo.first", "T")
        .with_generic_param_type_ids("Repo", vec![generic_t]);

    struct ApplyRoot {
        ty: TypeId,
    }
    impl RootResolver for ApplyRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
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
        .expect("generic chain resolves");
    assert_eq!(result.target_symbol_id, 3);
    // T must substitute to User via the canonical params + string-yield rebind.
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

#[test]
fn identifier_root_resolves_via_scope_chain_field_type() {
    // Variable `customer` is declared inside method `Equinox.Foo.Bar`.
    // `field_type_name("Equinox.Foo.Bar.customer") = "Customer"`. Chain
    // `customer.name` should resolve through Customer's field map.
    let mut arena = TypeArena::new();
    let customer_ty = arena.class("Customer");
    let str_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Str);

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        10,
        SymbolTypeData {
            return_type: Some(customer_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(customer_ty, 10);
    symbol_types.insert(
        11,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        customer_ty,
        sym_info(11, "name", "Customer.name", "field", Some("Customer")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup =
        EmptyLookup::new().with_field_type("Equinox.Foo.Bar.customer", "Customer");

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
            seg("customer", SegmentKind::Identifier),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("Bar", Some("Equinox.Foo"));
    let mut r = dummy_extracted_ref("name");
    r.kind = EdgeKind::Reads;
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: vec!["Equinox.Foo.Bar".to_string(), "Equinox.Foo".to_string()],
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("customer.name resolves via scope-chain field type");
    assert_eq!(result.target_symbol_id, 11);
    assert_eq!(result.resolved_yield_type, str_ty);
}

#[test]
fn wildcard_import_fallback_prepends_namespace_to_member_qname() {
    // Chain `JsonConvert.SerializeObject(x)` where JsonConvert resolves
    // to a short qname and SerializeObject lives at
    // "Newtonsoft.Json.JsonConvert.SerializeObject". The C# file has
    // `using Newtonsoft.Json;` (wildcard import) so the engine should
    // try `Newtonsoft.Json.JsonConvert.SerializeObject` after the
    // direct probe misses.
    let mut arena = TypeArena::new();
    let jc_ty = arena.class("JsonConvert");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        200,
        SymbolTypeData {
            return_type: Some(jc_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(jc_ty, 200);

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();

    let serialize_sym = sym_info(
        201,
        "SerializeObject",
        "Newtonsoft.Json.JsonConvert.SerializeObject",
        "method",
        Some("Newtonsoft.Json.JsonConvert"),
    );
    let lookup = EmptyLookup::new()
        .with_type("JsonConvert", "JsonConvert")
        .with_qname_symbol(
            "Newtonsoft.Json.JsonConvert.SerializeObject",
            serialize_sym.clone(),
        );

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
            seg("JsonConvert", SegmentKind::TypeAccess),
            seg("SerializeObject", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("SerializeObject");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let mut fc = file_ctx();
    fc.imports.push(crate::indexer::resolve::engine::ImportEntry {
        imported_name: "Newtonsoft.Json".to_string(),
        module_path: Some("Newtonsoft.Json".to_string()),
        alias: None,
        is_wildcard: true,
    });
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("JsonConvert.SerializeObject resolves via using-directive fallback");
    assert_eq!(result.target_symbol_id, 201);
}

#[test]
fn inheritance_walk_finds_member_on_parent_via_qname() {
    // Chain `userRepo.findOne` where UserRepo extends BaseRepo and findOne
    // is declared on BaseRepo. MembersIndex has no entries for either,
    // by_qualified_name("UserRepo.findOne") misses, but
    // parent_class_qname("UserRepo") -> "BaseRepo" and
    // by_qualified_name("BaseRepo.findOne") hits.
    let mut arena = TypeArena::new();
    let user_repo_ty = arena.class("UserRepo");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        300,
        SymbolTypeData {
            return_type: Some(user_repo_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(user_repo_ty, 300);

    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();

    let find_one_sym =
        sym_info(301, "findOne", "BaseRepo.findOne", "method", Some("BaseRepo"));
    let lookup = EmptyLookup::new()
        .with_type("UserRepo", "UserRepo")
        .with_parent("UserRepo", "BaseRepo")
        .with_qname_symbol("BaseRepo.findOne", find_one_sym.clone());

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
            seg("UserRepo", SegmentKind::TypeAccess),
            seg("findOne", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("findOne");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("UserRepo.findOne resolves via parent BaseRepo");
    assert_eq!(result.target_symbol_id, 301);
}

#[test]
fn fluent_this_return_preserves_receiver_type_for_next_segment() {
    // Chain `builder.setTitle().setVersion()` where setTitle and setVersion
    // both have `: this` as their return type, so each call keeps the
    // chain receiver as Builder. The engine must not yield arena.class("this")
    // — it must thread the original receiver type forward.
    let mut arena = TypeArena::new();
    let builder_ty = arena.class("Builder");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        400,
        SymbolTypeData {
            return_type: Some(builder_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(builder_ty, 400);
    // setTitle / setVersion have NO SymbolTypeData entry — yield_type_of
    // falls through to lookup.return_type_name, which returns "this".

    let mut members = MembersIndex::new();
    let set_title_sym = sym_info(
        401,
        "setTitle",
        "Builder.setTitle",
        "method",
        Some("Builder"),
    );
    let set_version_sym = sym_info(
        402,
        "setVersion",
        "Builder.setVersion",
        "method",
        Some("Builder"),
    );
    members.add_direct(builder_ty, set_title_sym.clone());
    members.add_direct(builder_ty, set_version_sym.clone());

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();

    let lookup = EmptyLookup::new()
        .with_type("Builder", "Builder")
        .with_return_type("Builder.setTitle", "this")
        .with_return_type("Builder.setVersion", "this");

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
            seg("Builder", SegmentKind::TypeAccess),
            seg("setTitle", SegmentKind::Property),
            seg("setVersion", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("setVersion");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("fluent chain resolves second method on preserved receiver");
    assert_eq!(result.target_symbol_id, 402);
}

#[test]
fn external_type_qname_promotion_finds_member_via_full_qname() {
    // current_ty resolved to the short name "Assertion" (e.g. from a
    // signature's bare return type), and the external chai package
    // owns "chai.Assertion". MembersIndex.lookup misses (externals are
    // filtered from the engine member set), the direct
    // by_qualified_name("Assertion.to") also misses, but the external
    // twin "chai.Assertion.to" should be found.
    let mut arena = TypeArena::new();
    let assertion_ty = arena.class("Assertion");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        100,
        SymbolTypeData {
            return_type: Some(assertion_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(assertion_ty, 100);

    // No MembersIndex entry for "Assertion" -> walker must fall back
    // through qualified_member_lookup, which exercises the external
    // promotion when the direct qname misses.
    let members = MembersIndex::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();

    let to_member = sym_info(101, "to", "chai.Assertion.to", "property", Some("chai.Assertion"));
    let lookup = EmptyLookup::new()
        .with_type("Assertion", "Assertion")
        .with_external_type("Assertion", "chai.Assertion")
        .with_qname_symbol("chai.Assertion.to", to_member.clone());

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
            seg("Assertion", SegmentKind::TypeAccess),
            seg("to", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let mut r = dummy_extracted_ref("to");
    r.kind = EdgeKind::Reads;
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("Assertion.to resolves via chai.Assertion external qname promotion");
    assert_eq!(result.target_symbol_id, 101);
}

#[test]
fn intermediate_yield_falls_back_to_lookup_field_type() {
    // 3-segment chain `repo.findOne.name` where `repo` resolves to Repo,
    // Repo.findOne has no SymbolTypeMap entry but SymbolLookup carries
    // field_type_name("Repo.findOne") = "User". Engine fallback should
    // intern "User" as a class and continue to the .name lookup.
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
    // Sym 3 (Repo.findOne field) intentionally has no SymbolTypeData —
    // simulating an extractor that doesn't populate declared_type yet.
    symbol_types.insert(
        4,
        SymbolTypeData {
            declared_type: Some(str_ty),
            ..Default::default()
        },
    );

    let mut members = MembersIndex::new();
    members.add_direct(
        repo_ty,
        sym_info(3, "findOne", "Repo.findOne", "field", Some("Repo")),
    );
    members.add_direct(
        user_ty,
        sym_info(4, "name", "User.name", "field", Some("User")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("Repo", "Repo")
        .with_field_type("Repo.findOne", "User");

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
            seg("findOne", SegmentKind::Property),
            seg("name", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let mut r = dummy_extracted_ref("name");
    r.kind = EdgeKind::Reads;
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("3-segment chain resolves via lookup fallback");
    assert_eq!(result.target_symbol_id, 4);
}

#[test]
fn last_segment_resolves_when_yield_type_missing() {
    // Receiver type Customer has a method ToViewModel whose return_type
    // is not registered in SymbolTypeMap. The walker should still resolve
    // the chain because the last segment's sym_id is the resolution
    // target — no downstream segment depends on the yield type.
    let mut arena = TypeArena::new();
    let customer_ty = arena.class("Customer");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        20,
        SymbolTypeData {
            return_type: Some(customer_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(customer_ty, 20);
    // Method 21 (ToViewModel) intentionally has no SymbolTypeData
    // entry — emulates C# extractor output where return_type isn't set.

    let mut members = MembersIndex::new();
    members.add_direct(
        customer_ty,
        sym_info(
            21,
            "ToViewModel",
            "Customer.ToViewModel",
            "method",
            Some("Customer"),
        ),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_type("Customer", "Customer");

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
            seg("Customer", SegmentKind::TypeAccess),
            seg("ToViewModel", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("ToViewModel");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("Customer.ToViewModel resolves even without method return type");
    assert_eq!(result.target_symbol_id, 21);
}

// ----------------------------------------------------------------------------
// CFG-derived Union narrowing flows through the chain walker
// ----------------------------------------------------------------------------

#[test]
fn cfg_union_narrowing_resolves_member_present_on_every_branch() {
    // The shape this proves end-to-end:
    //
    //   function f(x: unknown) {
    //     if (typeof x === "string" || typeof x === "number") {
    //       x.toString();   // x is string ∪ number — toString lives on both
    //     }
    //   }
    //
    // The CFG side already produces a `Fact::Union(["number","string"])` at
    // the probe byte (test `cfg_logical_or_unions_disagreeing_guards_on_true_edge`
    // pins that). This test drives the chain walker on a synthetic lookup
    // that returns the same union via `local_type_union`, asserting that the
    // walker builds a `Type::Union` root and the member arm in
    // `core/members.rs` finds `toString` on every branch.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let string_ty = arena.class("string");
    let number_ty = arena.class("number");

    let mut members = MembersIndex::new();
    members.add_direct(
        string_ty,
        sym_info(1, "toString", "string.toString", "method", Some("string")),
    );
    members.add_direct(
        number_ty,
        sym_info(2, "toString", "number.toString", "method", Some("number")),
    );

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_local_union("x", &["string", "number"]);

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
            seg("x", SegmentKind::Identifier),
            seg("toString", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("toString");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("toString present on every branch resolves through the union root");
    // The Union arm in members.rs requires every branch to carry the member
    // and returns the first branch's hit — `string` came first in the
    // builder, so id 1 (string.toString) wins.
    assert_eq!(result.target_symbol_id, 1);
}

#[test]
fn cfg_union_narrowing_drops_when_member_missing_on_a_branch() {
    // Sister case: `length` lives on string but not on number. The Union
    // arm in members.rs is conservative — any branch missing the member
    // fails the lookup, because at runtime the value could be on that
    // branch and the access would NPE. The chain walker resolves to None.
    use crate::type_checker::core::types::Type;

    let mut arena = TypeArena::new();
    let string_ty = arena.class("string");
    let _number_ty = arena.class("number");

    let mut members = MembersIndex::new();
    members.add_direct(
        string_ty,
        sym_info(1, "length", "string.length", "property", Some("string")),
    );
    // number has no `length` member — the Union lookup must drop.

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_local_union("x", &["string", "number"]);

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
            seg("x", SegmentKind::Identifier),
            seg("length", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("length");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(
        walker.walk(&chain, &ref_ctx, &fc).is_none(),
        "length is missing on the `number` branch — Union lookup refuses to bind"
    );
    // Touch the suppress-unused so a future grammar bump that prints the type
    // doesn't lose the explicit construction.
    let _ = Type::Union(vec![]);
}

// ----------------------------------------------------------------------------
// INFER-6 — built-in container generic semantics
//
// A chain past a container accessor (`arr.pop()`, `m.get()`) or a subscript
// (`arr[i]`) types THROUGH the container's element/value, so a following
// member resolves on the element. The element comes from the receiver's
// `Apply` args structurally; which accessor projects which slot is profile
// data (`container_accessors`), not a hardcoded method→element map.
// ----------------------------------------------------------------------------

use crate::type_checker::profile::language_profile::{
    AccessorSlot, ContainerShape,
};

/// A profile carrying the three built-in container accessors under test.
/// All other axes are DEFAULT_PROFILE's conservative values.
const CONTAINER_PROFILE: LanguageProfile = LanguageProfile {
    has_generics: true,
    container_accessors: &[
        ("pop", ContainerShape::Sequence, AccessorSlot::Element),
        ("get", ContainerShape::Map, AccessorSlot::Value),
    ],
    ..DEFAULT_PROFILE
};

// `Apply<Array,[User]>` root; `arr.pop()` must yield User so `.name` binds.
fn array_of_user(arena: &TypeArena) -> TypeId {
    let array_base = arena.class("Array");
    let user_ty = arena.class("User");
    arena.intern(Type::Apply {
        base: array_base,
        args: vec![user_ty],
    })
}

struct FixedTyRoot {
    ty: TypeId,
}
impl RootResolver for FixedTyRoot {
    fn resolve(
        &self,
        _seg: &ChainSegment,
        _ref_ctx: &RefContext,
        _file_ctx: &FileContext,
        _arena: &TypeArena,
        _lookup: &dyn SymbolLookup,
    ) -> Option<TypeId> {
        Some(self.ty)
    }
}

#[test]
fn array_pop_types_through_element() {
    // const arr: Array<User>;  arr.pop().name  → User.name. `Array` has no
    // project symbol `pop`; the container_accessors axis projects the Sequence
    // element (args[0] = User) and `.name` resolves on it.
    let mut arena = TypeArena::new();
    let arr_ty = array_of_user(&arena);
    let user_ty = arena.class("User");

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(7, "name", "User.name", "property", Some("User")),
    );

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &CONTAINER_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("arr", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("pop", SegmentKind::Property)
            },
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
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedTyRoot { ty: arr_ty })
        .expect("arr.pop() types through to the element so .name resolves");
    assert_eq!(result.target_symbol_id, 7);
}

#[test]
fn map_get_types_through_value() {
    // const m: Map<string, User>;  m.get(k).name  → User.name. The Map shape's
    // Value slot is args[1] = User.
    use crate::type_checker::core::types::Type;
    let mut arena = TypeArena::new();
    let map_base = arena.class("Map");
    let string_ty = arena.class("string");
    let user_ty = arena.class("User");
    let map_ty = arena.intern(Type::Apply {
        base: map_base,
        args: vec![string_ty, user_ty],
    });

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(8, "name", "User.name", "property", Some("User")),
    );

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &CONTAINER_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("m", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("get", SegmentKind::Property)
            },
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
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedTyRoot { ty: map_ty })
        .expect("m.get() types through to the Map value so .name resolves");
    assert_eq!(result.target_symbol_id, 8);
}

#[test]
fn computed_access_types_through_element() {
    // const arr: Array<User>;  arr[i].name  → User.name. The subscript
    // projects the Sequence element structurally (index value irrelevant for
    // Array). No container_accessors entry is needed for the subscript form.
    let mut arena = TypeArena::new();
    let arr_ty = array_of_user(&arena);
    let user_ty = arena.class("User");

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(9, "name", "User.name", "property", Some("User")),
    );

    let symbol_types = SymbolTypeMap::new();
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &CONTAINER_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("arr", SegmentKind::Identifier),
            seg("i", SegmentKind::ComputedAccess),
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
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedTyRoot { ty: arr_ty })
        .expect("arr[i] types through the element so .name resolves");
    assert_eq!(result.target_symbol_id, 9);
}

#[test]
fn empty_container_accessors_axis_leaves_chain_unchanged() {
    // With DEFAULT_PROFILE (empty container_accessors), `arr.pop().name` finds
    // no `pop` member on `Array` and projects nothing — the chain misses,
    // byte-identical to pre-INFER-6 behavior. Locks the axis as the only thing
    // that turns the projection on.
    let mut arena = TypeArena::new();
    let arr_ty = array_of_user(&arena);
    let user_ty = arena.class("User");

    let mut members = MembersIndex::new();
    members.add_direct(
        user_ty,
        sym_info(10, "name", "User.name", "property", Some("User")),
    );

    let symbol_types = SymbolTypeMap::new();
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
            seg("arr", SegmentKind::Identifier),
            ChainSegment {
                is_call: true,
                call_args: Vec::new(),
                ..seg("pop", SegmentKind::Property)
            },
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
    assert!(
        walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedTyRoot { ty: arr_ty })
            .is_none(),
        "no container_accessors entry → `pop` does not project → chain misses"
    );
}

#[test]
fn box_receiver_resolves_inner_type_method() {
    // let b: Box<C> = ...; b.method() — the receiver types as the inner C
    // (the single-inner wrapper peel), so `method` resolves on C's member,
    // not on Box (which carries no `method`).
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    // Box<C> interns as Apply{base:Class("Box"), args:[Class("C")]}.
    let box_ty = arena.intern_type_str("Box<C>");
    let c_ty = arena.class("C");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 99);

    let mut members = MembersIndex::new();
    members.add_direct(
        c_ty,
        sym_info(7, "method", "C.method", "method", Some("C")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("b", SegmentKind::Identifier),
            seg("method", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("method");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: box_ty })
        .expect("method resolves on the peeled inner C");
    assert_eq!(result.target_symbol_id, 7);
}

#[test]
fn pin_and_cow_receivers_resolve_inner_type_method() {
    // `Pin<C>` and `Cow<'a, C>` are pure-Deref single-inner wrappers (Pin derefs
    // to its pointee, Cow to its borrowed inner). Both must peel to C so a
    // `.method()` resolves on C. Cow's first generic arg is a LIFETIME — the
    // intern-time lifetime drop makes `Cow<'a, C>` a single-type-arg Apply so the
    // existing peel projects `args[0]` = C. Mirrors the Box case for the two
    // additional wrappers added to the Rust profile data.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 99);

    let mut members = MembersIndex::new();
    members.add_direct(
        c_ty,
        sym_info(7, "method", "C.method", "method", Some("C")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("v", SegmentKind::Identifier),
            seg("method", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("method");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();

    for spec in ["Pin<C>", "Cow<'a, C>"] {
        let wrapped = arena.intern_type_str(spec);
        let result = walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: wrapped })
            .unwrap_or_else(|| panic!("method resolves on the peeled inner C for {spec}"));
        assert_eq!(result.target_symbol_id, 7, "{spec} must peel to C");
    }
}

#[test]
fn box_field_receiver_resolves_inner_via_default_root() {
    // End-to-end through the REAL DefaultRootResolver scope-walk path: a
    // receiver `b` whose enclosing-scope declared type is `Box<C>` must intern
    // structurally as Apply{Box,[C]} so `peel_receiver_wrappers` fires and
    // `b.method()` binds C's member. Driving `walk()` (not a FixedRoot that
    // pre-interns the Apply) exercises the `field_type_name` root site, which
    // interned a flat `Class("Box<C>")` the peel could never decompose.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 99);

    let mut members = MembersIndex::new();
    members.add_direct(
        c_ty,
        sym_info(7, "method", "C.method", "method", Some("C")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // `b` is a field/local of enclosing scope `Scope`, declared type `Box<C>`.
    let lookup = EmptyLookup::new().with_field_type("Scope.b", "Box<C>");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("b", SegmentKind::Identifier),
            seg("method", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("method");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: vec!["Scope".to_string()],
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("method resolves on the peeled inner C via DefaultRootResolver");
    assert_eq!(result.target_symbol_id, 7);
}

#[test]
fn vec_receiver_does_not_peel() {
    // Vec is NOT in the single-inner wrapper list, so `Vec<C>.method` stays on
    // Vec — proving the peel is allowlist-gated, not a blanket arity-1 Apply
    // peel that would unsoundly skip every container.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let vec_ty = arena.intern_type_str("Vec<C>");
    let vec_base = arena.class("Vec");
    let c_ty = arena.class("C");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(vec_base, 88);

    let mut members = MembersIndex::new();
    // `method` lives on Vec; C has none. A peel to C would miss.
    members.add_direct(
        vec_base,
        sym_info(11, "method", "Vec.method", "method", Some("Vec")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new();

    struct FixedRoot {
        ty: TypeId,
    }
    impl RootResolver for FixedRoot {
        fn resolve(
            &self,
            _seg: &ChainSegment,
            _ref_ctx: &RefContext,
            _file_ctx: &FileContext,
            _arena: &TypeArena,
            _lookup: &dyn SymbolLookup,
        ) -> Option<TypeId> {
            Some(self.ty)
        }
    }
    let _ = c_ty;

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("v", SegmentKind::Identifier),
            seg("method", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("method");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRoot { ty: vec_ty })
        .expect("method resolves on Vec — no peel");
    assert_eq!(result.target_symbol_id, 11);
}

// Shared root resolver for the user-Deref peel tests below: each pins the
// receiver to a fixed `MyWrapper` Class TypeId.
struct DerefFixedRoot {
    ty: TypeId,
}
impl RootResolver for DerefFixedRoot {
    fn resolve(
        &self,
        _seg: &ChainSegment,
        _ref_ctx: &RefContext,
        _file_ctx: &FileContext,
        _arena: &TypeArena,
        _lookup: &dyn SymbolLookup,
    ) -> Option<TypeId> {
        Some(self.ty)
    }
}

#[test]
fn user_deref_receiver_resolves_inner_target_method() {
    // `let w: MyWrapper = ...; w.inner_method()` where `MyWrapper` is a USER
    // type with `impl Deref for MyWrapper { type Target = Inner }` and
    // `inner_method` lives on Inner. Rust's autoderef makes `w.inner_method()`
    // resolve to `Inner::inner_method`. The receiver peel must climb the user
    // Deref: a `MyWrapper → Deref` supertype edge (structural impl evidence)
    // plus the indexed `field_type["MyWrapper.Target"] = "Inner"` binding peel
    // the receiver to Inner before member lookup.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let wrapper_ty = arena.class("MyWrapper");
    let deref_ty = arena.class("Deref");
    let inner_ty = arena.class("Inner");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(inner_ty, 1);

    let mut members = MembersIndex::new();
    // `inner_method` lives on Inner; MyWrapper has none. Resolution requires the
    // Deref peel to Inner.
    members.add_direct(
        inner_ty,
        sym_info(7, "inner_method", "Inner.inner_method", "method", Some("Inner")),
    );

    let mut supertypes = SupertypeGraph::new();
    // The impl-container reroute forms `MyWrapper → Deref` for `impl Deref for
    // MyWrapper`.
    supertypes.add_edge(wrapper_ty, deref_ty);
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_field_type("MyWrapper.Target", "Inner");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("w", SegmentKind::Identifier),
            seg("inner_method", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("inner_method");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &DerefFixedRoot { ty: wrapper_ty })
        .expect("inner_method resolves on the peeled Deref target Inner");
    assert_eq!(result.target_symbol_id, 7);
}

#[test]
fn user_deref_peel_declines_without_deref_edge() {
    // A type with a `Target` assoc binding but NO `→ Deref` supertype edge is
    // not a Deref wrapper — the peel must not fire on a bare name coincidence.
    // Without the structural impl edge the receiver stays `Plain`, so a method
    // that only exists on the (unrelated) `Inner` cannot resolve.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let plain_ty = arena.class("Plain");
    let inner_ty = arena.class("Inner");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(inner_ty, 1);

    let mut members = MembersIndex::new();
    members.add_direct(
        inner_ty,
        sym_info(7, "inner_method", "Inner.inner_method", "method", Some("Inner")),
    );

    // No edge: Plain does NOT implement Deref.
    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_field_type("Plain.Target", "Inner");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("p", SegmentKind::Identifier),
            seg("inner_method", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("inner_method");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result =
        walker.walk_with_root(&chain, &ref_ctx, &fc, &DerefFixedRoot { ty: plain_ty });
    assert!(
        result.is_none(),
        "no Deref edge → no peel → inner_method must not resolve"
    );
}

#[test]
fn user_deref_peel_inert_under_default_profile() {
    // The peel is gated on `LanguageProfile::deref_wrapper`; the default profile
    // sets it to None, so even with a `→ Deref` edge and a `Target` binding the
    // receiver is NOT peeled — every non-Rust language is byte-identical.
    use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

    let arena = TypeArena::new();
    let wrapper_ty = arena.class("MyWrapper");
    let deref_ty = arena.class("Deref");
    let inner_ty = arena.class("Inner");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(inner_ty, 1);

    let mut members = MembersIndex::new();
    members.add_direct(
        inner_ty,
        sym_info(7, "inner_method", "Inner.inner_method", "method", Some("Inner")),
    );

    let mut supertypes = SupertypeGraph::new();
    supertypes.add_edge(wrapper_ty, deref_ty);
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new().with_field_type("MyWrapper.Target", "Inner");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("w", SegmentKind::Identifier),
            seg("inner_method", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("inner_method");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result =
        walker.walk_with_root(&chain, &ref_ctx, &fc, &DerefFixedRoot { ty: wrapper_ty });
    assert!(
        result.is_none(),
        "default profile has no deref_wrapper → no peel → no resolution"
    );
}

struct FixedRootTy {
    ty: TypeId,
}
impl RootResolver for FixedRootTy {
    fn resolve(
        &self,
        _seg: &ChainSegment,
        _ref_ctx: &RefContext,
        _file_ctx: &FileContext,
        _arena: &TypeArena,
        _lookup: &dyn SymbolLookup,
    ) -> Option<TypeId> {
        Some(self.ty)
    }
}

#[test]
fn associated_type_self_output_projects_to_impl_binding() {
    // `c.add().finish()` where `C.add` returns `Self::Output` and the impl
    // binds `type Output = Concrete` (field_type "C.Output" = "Concrete").
    // yield_type_of must project Self::Output through the receiver C to
    // Concrete so the next segment resolves Concrete.finish.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let concrete_ty = arena.class("Concrete");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(concrete_ty, 2);

    let mut members = MembersIndex::new();
    // C.add: a method whose return string is the associated-type projection.
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    // Concrete.finish: only reachable if the chain typed past add() to Concrete.
    members.add_direct(
        concrete_ty,
        sym_info(9, "finish", "Concrete.finish", "method", Some("Concrete")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Concrete", "Concrete")
        .with_return_type("C.add", "Self::Output")
        .with_field_type("C.Output", "Concrete");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("finish", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("finish");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
        .expect("Self::Output projects through C.Output to Concrete; finish resolves");
    assert_eq!(result.target_symbol_id, 9);
}

#[test]
fn associated_type_projection_declines_when_no_impl_binding() {
    // Same chain, but NO `C.Output` impl binding. The projection must NOT fire
    // on a coincidental name match — Self::Output interns as an opaque Class
    // with no `finish` member, so the walk declines (widening-only soundness).
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let concrete_ty = arena.class("Concrete");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(concrete_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        concrete_ty,
        sym_info(9, "finish", "Concrete.finish", "method", Some("Concrete")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // No `C.Output` field_type — the binding is absent.
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Concrete", "Concrete")
        .with_return_type("C.add", "Self::Output");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("finish", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("finish");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(
        walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
            .is_none(),
        "no impl binding => no projection => walk declines"
    );
}

#[test]
fn associated_type_projection_off_by_default_profile() {
    // The projection is profile-gated (`associated_type_projection`). Under
    // DEFAULT_PROFILE the flag is false, so Self::Output is NOT rewritten even
    // with the impl binding present — the next segment cannot resolve.
    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let concrete_ty = arena.class("Concrete");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(concrete_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        concrete_ty,
        sym_info(9, "finish", "Concrete.finish", "method", Some("Concrete")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Concrete", "Concrete")
        .with_return_type("C.add", "Self::Output")
        .with_field_type("C.Output", "Concrete");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &DEFAULT_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("finish", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("finish");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(
        walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
            .is_none(),
        "default profile keeps associated_type_projection off"
    );
}

#[test]
fn associated_type_projection_ignores_plain_module_path_return() {
    // A method returning `module::Foo` (a real path, not an assoc projection)
    // must NOT be hijacked into projecting through `C.Foo`: there is no such
    // impl binding, so the `::`-split lookup misses and the raw string interns
    // unchanged as `Class("module::Foo")`. The projection only ever fires when
    // the `{receiver}.{last}` binding HITS — a real path return declines, never
    // mis-binds. A `C.Foo` binding to `Hijacked` is present to PROVE the split
    // does not key on the path's last segment when the receiver is the path's
    // module-qualified return rather than an assoc projection of C.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let hijacked_ty = arena.class("Hijacked");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(hijacked_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "make", "C.make", "method", Some("C")));
    // If the projection wrongly keyed on the LAST `::` segment of the return
    // (`Foo`) it would project `C.Foo` -> Hijacked and `go` would bind id 13.
    members.add_direct(
        hijacked_ty,
        sym_info(13, "go", "Hijacked.go", "method", Some("Hijacked")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Hijacked", "Hijacked")
        .with_return_type("C.make", "module::Foo")
        .with_field_type("C.Foo", "Hijacked");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("make", SegmentKind::Property),
            seg("go", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("go");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(
        walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
            .is_none(),
        "module::Foo is a path return, not an assoc projection of C — no hijack"
    );
}

#[test]
fn associated_type_angle_self_qualified_projects_to_impl_binding() {
    // `c.add().finish()` where `C.add` returns the fully-qualified angle form
    // `<Self as Trait>::Output`. The inner concrete of the angle head is the
    // `Self` keyword, which must resolve to the receiver C — keying the impl
    // binding `field_type["C.Output"] = "Concrete"` — exactly like the bare
    // `Self::Output` form. Without resolving `Self` to C, the head keys the
    // never-present `Self.Output` and the projection silently declines.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let concrete_ty = arena.class("Concrete");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(concrete_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        concrete_ty,
        sym_info(9, "finish", "Concrete.finish", "method", Some("Concrete")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Concrete", "Concrete")
        .with_return_type("C.add", "<Self as Trait>::Output")
        .with_field_type("C.Output", "Concrete");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("finish", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("finish");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
        .expect("<Self as Trait>::Output projects through C.Output to Concrete");
    assert_eq!(result.target_symbol_id, 9);
}

#[test]
fn associated_type_angle_concrete_qualified_projects_when_head_is_receiver() {
    // The angle head names the concrete receiver directly: `<C as Trait>::Output`.
    // The inner concrete `C` equals the receiver qname, so the binding keys
    // `C.Output` and the projection fires. (The trait is disambiguating context
    // only; the binding keys on C.)
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let concrete_ty = arena.class("Concrete");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(concrete_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        concrete_ty,
        sym_info(9, "finish", "Concrete.finish", "method", Some("Concrete")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Concrete", "Concrete")
        .with_return_type("C.add", "<C as Trait>::Output")
        .with_field_type("C.Output", "Concrete");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("finish", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("finish");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
        .expect("<C as Trait>::Output projects through C.Output to Concrete");
    assert_eq!(result.target_symbol_id, 9);
}

#[test]
fn associated_type_angle_unrelated_concrete_head_declines() {
    // The angle head names a DIFFERENT concrete type than the receiver:
    // `<Other as Trait>::Output`. The inner concrete `Other` is neither a
    // self-keyword nor the receiver qname C, so the projection keys `Other.Output`
    // (a binding genuinely about `Other`, not C). With no `Other.Output` binding
    // present the projection declines — it must NOT silently key `C.Output`
    // and bind a member that belongs to a different type's associated type.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let concrete_ty = arena.class("Concrete");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(concrete_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        concrete_ty,
        sym_info(9, "finish", "Concrete.finish", "method", Some("Concrete")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // `C.Output` IS present; the head names `Other`, so projecting through
    // `C.Output` would be a hijack. The correct key is `Other.Output` (absent).
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Concrete", "Concrete")
        .with_return_type("C.add", "<Other as Trait>::Output")
        .with_field_type("C.Output", "Concrete");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("finish", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("finish");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(
        walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
            .is_none(),
        "<Other as Trait>::Output keys Other.Output (absent) — no hijack to C.Output"
    );
}

#[test]
fn associated_type_chained_projection_resolves_assoc_of_assoc() {
    // `c.add().leaf()` where `C.add` returns the CHAINED associated form
    // `Self::Output::Item`. Two hops: the impl binds `type Output = Mid`
    // (field_type "C.Output" = "Mid") and `Mid` in turn binds `type Item = Leaf`
    // (field_type "Mid.Item" = "Leaf"). The projection must resolve the head
    // `Self::Output` to `Mid` first, then project `::Item` off `Mid` to `Leaf`,
    // so the next segment resolves `Leaf.leaf`.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let leaf_ty = arena.class("Leaf");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(leaf_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        leaf_ty,
        sym_info(9, "leaf", "Leaf.leaf", "method", Some("Leaf")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Leaf", "Leaf")
        .with_return_type("C.add", "Self::Output::Item")
        // First hop: C's `type Output = Mid`.
        .with_field_type("C.Output", "Mid")
        // Second hop: Mid's `type Item = Leaf`.
        .with_field_type("Mid.Item", "Leaf");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("leaf", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("leaf");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
        .expect("Self::Output::Item projects C.Output->Mid then Mid.Item->Leaf; leaf resolves");
    assert_eq!(result.target_symbol_id, 9);
}

#[test]
fn associated_type_chained_projection_declines_when_inner_binding_absent() {
    // Same chained `Self::Output::Item` return, but the FIRST hop binding
    // (`C.Output`) is absent. The head `Self::Output` resolves to no concrete
    // type, so the outer `::Item` projection has no receiver to key — the whole
    // chained projection declines (widening-only: a missing inner hop never
    // silently keys a coincidental `Self.Item`/bare `Item`).
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let leaf_ty = arena.class("Leaf");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(leaf_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        leaf_ty,
        sym_info(9, "leaf", "Leaf.leaf", "method", Some("Leaf")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // `C.Output` ABSENT; only the second-hop binding is present. The head must
    // not resolve, so `Mid.Item` is never consulted.
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Leaf", "Leaf")
        .with_return_type("C.add", "Self::Output::Item")
        .with_field_type("Mid.Item", "Leaf");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("leaf", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("leaf");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    assert!(
        walker
            .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
            .is_none(),
        "missing inner hop (C.Output) => head unresolved => chained projection declines"
    );
}

#[test]
fn associated_type_chained_angle_projection_resolves_assoc_of_assoc() {
    // The chained head is the angle-qualified form: `<C as Trait>::Output::Item`.
    // The inner head `<C as Trait>::Output` resolves through the receiver C's
    // `Output` binding to `Mid`, then `::Item` projects off `Mid` to `Leaf`.
    use crate::languages::rust_lang::profile::RUST_PROFILE;

    let arena = TypeArena::new();
    let c_ty = arena.class("C");
    let leaf_ty = arena.class("Leaf");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.mark_self_yielding(c_ty, 1);
    symbol_types.mark_self_yielding(leaf_ty, 2);

    let mut members = MembersIndex::new();
    members.add_direct(c_ty, sym_info(5, "add", "C.add", "method", Some("C")));
    members.add_direct(
        leaf_ty,
        sym_info(9, "leaf", "Leaf.leaf", "method", Some("Leaf")),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    let lookup = EmptyLookup::new()
        .with_type("C", "C")
        .with_type("Leaf", "Leaf")
        .with_return_type("C.add", "<C as Trait>::Output::Item")
        .with_field_type("C.Output", "Mid")
        .with_field_type("Mid.Item", "Leaf");

    let walker = ChainWalker::new(
        &arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &RUST_PROFILE,
        &lookup,
    );
    let chain = MemberChain {
        segments: vec![
            seg("c", SegmentKind::Identifier),
            seg("add", SegmentKind::Property),
            seg("leaf", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("leaf");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let fc = file_ctx();
    let result = walker
        .walk_with_root(&chain, &ref_ctx, &fc, &FixedRootTy { ty: c_ty })
        .expect("<C as Trait>::Output::Item projects C.Output->Mid then Mid.Item->Leaf");
    assert_eq!(result.target_symbol_id, 9);
}

// ---------------------------------------------------------------------------
// Wildcard-import-prefix ROOT promotion (.NET `using NS;` / `Imports NS`)
// ---------------------------------------------------------------------------

/// Mirrors the C#/VB chain-qualification axis: bare receivers promote to a
/// package-qualified qname via same-package and import sources. All other
/// axes are DEFAULT_PROFILE's conservative values.
const SAME_PACKAGE_PROFILE: LanguageProfile = LanguageProfile {
    chain_qualification: ChainQualification::SamePackageAndImports,
    ..DEFAULT_PROFILE
};

#[test]
fn wildcard_using_promotes_root_receiver_to_fqn() {
    // C# `using System.Windows.Controls;` + chain `Button.Show()`. The bare
    // `Button` root self-yields, but its member `Show` is keyed ONLY under
    // the fully-qualified receiver `System.Windows.Controls.Button`. The
    // member resolves only if the wildcard import promotes the root receiver
    // `Button` -> `System.Windows.Controls.Button` BEFORE member lookup.
    let mut arena = TypeArena::new();
    let button_ty = arena.class("Button");
    let fqn_ty = arena.class("System.Windows.Controls.Button");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        200,
        SymbolTypeData {
            return_type: Some(button_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(button_ty, 200);

    let mut members = MembersIndex::new();
    members.add_direct(
        fqn_ty,
        sym_info(
            201,
            "Show",
            "System.Windows.Controls.Button.Show",
            "method",
            Some("System.Windows.Controls.Button"),
        ),
    );

    let supertypes = SupertypeGraph::new();
    let aliases = AliasIndex::default();
    // The promotion guard `keys_a_type_or_member` consults by_qualified_name;
    // the FQN type symbol is what makes the wildcard-prefix candidate key.
    // Deliberately NOT registered on any bare-name path.
    let lookup = EmptyLookup::new().with_qname_symbol(
        "System.Windows.Controls.Button",
        sym_info(
            300,
            "Button",
            "System.Windows.Controls.Button",
            "class",
            None,
        ),
    );

    let mut walker = ChainWalker::new(
        &mut arena,
        &members,
        &supertypes,
        &symbol_types,
        &aliases,
        &SAME_PACKAGE_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("Button", SegmentKind::TypeAccess),
            seg("Show", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("Show");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let mut fc = file_ctx();
    fc.imports.push(crate::indexer::resolve::engine::ImportEntry {
        imported_name: "System.Windows.Controls".to_string(),
        module_path: Some("System.Windows.Controls".to_string()),
        alias: None,
        is_wildcard: true,
    });

    let result = walker
        .walk(&chain, &ref_ctx, &fc)
        .expect("Button.Show resolves via wildcard-using root promotion");
    assert_eq!(result.target_symbol_id, 201);
}

#[test]
fn wildcard_using_promotes_only_when_fqn_keys() {
    // Widening guard: same setup but NOTHING keyed under
    // `System.Windows.Controls.Button` (no member, no type symbol), so
    // `keys_a_type_or_member` is false. The root stays bare `Button`,
    // `Show` is unknown, and the walk declines — no false 1.0 bind from a
    // wildcard namespace that doesn't actually export the type.
    let mut arena = TypeArena::new();
    let button_ty = arena.class("Button");

    let mut symbol_types = SymbolTypeMap::new();
    symbol_types.insert(
        200,
        SymbolTypeData {
            return_type: Some(button_ty),
            ..Default::default()
        },
    );
    symbol_types.mark_self_yielding(button_ty, 200);

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
        &SAME_PACKAGE_PROFILE,
        &lookup,
    );

    let chain = MemberChain {
        segments: vec![
            seg("Button", SegmentKind::TypeAccess),
            seg("Show", SegmentKind::Property),
        ],
    };
    let source = dummy_source_symbol("caller", None);
    let r = dummy_extracted_ref("Show");
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &source,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    let mut fc = file_ctx();
    fc.imports.push(crate::indexer::resolve::engine::ImportEntry {
        imported_name: "System.Windows.Controls".to_string(),
        module_path: Some("System.Windows.Controls".to_string()),
        alias: None,
        is_wildcard: true,
    });

    assert!(
        walker.walk(&chain, &ref_ctx, &fc).is_none(),
        "no FQN keyed → no promotion → no false bind"
    );
}
