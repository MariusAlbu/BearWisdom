// Tests for instance_attrs.rs — which declaration owns a `self.x = …` target.

use super::super::extract;
use crate::types::{EdgeKind, ExtractedSymbol, SymbolKind};

/// Indices of the symbols `class` carries under `name`.
fn members(symbols: &[ExtractedSymbol], class: &str, name: &str) -> Vec<usize> {
    let class_index = symbols
        .iter()
        .position(|s| s.name == class && s.kind == SymbolKind::Class);
    symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.parent_index == class_index && s.name == name)
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn an_instance_attribute_is_a_property_of_the_class() {
    let source = "class Svc:\n    def __init__(self):\n        self.repo = 1\n";
    let r = extract::extract(source);
    let found = members(&r.symbols, "Svc", "repo");
    assert_eq!(found.len(), 1, "{:?}", r.symbols);
    let declaration = &r.symbols[found[0]];
    assert_eq!(declaration.kind, SymbolKind::Property);
    assert_eq!(declaration.qualified_name, "Svc.repo");
    assert_eq!(declaration.scope_path.as_deref(), Some("Svc"));
}

#[test]
fn a_second_method_assigning_the_same_name_reuses_the_declaration() {
    let source = concat!(
        "class Svc:\n",
        "    def __init__(self):\n",
        "        self.repo = 1\n",
        "    def reset(self):\n",
        "        self.repo = 2\n",
    );
    let r = extract::extract(source);
    assert_eq!(members(&r.symbols, "Svc", "repo").len(), 1, "{:?}", r.symbols);
}

#[test]
fn a_class_body_attribute_absorbs_the_method_assignment() {
    let source = concat!(
        "class Svc:\n",
        "    repo: int\n",
        "    def __init__(self):\n",
        "        self.repo = 1\n",
    );
    let r = extract::extract(source);
    assert_eq!(members(&r.symbols, "Svc", "repo").len(), 1, "{:?}", r.symbols);
}

#[test]
fn a_constructor_initializer_types_the_declaration() {
    let source = "class Svc:\n    def __init__(self):\n        self.cache = Cache()\n";
    let r = extract::extract(source);
    let found = members(&r.symbols, "Svc", "cache");
    assert_eq!(found.len(), 1, "{:?}", r.symbols);
    assert!(
        r.refs.iter().any(|reference| reference.kind == EdgeKind::TypeRef
            && reference.target_name == "Cache"
            && reference.source_symbol_index == found[0]),
        "{:?}",
        r.refs
    );
}

#[test]
fn an_attribute_on_another_object_is_not_a_class_member() {
    let source = "class Svc:\n    def run(self, other):\n        other.repo = 1\n";
    let r = extract::extract(source);
    assert!(
        members(&r.symbols, "Svc", "repo").is_empty(),
        "{:?}",
        r.symbols
    );
}

#[test]
fn an_attribute_assignment_outside_a_class_declares_no_member() {
    let source = "def run(obj):\n    obj.repo = 1\n";
    let r = extract::extract(source);
    assert!(
        !r.symbols
            .iter()
            .any(|s| s.name == "repo" && s.kind == SymbolKind::Property),
        "{:?}",
        r.symbols
    );
}

#[test]
fn an_assignment_under_a_branch_declares_on_the_class() {
    let source = concat!(
        "class Svc:\n",
        "    def run(self, flag):\n",
        "        if flag:\n",
        "            self.repo = Cache()\n",
    );
    let r = extract::extract(source);
    let found = members(&r.symbols, "Svc", "repo");
    assert_eq!(found.len(), 1, "{:?}", r.symbols);
    assert_eq!(r.symbols[found[0]].qualified_name, "Svc.repo");
}
