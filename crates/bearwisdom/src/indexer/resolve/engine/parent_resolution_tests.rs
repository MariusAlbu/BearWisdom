use std::collections::HashMap;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::{rebuild_inherits_by_id, resolve_parent_id_scoped};
use crate::indexer::resolve::engine::contract::{
    Symbol as ContractSymbol, SymbolLookup, SymbolSet,
};

#[derive(Default)]
struct FakeLookup {
    by_name: HashMap<String, Vec<ContractSymbol>>,
    by_qname: HashMap<String, ContractSymbol>,
    members: HashMap<i64, Vec<ContractSymbol>>,
    empty: Vec<ContractSymbol>,
    empty_pairs: Vec<(String, String)>,
}

fn sym(id: i64, name: &str, qname: &str, kind: &str, pkg: Option<i64>) -> ContractSymbol {
    ContractSymbol {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from("x.php"),
        scope_path: None,
        package_id: pkg,
        signature: None,
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for FakeLookup {}

impl SymbolLookup for FakeLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.by_name.get(name).unwrap_or(&self.empty))
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&ContractSymbol> {
        self.by_qname.get(qname)
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn members_of_id(&self, id: i64) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.members.get(&id).unwrap_or(&self.empty))
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
        &self.empty_pairs
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn homonym_lookup() -> FakeLookup {
    let mut l = FakeLookup::default();
    let internal = sym(1, "TestCase", "Illuminate\\Foundation\\Testing.TestCase", "class", Some(7));
    let external = sym(2, "TestCase", "PHPUnit\\Framework.TestCase", "class", None);
    l.members.insert(1, vec![sym(11, "seed", "Illuminate\\Foundation\\Testing.TestCase.seed", "method", Some(7))]);
    l.members.insert(2, vec![sym(21, "assertTrue", "PHPUnit\\Framework.TestCase.assertTrue", "method", None)]);
    l.by_name.insert("TestCase".into(), vec![internal.clone(), external.clone()]);
    l.by_qname.insert(internal.qualified_name.clone(), internal);
    l.by_qname.insert(external.qualified_name.clone(), external);
    l
}

#[test]
fn import_evidence_beats_member_bearing_homonym() {
    let l = homonym_lookup();
    assert_eq!(
        resolve_parent_id_scoped(&l, "TestCase", "Tests.AuthTest", Some(9), Some("PHPUnit\\Framework")),
        Some(2),
        "the imported module's declaration must win over an earlier homonym"
    );
}

#[test]
fn without_evidence_the_heuristic_ladder_holds() {
    let l = homonym_lookup();
    // No import evidence, child in package 7: the same-package member-bearing
    // candidate wins.
    assert_eq!(resolve_parent_id_scoped(&l, "TestCase", "App.SomeTest", Some(7), None), Some(1));
    // Foreign package, no evidence: first member-bearing candidate.
    assert_eq!(resolve_parent_id_scoped(&l, "TestCase", "App.SomeTest", Some(9), None), Some(1));
}

#[test]
fn evidence_with_no_matching_candidate_falls_through_to_ladder() {
    let l = homonym_lookup();
    assert_eq!(
        resolve_parent_id_scoped(&l, "TestCase", "App.SomeTest", Some(7), Some("Some\\Other\\Ns")),
        Some(1),
        "unmatchable evidence must not lose the edge entirely"
    );
}

#[test]
fn same_namespace_sibling_beats_foreign_member_bearing_homonym() {
    let mut l = FakeLookup::default();
    let foreign = sym(1, "Assert", "Illuminate\\Testing.Assert", "class", Some(7));
    let sibling = sym(2, "Assert", "PHPUnit\\Framework.Assert", "class", None);
    l.members.insert(1, vec![sym(11, "assertJson", "Illuminate\\Testing.Assert.assertJson", "method", Some(7))]);
    l.members.insert(2, vec![sym(21, "assertTrue", "PHPUnit\\Framework.Assert.assertTrue", "method", None)]);
    l.by_name.insert("Assert".into(), vec![foreign.clone(), sibling.clone()]);
    l.by_qname.insert(foreign.qualified_name.clone(), foreign);
    l.by_qname.insert(sibling.qualified_name.clone(), sibling);
    // `TestCase extends Assert` inside PHPUnit\Framework, no import: the
    // same-namespace Assert wins over the foreign member-bearing homonym.
    assert_eq!(
        resolve_parent_id_scoped(&l, "Assert", "PHPUnit\\Framework.TestCase", None, None),
        Some(2)
    );
}

#[test]
fn rebuild_threads_evidence_per_edge() {
    let mut l = homonym_lookup();
    let child = sym(30, "AuthTest", "Tests.AuthTest", "class", Some(9));
    l.by_qname.insert(child.qualified_name.clone(), child);
    let mut inherits: FxHashMap<String, Vec<String>> = FxHashMap::default();
    inherits.insert("Tests.AuthTest".into(), vec!["TestCase".into()]);
    let mut evidence: FxHashMap<(String, String), String> = FxHashMap::default();
    evidence.insert(
        ("Tests.AuthTest".into(), "TestCase".into()),
        "PHPUnit\\Framework".into(),
    );
    let map = rebuild_inherits_by_id(&l, &inherits, &evidence);
    assert_eq!(map.get(&30), Some(&vec![2]));
}
