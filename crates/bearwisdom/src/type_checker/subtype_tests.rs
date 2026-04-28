// =============================================================================
// type_checker/subtype_tests.rs — Unit tests for the conditional-type
// subtype check.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::types::AliasTarget;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test fixture — reuses the same minimal-SymbolLookup pattern as
// `alias_tests.rs`. Only `parent_class_qname` is consulted by
// `is_assignable_to`; everything else stays at trait defaults.
// ---------------------------------------------------------------------------

struct SubtypeFixture {
    parents: HashMap<String, String>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl SubtypeFixture {
    fn new() -> Self {
        Self {
            parents: HashMap::new(),
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }

    fn with_parent(mut self, child: &str, parent: &str) -> Self {
        self.parents.insert(child.to_string(), parent.to_string());
        self
    }
}

impl SymbolLookup for SubtypeFixture {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
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
    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.parents.get(class_qname).map(|s| s.as_str())
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn identity_is_assignable() {
    let lookup = SubtypeFixture::new();
    assert_eq!(is_assignable_to("string", "string", &lookup), Some(true));
    assert_eq!(is_assignable_to("User", "User", &lookup), Some(true));
}

#[test]
fn anything_assignable_to_any() {
    let lookup = SubtypeFixture::new();
    assert_eq!(is_assignable_to("string", "any", &lookup), Some(true));
    assert_eq!(is_assignable_to("User", "any", &lookup), Some(true));
}

#[test]
fn anything_assignable_to_unknown() {
    let lookup = SubtypeFixture::new();
    assert_eq!(is_assignable_to("string", "unknown", &lookup), Some(true));
    assert_eq!(is_assignable_to("User", "unknown", &lookup), Some(true));
}

#[test]
fn never_assignable_to_anything() {
    let lookup = SubtypeFixture::new();
    assert_eq!(is_assignable_to("never", "string", &lookup), Some(true));
    assert_eq!(is_assignable_to("never", "User", &lookup), Some(true));
}

#[test]
fn inheritance_one_hop() {
    // Admin extends User — Admin is assignable to User.
    let lookup = SubtypeFixture::new().with_parent("Admin", "User");
    assert_eq!(is_assignable_to("Admin", "User", &lookup), Some(true));
}

#[test]
fn inheritance_multi_hop() {
    // SuperAdmin → Admin → User. SuperAdmin assignable to User.
    let lookup = SubtypeFixture::new()
        .with_parent("SuperAdmin", "Admin")
        .with_parent("Admin", "User");
    assert_eq!(is_assignable_to("SuperAdmin", "User", &lookup), Some(true));
}

#[test]
fn inheritance_does_not_match_when_unrelated() {
    let lookup = SubtypeFixture::new()
        .with_parent("Admin", "User")
        .with_parent("Order", "Entity");
    // No relation between Order and User — undecidable, NOT false.
    assert_eq!(is_assignable_to("Order", "User", &lookup), None);
}

#[test]
fn inheritance_cycle_terminates() {
    // Pathological: A → B → A. Walker must terminate without
    // reporting an inheritance match.
    let lookup = SubtypeFixture::new()
        .with_parent("A", "B")
        .with_parent("B", "A");
    assert_eq!(is_assignable_to("A", "C", &lookup), None);
}

#[test]
fn distinct_primitives_are_not_assignable() {
    let lookup = SubtypeFixture::new();
    assert_eq!(is_assignable_to("string", "number", &lookup), Some(false));
    assert_eq!(is_assignable_to("number", "boolean", &lookup), Some(false));
    assert_eq!(is_assignable_to("undefined", "null", &lookup), Some(false));
}

#[test]
fn primitive_to_user_type_is_undecidable() {
    // Could go either way without more info — return None so
    // the caller skips the conditional and falls through to a miss.
    let lookup = SubtypeFixture::new();
    assert_eq!(is_assignable_to("string", "User", &lookup), None);
    assert_eq!(is_assignable_to("User", "string", &lookup), None);
}

#[test]
fn empty_strings_are_undecidable() {
    let lookup = SubtypeFixture::new();
    assert_eq!(is_assignable_to("", "User", &lookup), None);
    assert_eq!(is_assignable_to("User", "", &lookup), None);
}
