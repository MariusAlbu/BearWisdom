use super::*;
use crate::indexer::resolve::engine::{
    self,
    chain::Receiver,
    contract::{
        generic_return::{substitute, GenericReturn},
        member_applicability::{expand, ReceiverPattern},
        FlowCacheLookup, Symbol, SymbolSet, TypeInfo,
    },
    member_selection::{self, Selection},
    testkit::{sym, Lookup},
};
use crate::type_checker::core::types::{GenericParamData, NominalContextId, PrimKind, Type};
use rustc_hash::FxHashMap;

struct Scoped {
    inner: Lookup,
    context: Option<NominalContextId>,
    info: FxHashMap<i64, TypeInfo>,
    denied: Option<i64>,
}
impl FlowCacheLookup for Scoped {
    fn nominal_context(&self) -> Option<NominalContextId> {
        self.context
    }
    fn declaration_accessible(&self, id: i64) -> bool {
        self.denied != Some(id)
    }
}
impl SymbolLookup for Scoped {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        panic!("name recovery is not evidence")
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> {
        panic!("qname recovery is not evidence")
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        panic!("display member lookup")
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        panic!("display type lookup")
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
        vec![]
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        panic!("display field type")
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        panic!("display return type")
    }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> {
        panic!("display generic params")
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &[]
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn symbol_by_id(&self, id: i64) -> Option<&Symbol> {
        self.inner.symbol_by_id(id)
    }
    fn member_index(&self) -> Option<&engine::member_index::MemberIndex> {
        self.inner.member_index()
    }
    fn members_of_id(&self, id: i64) -> SymbolSet<'_> {
        self.inner.members_of_id(id)
    }
    fn canonical_type_info(&self, id: i64) -> Option<&TypeInfo> {
        self.info.get(&id)
    }
    fn return_type_id_of(&self, id: i64) -> Option<TypeId> {
        self.info.get(&id)?.return_type_id
    }
    fn field_type_id_of(&self, id: i64) -> Option<TypeId> {
        self.info.get(&id)?.field_type_id
    }
    fn canonical_decl_id(&self, id: i64) -> i64 {
        if id == 11 {
            10
        } else {
            id
        }
    }
}
fn fixture(context: NominalContextId) -> Scoped {
    Scoped {
        context: Some(context),
        denied: None,
        info: FxHashMap::default(),
        inner: Lookup::new()
            .with(sym(10, "Poison", "Poison", "interface", "shared.d.ts"))
            .with(sym(11, "Poison", "Poison", "interface", "extra.d.ts"))
            .with(sym(20, "Alias", "Alias", "type_alias", "shared.d.ts"))
            .with_member_id(10, sym(30, "get", "Poison.get", "method", "extra.d.ts")),
    }
}

#[test]
fn source_bound_type_construction_is_separate_from_member_use_site_access() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let mut scoped = fixture(a);
    scoped.denied = Some(10);
    let lookup: &dyn SymbolLookup = &scoped;
    let own = lookup.declaration_type(&arena, 10).unwrap();
    let name = scoped.member_index().unwrap().name("get").unwrap();
    assert_eq!(
        member_selection::select_typed(lookup, &arena, Receiver::new(own, 10), name, &|_| true),
        Selection::Inaccessible
    );
}

#[test]
fn configured_member_yields_never_borrow_same_named_or_foreign_signatures() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let mut scoped = fixture(a);
    let own = arena.decl_in(a, "Poison", 10);
    let foreign = arena.decl_in(b, "Poison", 10);
    let member = scoped.symbol_by_id(30).unwrap().clone();
    assert!(engine::type_slots::return_type_by_identity(&scoped, &member).is_none());
    assert!(engine::type_slots::field_type_by_identity(&scoped, &member).is_none());
    for ty in [None, Some(foreign), Some(own)] {
        scoped.info.insert(
            30,
            TypeInfo {
                return_type_id: ty,
                ..Default::default()
            },
        );
        let yielded = engine::chain::member_yield_type(&scoped, &arena, &member, true).unwrap();
        assert_eq!(
            yielded,
            if ty == Some(own) {
                own
            } else {
                arena.intern(Type::Unknown)
            }
        );
    }
    let callable = arena.intern(Type::Function {
        params: vec![],
        return_: own,
    });
    scoped.info.insert(
        30,
        TypeInfo {
            field_type_id: Some(callable),
            ..Default::default()
        },
    );
    assert_eq!(
        engine::chain::member_yield_type(&scoped, &arena, &member, true),
        Some(callable)
    );
}

