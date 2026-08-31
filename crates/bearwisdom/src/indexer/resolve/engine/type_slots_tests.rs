use std::sync::Arc;

use super::{field_type_by_identity, return_type_by_identity};
use crate::indexer::resolve::engine::contract::{Symbol as ContractSymbol, SymbolLookup, SymbolSet};
use crate::type_checker::core::types::{TypeArena, TypeId};

struct SlotLookup {
    /// Same-qname declaration set the probe iterates.
    siblings: Vec<ContractSymbol>,
    /// (symbol id, return TypeId) pairs — the id-keyed slots.
    returns: Vec<(i64, TypeId)>,
    fields: Vec<(i64, TypeId)>,
    empty: Vec<ContractSymbol>,
}

fn sym(id: i64, file: &str) -> ContractSymbol {
    ContractSymbol {
        id,
        name: "f".to_string(),
        qualified_name: "f".to_string(),
        kind: "function".to_string(),
        visibility: None,
        file_path: Arc::from(file),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for SlotLookup {}

impl SymbolLookup for SlotLookup {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&ContractSymbol> {
        None
    }
    fn all_by_qualified_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.siblings)
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
    fn generic_params(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &[]
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn return_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.returns.iter().find(|(id, _)| *id == symbol_id).map(|(_, t)| *t)
    }
    fn field_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.fields.iter().find(|(id, _)| *id == symbol_id).map(|(_, t)| *t)
    }
}

fn tid(arena: &TypeArena, name: &str) -> TypeId {
    arena.class(name)
}

#[test]
fn own_id_slot_wins() {
    let arena = TypeArena::new();
    let t = tid(&arena, "T");
    let lookup = SlotLookup {
        siblings: vec![sym(1, "a.ts"), sym(2, "a.ts")],
        returns: vec![(1, t), (2, tid(&arena, "Other"))],
        fields: vec![],
        empty: vec![],
    };
    assert_eq!(return_type_by_identity(&lookup, &sym(1, "a.ts")), Some(t));
}

#[test]
fn same_file_sibling_covers_an_overload_set() {
    // The resolved row (implementation) carries no return; the declaration
    // row in the SAME file does — the probe recovers it through ids.
    let arena = TypeArena::new();
    let t = tid(&arena, "T");
    let lookup = SlotLookup {
        siblings: vec![sym(1, "a.ts"), sym(2, "a.ts")],
        returns: vec![(2, t)],
        fields: vec![(2, t)],
        empty: vec![],
    };
    assert_eq!(return_type_by_identity(&lookup, &sym(1, "a.ts")), Some(t));
    assert_eq!(field_type_by_identity(&lookup, &sym(1, "a.ts")), Some(t));
}

#[test]
fn cross_file_same_qname_never_answers() {
    // A same-named declaration in ANOTHER file (another package) holds a
    // type; the probe must not serve it — that is the qname-slot hijack
    // this module exists to prevent.
    let arena = TypeArena::new();
    let other = tid(&arena, "Other");
    let lookup = SlotLookup {
        siblings: vec![sym(1, "a.ts"), sym(2, "other-pkg/b.ts")],
        returns: vec![(2, other)],
        fields: vec![(2, other)],
        empty: vec![],
    };
    assert_eq!(return_type_by_identity(&lookup, &sym(1, "a.ts")), None);
    assert_eq!(field_type_by_identity(&lookup, &sym(1, "a.ts")), None);
}
