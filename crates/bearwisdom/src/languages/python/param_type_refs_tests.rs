// Tests for param_type_refs.rs — when a bare-name initializer carries the
// annotated type of the parameter it names.

use super::super::extract;
use crate::types::{EdgeKind, SymbolKind};

/// Whether a `TypeRef` to `type_name` is attributed to the class property
/// `class.name`.
fn property_types_as(source: &str, class: &str, name: &str, type_name: &str) -> bool {
    let r = extract::extract(source);
    let class_index = r
        .symbols
        .iter()
        .position(|s| s.name == class && s.kind == SymbolKind::Class);
    let Some(index) = r
        .symbols
        .iter()
        .position(|s| s.parent_index == class_index && s.name == name)
    else {
        return false;
    };
    r.refs.iter().any(|reference| {
        reference.kind == EdgeKind::TypeRef
            && reference.target_name == type_name
            && reference.source_symbol_index == index
    })
}

#[test]
fn an_annotated_parameter_types_the_member_it_initializes() {
    let source = concat!(
        "class Svc:\n",
        "    def __init__(self, repo: Repo):\n",
        "        self.repo = repo\n",
    );
    assert!(property_types_as(source, "Svc", "repo", "Repo"));
}

#[test]
fn a_defaulted_annotated_parameter_types_the_member() {
    let source = concat!(
        "class Svc:\n",
        "    def __init__(self, repo: Repo = None):\n",
        "        self.repo = repo\n",
    );
    assert!(property_types_as(source, "Svc", "repo", "Repo"));
}

#[test]
fn a_generic_annotation_contributes_its_head() {
    let source = concat!(
        "class Svc:\n",
        "    def __init__(self, items: List[Repo]):\n",
        "        self.items = items\n",
    );
    assert!(property_types_as(source, "Svc", "items", "List"));
}

#[test]
fn an_unannotated_parameter_types_nothing() {
    let source = concat!(
        "class Svc:\n",
        "    def __init__(self, repo):\n",
        "        self.repo = repo\n",
    );
    let r = extract::extract(source);
    let index = r
        .symbols
        .iter()
        .position(|s| s.name == "repo" && s.kind == SymbolKind::Property)
        .unwrap();
    assert!(
        !r.refs
            .iter()
            .any(|reference| reference.kind == EdgeKind::TypeRef
                && reference.source_symbol_index == index),
        "{:?}",
        r.refs
    );
}

#[test]
fn a_rebound_name_no_longer_carries_the_parameter_annotation() {
    let source = concat!(
        "class Svc:\n",
        "    def __init__(self, repo: Repo):\n",
        "        repo = wrap(repo)\n",
        "        self.repo = repo\n",
    );
    assert!(!property_types_as(source, "Svc", "repo", "Repo"));
}

#[test]
fn a_member_initialized_from_another_member_takes_no_parameter_type() {
    let source = concat!(
        "class Svc:\n",
        "    def __init__(self, repo: Repo):\n",
        "        self.store = self.repo\n",
    );
    assert!(!property_types_as(source, "Svc", "store", "Repo"));
}