#[test]
fn only_the_selected_program_can_interpret_a_bound_nominal_receiver() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let scoped = fixture(a);
    let lookup: &dyn SymbolLookup = &scoped;
    let own = lookup.declaration_type(&arena, 10).unwrap();
    assert_eq!(
        lookup.declaration_type(&arena, 11),
        Some(own),
        "attested canonicalization is within this view"
    );
    let foreign = arena.decl_in(b, "Poison", 10);
    let legacy = arena.decl("Poison", 10);
    let member = scoped.member_index().unwrap().name("get").unwrap();
    for ty in [own, foreign, legacy] {
        let expected = (ty == own).then_some(10);
        assert_eq!(
            engine::head_decl::head_symbol_id(&arena, lookup, ty, None),
            expected
        );
        assert_eq!(
            engine::head_decl::head_symbol_id_preferring_package(&arena, lookup, ty, None),
            expected
        );
        let receiver = Receiver::new(ty, 10);
        assert_eq!(
            member_selection::select_typed(lookup, &arena, receiver, member, &|_| true),
            if ty == own {
                Selection::Unique(30)
            } else {
                Selection::Incomplete
            }
        );
        for id in [None, Some(10)] {
            assert_eq!(
                engine::chain::lookup_member_on_bounded(
                    lookup,
                    &arena,
                    Receiver { ty, id },
                    "get",
                    &|_| true,
                    4
                )
                .map(|s| s.id),
                (ty == own).then_some(30)
            );
        }
    }
    let unconfigured: &dyn SymbolLookup = &scoped.inner;
    assert!(engine::head_decl::head_symbol_id(&arena, unconfigured, own, None).is_none());
    assert!(engine::chain::lookup_member_on_bounded(
        unconfigured,
        &arena,
        Receiver::new(own, 10),
        "get",
        &|_| true,
        4
    )
    .is_none());
}

#[test]
fn alias_templates_and_generic_receiver_bindings_stay_in_their_program() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let mut scoped = fixture(a);
    let param = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 10,
        bound: None,
    });
    let own = arena.decl_in(a, "Poison", 10);
    let foreign = arena.decl_in(b, "Poison", 10);
    let alias = arena.decl_in(a, "Alias", 20);
    let scalar = arena.primitive(PrimKind::Bool);
    scoped.info.insert(
        10,
        TypeInfo {
            generic_param_ids: vec![param],
            ..Default::default()
        },
    );
    scoped.info.insert(
        20,
        TypeInfo {
            lexical_alias: Some(GenericReturn::bound(vec![], vec![], own)),
            ..Default::default()
        },
    );
    let lookup: &dyn SymbolLookup = &scoped;
    assert_eq!(expand(lookup, &arena, alias), Some(own));
    assert_eq!(engine::alias::expand(alias, lookup, &arena), own);
    assert!(expand(lookup, &arena, arena.decl_in(b, "Alias", 20)).is_none());
    for base in [own, foreign] {
        let ty = arena.intern(Type::Apply {
            base,
            args: vec![scalar],
        });
        let bindings = engine::bound_call::receiver_bindings(lookup, &arena, ty, Some(10));
        assert_eq!(bindings.get(&param), (base == own).then_some(&scalar));
    }
    let own_pattern = ReceiverPattern {
        ty: own,
        parameters: vec![],
    };
    assert!(matches!(
        own_pattern.bindings(lookup, &arena, own),
        Ok(Some(_))
    ));
    assert!(own_pattern.bindings(lookup, &arena, foreign).is_err());
    let foreign_pattern = ReceiverPattern {
        ty: foreign,
        parameters: vec![],
    };
    assert!(foreign_pattern.bindings(lookup, &arena, own).is_err());
    scoped.info.get_mut(&20).unwrap().lexical_alias =
        Some(GenericReturn::bound(vec![], vec![], foreign));
    assert!(
        expand(&scoped, &arena, alias).is_none(),
        "a same-context alias cannot launder a foreign target"
    );
}

#[test]
fn nested_foreign_arguments_are_not_permission_to_read_the_correct_outer_owner() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let scoped = fixture(a);
    let lookup: &dyn SymbolLookup = &scoped;
    let own = lookup.declaration_type(&arena, 10).unwrap();
    let foreign = arena.decl_in(b, "Poison", 10);
    let mixed = arena.intern(Type::Apply {
        base: own,
        args: vec![foreign],
    });
    let name = scoped.member_index().unwrap().name("get").unwrap();
    assert_eq!(
        member_selection::select_typed(lookup, &arena, Receiver::new(mixed, 10), name, &|_| true),
        Selection::Incomplete
    );
    assert!(expand(lookup, &arena, mixed).is_none());
    assert!(matches!(
        arena.get(engine::alias::expand(mixed, lookup, &arena)),
        Type::Unknown
    ));
    let projected = engine::chain::expand_receiver(Receiver::new(mixed, 10), lookup, &arena, None);
    assert!(matches!(arena.get(projected.ty), Type::Unknown));
    assert!(projected.id.is_none());
}

#[test]
fn substitution_preserves_context_but_does_not_sanction_cross_program_arguments() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let scoped = fixture(a);
    let lookup: &dyn SymbolLookup = &scoped;
    let parameter = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 10,
        bound: None,
    });
    let own = lookup.declaration_type(&arena, 10).unwrap();
    let applied = arena.intern(Type::Apply {
        base: own,
        args: vec![arena.generic_type(parameter)],
    });
    let foreign = arena.decl_in(b, "Poison", 10);
    let result = substitute(
        &arena,
        applied,
        &[(parameter, foreign)].into_iter().collect(),
    );
    assert!(matches!(arena.get(result), Type::Apply { base, .. } if base == own));
    assert!(!lookup.accepts_type_context(&arena, result));
}
