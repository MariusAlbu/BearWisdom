// =============================================================================
// type_checker/subtype_tests.rs — Unit tests for the conditional-type
// subtype check.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::types::{PrimKind, Type, TypeArena};
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

// ---------------------------------------------------------------------------
// TypeId-form tests — mirror the string form's coverage and add the cases
// it can't represent (union variance, primitive identity, optional peeling).
// ---------------------------------------------------------------------------

#[test]
fn typed_identity_is_yes() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(user, user, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_anything_assignable_to_any_or_unknown() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let any = arena.class("any");
    let unknown = arena.class("unknown");
    let lookup = SubtypeFixture::new();

    assert_eq!(
        is_assignable_to_typed(user, any, &arena, &lookup),
        SubtypeResult::Yes
    );
    assert_eq!(
        is_assignable_to_typed(user, unknown, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_never_assignable_to_anything() {
    let mut arena = TypeArena::new();
    let never_prim = arena.primitive(PrimKind::Never);
    let never_class = arena.class("never");
    let user = arena.class("User");
    let lookup = SubtypeFixture::new();

    assert_eq!(
        is_assignable_to_typed(never_prim, user, &arena, &lookup),
        SubtypeResult::Yes
    );
    assert_eq!(
        is_assignable_to_typed(never_class, user, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_inheritance_walk_one_and_multi_hop() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let admin = arena.class("Admin");
    let super_admin = arena.class("SuperAdmin");
    let lookup = SubtypeFixture::new()
        .with_parent("Admin", "User")
        .with_parent("SuperAdmin", "Admin");

    assert_eq!(
        is_assignable_to_typed(admin, user, &arena, &lookup),
        SubtypeResult::Yes
    );
    assert_eq!(
        is_assignable_to_typed(super_admin, user, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_inheritance_cycle_returns_unknown() {
    let mut arena = TypeArena::new();
    let a = arena.class("A");
    let c = arena.class("C");
    let lookup = SubtypeFixture::new()
        .with_parent("A", "B")
        .with_parent("B", "A");
    assert_eq!(
        is_assignable_to_typed(a, c, &arena, &lookup),
        SubtypeResult::Unknown
    );
}

#[test]
fn typed_distinct_primitives_are_not_assignable() {
    let mut arena = TypeArena::new();
    let s = arena.primitive(PrimKind::Str);
    let n = arena.primitive(PrimKind::Int);
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(s, n, &arena, &lookup),
        SubtypeResult::No
    );
}

#[test]
fn typed_equal_primitives_are_assignable() {
    let mut arena = TypeArena::new();
    let s1 = arena.primitive(PrimKind::Str);
    let s2 = arena.primitive(PrimKind::Str);
    let lookup = SubtypeFixture::new();
    // Interning ensures identity.
    assert_eq!(s1, s2);
    assert_eq!(
        is_assignable_to_typed(s1, s2, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_optional_target_peels_one_layer() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let opt_user = arena.intern(Type::Optional(user));
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(user, opt_user, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_union_source_assignable_when_all_branches_are() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let admin = arena.class("Admin");
    let union = arena.intern(Type::Union(vec![user, admin]));
    let lookup = SubtypeFixture::new().with_parent("Admin", "User");
    // Admin → User; User → User by identity; union → User.
    assert_eq!(
        is_assignable_to_typed(union, user, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_union_source_fails_when_any_branch_fails() {
    // string | number  → string. The number branch is disjoint
    // primitives (Int vs Str → No) so the whole union → No, even
    // though the string branch matches by identity.
    let mut arena = TypeArena::new();
    let string_ty = arena.primitive(PrimKind::Str);
    let number_ty = arena.primitive(PrimKind::Int);
    let union = arena.intern(Type::Union(vec![string_ty, number_ty]));
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(union, string_ty, &arena, &lookup),
        SubtypeResult::No
    );
}

#[test]
fn typed_union_target_assignable_when_any_branch_matches() {
    let mut arena = TypeArena::new();
    let admin = arena.class("Admin");
    let user = arena.class("User");
    let order = arena.class("Order");
    let union = arena.intern(Type::Union(vec![user, order]));
    let lookup = SubtypeFixture::new().with_parent("Admin", "User");
    assert_eq!(
        is_assignable_to_typed(admin, union, &arena, &lookup),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_nominal_primitive_names_disjoint_with_prims() {
    // Annotations intern `string` / `number` as `Class`, not `Primitive`.
    // With a language primitive map they're still recognized as disjoint.
    let mut arena = TypeArena::new();
    let s = arena.class("string");
    let n = arena.class("number");
    let lookup = SubtypeFixture::new();
    let prims: &[(&str, PrimKind)] = &[("string", PrimKind::Str), ("number", PrimKind::Int)];
    assert_eq!(
        is_assignable_to_typed_with(s, n, &arena, &lookup, prims),
        SubtypeResult::No
    );
}

#[test]
fn typed_nominal_primitive_names_undecided_without_prims() {
    // No primitive map: nominal `Class("string")` vs `Class("number")` has no
    // inheritance link, so the check stays conservative (Unknown).
    let mut arena = TypeArena::new();
    let s = arena.class("string");
    let n = arena.class("number");
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(s, n, &arena, &lookup),
        SubtypeResult::Unknown
    );
}

#[test]
fn typed_bridge_primitive_value_vs_nominal_param() {
    // A `Primitive(Str)` arg against a nominal `Class("number")` / `Class("string")`
    // param — the bridge decides disjointness either way.
    let mut arena = TypeArena::new();
    let str_val = arena.primitive(PrimKind::Str);
    let number_param = arena.class("number");
    let string_param = arena.class("string");
    let lookup = SubtypeFixture::new();
    let prims: &[(&str, PrimKind)] = &[("string", PrimKind::Str), ("number", PrimKind::Int)];
    assert_eq!(
        is_assignable_to_typed_with(str_val, number_param, &arena, &lookup, prims),
        SubtypeResult::No
    );
    assert_eq!(
        is_assignable_to_typed_with(str_val, string_param, &arena, &lookup, prims),
        SubtypeResult::Yes
    );
}

#[test]
fn typed_same_kind_different_primitive_names_assignable() {
    // Two distinct names mapping to the same kind (`int` / `Int32`) are
    // assignable — disjointness keys on the kind, not the spelling.
    let mut arena = TypeArena::new();
    let int_name = arena.class("int");
    let int32_name = arena.class("Int32");
    let lookup = SubtypeFixture::new();
    let prims: &[(&str, PrimKind)] = &[("int", PrimKind::Int), ("Int32", PrimKind::Int)];
    assert_eq!(
        is_assignable_to_typed_with(int_name, int32_name, &arena, &lookup, prims),
        SubtypeResult::Yes
    );
}
