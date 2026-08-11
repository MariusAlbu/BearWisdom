use super::*;
use crate::types::EdgeKind;

fn imports_refs(source: &str) -> Vec<crate::types::ExtractedRef> {
    crate::languages::python::extract::extract(source)
        .refs
        .into_iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .collect()
}

// ---------------------------------------------------------------------------
// `import` statement forms
// ---------------------------------------------------------------------------

#[test]
fn plain_single_segment_import_has_no_module() {
    let refs = imports_refs("import json\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "json")
        .expect("expected an Imports ref for 'json'");
    assert!(imp.module.is_none());
    assert!(imp.chain.is_none());
}

#[test]
fn plain_dotted_import_targets_last_segment_under_parent_module() {
    let refs = imports_refs("import foo.bar\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "bar")
        .expect("expected an Imports ref for 'bar' (last segment)");
    assert_eq!(imp.module.as_deref(), Some("foo"));
    assert!(imp.chain.is_none());
}

#[test]
fn dotted_import_with_alias_carries_declared_name_as_chain_and_alias_as_target() {
    let refs = imports_refs("import foo.bar as fb\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "fb")
        .expect("expected an Imports ref bound to the alias 'fb'");
    assert_eq!(imp.module.as_deref(), Some("foo"));
    let chain = imp.chain.as_ref().expect("expected a rename chain");
    assert_eq!(chain.segments.len(), 1);
    assert_eq!(chain.segments[0].name, "bar");
}

#[test]
fn single_segment_import_with_alias_carries_declared_name_as_chain() {
    let refs = imports_refs("import numpy as np\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "np")
        .expect("expected an Imports ref bound to the alias 'np'");
    assert!(imp.module.is_none());
    let chain = imp.chain.as_ref().expect("expected a rename chain");
    assert_eq!(chain.segments[0].name, "numpy");
}

// ---------------------------------------------------------------------------
// `from ... import ...` forms
// ---------------------------------------------------------------------------

#[test]
fn from_import_targets_declared_name_under_module() {
    let refs = imports_refs("from a.b import c\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "c")
        .expect("expected an Imports ref for 'c'");
    assert_eq!(imp.module.as_deref(), Some("a.b"));
    assert!(imp.chain.is_none());
}

#[test]
fn from_import_with_alias_carries_declared_name_as_chain_and_alias_as_target() {
    let refs = imports_refs("from a import b as c\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "c")
        .expect("expected an Imports ref bound to the alias 'c'");
    assert_eq!(imp.module.as_deref(), Some("a"));
    let chain = imp.chain.as_ref().expect("expected a rename chain");
    assert_eq!(chain.segments.len(), 1);
    assert_eq!(chain.segments[0].name, "b");
}

#[test]
fn from_import_alias_matching_dunder_all_keeps_declared_name_with_no_chain() {
    // A re-exported alias (`__all__` lists the bound name) must keep
    // `target_name` as the module's own declared name with no chain — the
    // shape the file-level re-export map reads directly as a cross-module
    // hop; it never consults `chain`.
    let refs = imports_refs("from .models import User as Account\n__all__ = [\"Account\"]\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "User")
        .expect("expected an Imports ref with declared-side target_name 'User'");
    assert!(imp.chain.is_none());
    assert!(imp.is_reexport);
}

#[test]
fn from_import_alias_equal_to_declared_name_carries_no_chain() {
    let refs = imports_refs("from a import b as b\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "b")
        .expect("expected an Imports ref for 'b'");
    assert!(imp.chain.is_none());
}

#[test]
fn wildcard_import_targets_star() {
    let refs = imports_refs("from a.b import *\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "*")
        .expect("expected a wildcard Imports ref");
    assert_eq!(imp.module.as_deref(), Some("a.b"));
}

// ---------------------------------------------------------------------------
// Relative imports — leading dot(s) preserved in `module`
// ---------------------------------------------------------------------------

#[test]
fn bare_relative_import_keeps_single_dot_module() {
    let refs = imports_refs("from . import x\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "x")
        .expect("expected an Imports ref for 'x'");
    assert_eq!(imp.module.as_deref(), Some("."));
}

#[test]
fn named_relative_import_keeps_leading_dot_in_module() {
    let refs = imports_refs("from .rel import y\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "y")
        .expect("expected an Imports ref for 'y'");
    assert_eq!(imp.module.as_deref(), Some(".rel"));
}

#[test]
fn multi_level_relative_import_keeps_both_dots_in_module() {
    let refs = imports_refs("from ..pkg import w\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "w")
        .expect("expected an Imports ref for 'w'");
    assert_eq!(imp.module.as_deref(), Some("..pkg"));
}

#[test]
fn relative_wildcard_import_keeps_leading_dot_in_module() {
    let refs = imports_refs("from .models import *\n");
    let imp = refs
        .iter()
        .find(|r| r.target_name == "*")
        .expect("expected a wildcard Imports ref");
    assert_eq!(imp.module.as_deref(), Some(".models"));
}

// ---------------------------------------------------------------------------
// build_import_map — chain-root module attachment for qualified calls
// ---------------------------------------------------------------------------

#[test]
fn build_import_map_feeds_dot_prefixed_module_onto_chain_root_calls() {
    // `y.something()` — chain root `y` was bound via a relative import;
    // `extract_calls_from_body` looks up `y` in the map `build_import_map`
    // built and attaches the result as the Calls ref's `module`. Proves the
    // map itself carries the leading dot, not just the raw Imports ref.
    let source = "from .rel import y\n\ndef caller():\n    y.something()\n";
    let tree = crate::languages::python::extract::extract(source);
    let call_ref = tree
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "something")
        .expect("expected a Calls ref for 'something'");
    assert_eq!(call_ref.module.as_deref(), Some(".rel"));
}
