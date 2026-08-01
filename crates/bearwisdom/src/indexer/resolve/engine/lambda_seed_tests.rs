use std::cell::RefCell;

use super::*;
use crate::indexer::resolve::engine::contract::{SymbolSet, Symbol as ContractSymbol};
use crate::type_checker::core::types::Type;

/// Minimal lookup: generic parameters for one owner, plus a recording local
/// cache so a seeded lambda parameter is observable.
struct SeedLookup {
    owner: String,
    params: Vec<String>,
    empty: Vec<ContractSymbol>,
    empty_pairs: Vec<(String, String)>,
    seeded: RefCell<Vec<(String, TypeId)>>,
}

impl SeedLookup {
    fn new(owner: &str, params: &[&str]) -> Self {
        Self {
            owner: owner.to_string(),
            params: params.iter().map(|s| (*s).to_string()).collect(),
            empty: Vec::new(),
            empty_pairs: Vec::new(),
            seeded: RefCell::new(Vec::new()),
        }
    }
}

impl SymbolLookup for SeedLookup {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&ContractSymbol> {
        None
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&ContractSymbol> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn generic_params(&self, type_name: &str) -> Option<Vec<String>> {
        (type_name == self.owner).then(|| self.params.clone())
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_pairs
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn record_local_type_id(&self, name: String, id: TypeId) {
        self.seeded.borrow_mut().push((name, id));
    }
    fn record_local_type(&self, _: String, _: String) {}
}

/// `map(fn: (value: T) => U): Array<U>` declared on `Array<T>`.
fn map_method() -> Symbol {
    Symbol {
        signature: Some("map(fn: (value: T) => U): Array<U>".to_string()),
        ..crate::indexer::resolve::engine::testkit::sym(1, "map", "Array.map", "method", "a.ts")
    }
}

fn array_of(arena: &TypeArena, elem: &str) -> TypeId {
    arena.intern(Type::Apply {
        base: arena.class("Array"),
        args: vec![arena.class(elem)],
    })
}

#[test]
fn a_lambda_parameter_is_seeded_from_the_receivers_element_type() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["x".to_string()],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
    );

    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].0, "x");
    assert_eq!(arena.get(seeded[0].1), Type::Class("User".to_string()));
}

#[test]
fn an_unbindable_receiver_seeds_nothing() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["x".to_string()],
    }];

    // Bare `Array` — no applied argument, so the callback parameter stays `T`.
    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        arena.class("Array"),
        None,
        &FxHashMap::default(),
    );

    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn an_unnamed_lambda_binding_is_skipped() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    // A destructuring binding the extractor could not name.
    let args = vec![CallArg::Lambda {
        params: vec![String::new()],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
    );

    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn a_call_with_no_lambda_argument_seeds_nothing() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Ident("cb".to_string())];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
    );

    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn an_argument_driven_binding_reaches_the_callback_parameter() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["x".to_string()],
    }];
    // The receiver left `T` open; a sibling argument pinned it.
    let mut arg_env: FxHashMap<String, TypeId> = FxHashMap::default();
    arg_env.insert("T".to_string(), arena.class("Account"));

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        arena.class("Array"),
        None,
        &arg_env,
    );

    let seeded = lookup.seeded.borrow();
    assert_eq!(arena.get(seeded[0].1), Type::Class("Account".to_string()));
}
