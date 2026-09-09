use super::super::{
    contract::{flow_cache::FlowCacheLookup, Symbol, SymbolSet, TypeInfo},
    testkit::sym,
};
use super::*;
use crate::type_checker::core::types::{GenericParamData, Type};
use rustc_hash::FxHashMap;

#[derive(Default)]
struct Lookup {
    symbols: FxHashMap<i64, Symbol>,
    info: FxHashMap<i64, TypeInfo>,
    members: FxHashMap<i64, Vec<Symbol>>,
    parents: FxHashMap<i64, Vec<i64>>,
    args: FxHashMap<(i64, i64), Vec<TypeId>>,
}
impl FlowCacheLookup for Lookup {}
impl SymbolLookup for Lookup {
    fn field_type_name(&self, _: &str) -> Option<&str> {
        panic!("no field text")
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        panic!("no return text")
    }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> {
        panic!("no parameter text")
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        panic!("no export text")
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        panic!("no external text")
    }
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        panic!("no name lookup")
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> {
        panic!("no qualified lookup")
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        panic!("no name member lookup")
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        panic!("no type name lookup")
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
        panic!("no namespace lookup")
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        panic!("no namespace lookup")
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        panic!("no file lookup")
    }
    fn symbol_by_id(&self, id: i64) -> Option<&Symbol> {
        self.symbols.get(&id)
    }
    fn canonical_type_info(&self, id: i64) -> Option<&TypeInfo> {
        self.info.get(&id)
    }
    fn members_of_id(&self, id: i64) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.members.get(&id).map(Vec::as_slice).unwrap_or(&[]))
    }
    fn parent_class_ids(&self, id: i64) -> Vec<i64> {
        self.parents.get(&id).cloned().unwrap_or_default()
    }
    fn parent_class_arg_ids_of(&self, child: i64, parent: i64) -> &[TypeId] {
        self.args
            .get(&(child, parent))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

fn fixture(arena: &TypeArena) -> Lookup {
    let mut lookup = Lookup::default();
    for id in 1..=4 {
        lookup
            .symbols
            .insert(id, sym(id, "Same", "Same", "class", "a.ts"));
        let p = arena.intern_generic(GenericParamData {
            name: "T".into(),
            owner_symbol_index: id as usize,
            bound: None,
            kind: Default::default(),
        });
        lookup.info.insert(
            id,
            TypeInfo {
                generic_param_ids: vec![p],
                base_type_id: Some(arena.intern(Type::Unknown)),
                ..Default::default()
            },
        );
    }
    lookup
        .members
        .insert(3, vec![sym(30, "next", "Same.next", "method", "b.ts")]);
    lookup.parents.insert(1, vec![2]);
    lookup.parents.insert(2, vec![3]);
    for (child, parent) in [(1, 2), (2, 3)] {
        lookup.args.insert(
            (child, parent),
            vec![arena.generic_type(lookup.info[&child].generic_param_ids[0])],
        );
    }
    lookup
}

#[test]
fn identical_display_names_do_not_skip_numeric_generic_composition() {
    let arena = TypeArena::new();
    let lookup = fixture(&arena);
    let doc = arena.decl("Same", 99);
    let receiver = arena.intern(Type::Apply {
        base: arena.decl("Same", 1),
        args: vec![doc],
    });
    let env = for_member(&lookup, &arena, receiver, Some(1), 30).unwrap();
    let parent_param = lookup.info[&3].generic_param_ids[0];
    assert_eq!(env[&parent_param], doc);
    assert!(!env.contains_key(&lookup.info[&1].generic_param_ids[0]));
    let yielded = arena.generic_type(parent_param);
    let member = &lookup.members[&3][0];
    assert_eq!(
        super::super::substitution::substitute_supertype_args(
            &lookup,
            &arena,
            member,
            yielded,
            receiver,
            Some(1)
        ),
        doc
    );
}

#[test]
fn missing_cyclic_or_inconsistent_paths_cannot_guess_an_instantiation() {
    let arena = TypeArena::new();
    let mut lookup = fixture(&arena);
    let doc = arena.decl("Doc", 99);
    let receiver = arena.intern(Type::Apply {
        base: arena.decl("Same", 1),
        args: vec![doc],
    });
    lookup.parents.insert(2, vec![1]);
    assert!(for_member(&lookup, &arena, receiver, Some(1), 30).is_none());
    lookup.parents.insert(2, vec![3]);
    lookup.symbols.remove(&3);
    assert!(for_member(&lookup, &arena, receiver, Some(1), 30).is_none());
    lookup
        .symbols
        .insert(3, sym(3, "Same", "Same", "class", "a.ts"));
    lookup.parents.insert(1, vec![2, 4]);
    lookup.parents.insert(4, vec![3]);
    lookup.args.insert((1, 4), vec![arena.decl("Doc", 100)]);
    lookup.args.insert(
        (4, 3),
        vec![arena.generic_type(lookup.info[&4].generic_param_ids[0])],
    );
    assert!(
        for_member(&lookup, &arena, receiver, Some(1), 30).is_none(),
        "conflicting diamond applications"
    );
    lookup.args.insert((1, 4), vec![doc]);
    assert_eq!(
        for_member(&lookup, &arena, receiver, Some(1), 30).unwrap()
            [&lookup.info[&3].generic_param_ids[0]],
        doc
    );
    lookup.args.remove(&(2, 3));
    assert!(
        for_member(&lookup, &arena, receiver, Some(1), 30).is_none(),
        "missing generic edge arguments"
    );
}
