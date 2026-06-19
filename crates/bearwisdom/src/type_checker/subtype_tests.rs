// =============================================================================
// type_checker/subtype_tests.rs — Unit tests for the conditional-type
// subtype check.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup, SymbolSet};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::symbol_types::{SymbolTypeData, SymbolTypeMap};
use crate::type_checker::core::types::{PrimKind, Type, TypeArena, TypeId};
use crate::types::AliasTarget;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test fixture — reuses the same minimal-SymbolLookup pattern as
// `alias_tests.rs`. Only `parent_class_qname` is consulted by
// `is_assignable_to`; everything else stays at trait defaults.
// ---------------------------------------------------------------------------

struct SubtypeFixture {
    parents: HashMap<String, String>,
    empty: Vec<Symbol>,
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
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> {
        None
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
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

/// Empty per-symbol type map for the nominal-only TypeId tests that never reach
/// the structural member-type comparison.
fn empty_types() -> SymbolTypeMap {
    SymbolTypeMap::new()
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
        is_assignable_to_typed(
            user,
            user,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(
            user,
            any,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
        SubtypeResult::Yes
    );
    assert_eq!(
        is_assignable_to_typed(
            user,
            unknown,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(
            never_prim,
            user,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
        SubtypeResult::Yes
    );
    assert_eq!(
        is_assignable_to_typed(
            never_class,
            user,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(
            admin,
            user,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
        SubtypeResult::Yes
    );
    assert_eq!(
        is_assignable_to_typed(
            super_admin,
            user,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(a, c, &arena, &lookup, &MembersIndex::new(), &empty_types()),
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
        is_assignable_to_typed(s, n, &arena, &lookup, &MembersIndex::new(), &empty_types()),
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
        is_assignable_to_typed(
            s1,
            s2,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(
            user,
            opt_user,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(
            union,
            user,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(
            union,
            string_ty,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed(
            admin,
            union,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types()
        ),
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
        is_assignable_to_typed_with(
            s,
            n,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types(),
            prims
        ),
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
        is_assignable_to_typed(s, n, &arena, &lookup, &MembersIndex::new(), &empty_types()),
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
        is_assignable_to_typed_with(
            str_val,
            number_param,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types(),
            prims
        ),
        SubtypeResult::No
    );
    assert_eq!(
        is_assignable_to_typed_with(
            str_val,
            string_param,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types(),
            prims
        ),
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
        is_assignable_to_typed_with(
            int_name,
            int32_name,
            &arena,
            &lookup,
            &MembersIndex::new(),
            &empty_types(),
            prims
        ),
        SubtypeResult::Yes
    );
}

// ---------------------------------------------------------------------------
// Structural-assignability arm — shape matching between Class/Interface/Struct
// types. The arm only ever turns Unknown into Yes; a near-miss shape (missing
// member, a field where a method is required, or an INCOMPATIBLE member type)
// stays Unknown, never a false Yes. A target member is satisfied only when the
// source carries a member with the same name, the same kind, AND a member type
// assignable in the correct variance — matched against recorded
// `SymbolTypeData`. A matched member with no recorded type data keeps the arm
// at Unknown (the load-bearing missing-type-info → Unknown rule).
// ---------------------------------------------------------------------------

/// Build a member Symbol for the structural fixtures. `name` and `kind` are
/// the structural-presence key; `id` keys the member's recorded type data.
fn member(id: i64, name: &str, kind: &str) -> Symbol {
    Symbol {
        id,
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: std::sync::Arc::from("test.go"),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

/// `SymbolTypeData` for a method member: positional parameter types plus a
/// return type. Presence of `return_type` is what marks the member callable in
/// the structural comparison.
fn method_types(params: Vec<TypeId>, return_ty: TypeId) -> SymbolTypeData {
    SymbolTypeData {
        declared_type: None,
        return_type: Some(return_ty),
        param_types: params,
        generic_params: Vec::new(),
    }
}

/// `SymbolTypeData` for a field member: a single declared type, no return type.
fn field_types(declared: TypeId) -> SymbolTypeData {
    SymbolTypeData {
        declared_type: Some(declared),
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[test]
fn structural_struct_satisfies_interface_full_shape() {
    // Go: struct ReadCloser{Read()E, Close()E} satisfies interface
    // io.ReadCloser{Read()E, Close()E} with no nominal inheritance link. Member
    // types match exactly, so the structural arm yields Yes.
    let mut arena = TypeArena::new();
    let src = arena.class("mypkg.ReadCloser");
    let tgt = arena.class("io.ReadCloser");
    let e = arena.class("error");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    for (id, name, owner) in [
        (1, "Read", src),
        (2, "Close", src),
        (3, "Read", tgt),
        (4, "Close", tgt),
    ] {
        members.add_direct(owner, member(id, name, "method"));
        types.insert(id, method_types(vec![], e));
    }
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Yes
    );
}

#[test]
fn structural_extra_source_members_are_fine() {
    // struct Logger{Log, Info, Error} satisfies interface Output{Log}: the
    // target's members are a subset of the source's, and the matched member's
    // type matches.
    let mut arena = TypeArena::new();
    let src = arena.class("Logger");
    let tgt = arena.class("Output");
    let unit = arena.class("void");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    for (id, name, owner) in [
        (1, "Log", src),
        (2, "Info", src),
        (3, "Error", src),
        (4, "Log", tgt),
    ] {
        members.add_direct(owner, member(id, name, "method"));
        types.insert(id, method_types(vec![], unit));
    }
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Yes
    );
}

#[test]
fn structural_satisfied_via_extension_members() {
    // The source carries the required member only as an extension (C# extension
    // method, Rust `impl Trait for T`). The arm unions direct ∪ extensions, so
    // the shape is satisfied and the matched member's type matches.
    let mut arena = TypeArena::new();
    let src = arena.class("Widget");
    let tgt = arena.class("Drawable");
    let unit = arena.class("void");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_extension(src, member(1, "Draw", "method"));
    members.add_direct(tgt, member(2, "Draw", "method"));
    types.insert(1, method_types(vec![], unit));
    types.insert(2, method_types(vec![], unit));
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Yes
    );
}

#[test]
fn structural_field_shape_matches_on_declared_type() {
    // struct Point3{x, y, z: number} satisfies interface Point2{x, y: number}.
    // Field members compare their declared types covariantly; matching kinds.
    let mut arena = TypeArena::new();
    let src = arena.class("Point3");
    let tgt = arena.class("Point2");
    let num = arena.primitive(PrimKind::Float);
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    for (id, name, owner) in [
        (1, "x", src),
        (2, "y", src),
        (3, "z", src),
        (4, "x", tgt),
        (5, "y", tgt),
    ] {
        members.add_direct(owner, member(id, name, "field"));
        types.insert(id, field_types(num));
    }
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Yes
    );
}

#[test]
fn structural_identity_short_circuits_before_member_walk() {
    // A type vs itself returns Yes on the identity check, never reaching the
    // structural arm — proven by leaving the members index empty.
    let mut arena = TypeArena::new();
    let ty = arena.class("Closer");
    let members = MembersIndex::new();
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(ty, ty, &arena, &lookup, &members, &empty_types()),
        SubtypeResult::Yes
    );
}

#[test]
fn structural_missing_member_is_not_assignable() {
    // struct Reader{Read} vs interface ReadCloser{Read, Close}: Close is
    // absent on the source, so the necessity test fails. The arm reports
    // Unknown (never a false Yes); a missing member is the core near-miss.
    let mut arena = TypeArena::new();
    let src = arena.class("Reader");
    let tgt = arena.class("ReadCloser");
    let e = arena.class("error");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(src, member(1, "Read", "method"));
    members.add_direct(tgt, member(2, "Read", "method"));
    members.add_direct(tgt, member(3, "Close", "method"));
    types.insert(1, method_types(vec![], e));
    types.insert(2, method_types(vec![], e));
    types.insert(3, method_types(vec![], e));
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown,
        "missing Close must not be declared assignable"
    );
}

#[test]
fn structural_wrong_kind_is_not_assignable() {
    // interface Writer{Write: method} vs struct S{Write: field}. The name
    // matches but the kind does not — a field cannot satisfy a method
    // requirement, so the shape is not matched.
    let mut arena = TypeArena::new();
    let src = arena.class("S");
    let tgt = arena.class("Writer");
    let unit = arena.class("void");
    let num = arena.primitive(PrimKind::Float);
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(src, member(1, "Write", "field"));
    members.add_direct(tgt, member(2, "Write", "method"));
    types.insert(1, field_types(num));
    types.insert(2, method_types(vec![], unit));
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown,
        "a field must not satisfy a method-shaped member"
    );
}

#[test]
fn structural_empty_target_members_is_unknown() {
    // The target's member set can't be enumerated (external interface whose
    // members aren't hydrated, or a type with no recorded members). An empty
    // set is vacuously satisfied by anything — which would be a false Yes — so
    // the arm reports Unknown.
    let mut arena = TypeArena::new();
    let src = arena.class("HasStuff");
    let tgt = arena.class("io.Unhydrated");
    let unit = arena.class("void");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(src, member(1, "DoThing", "method"));
    types.insert(1, method_types(vec![], unit));
    // tgt deliberately has no members registered.
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown
    );
}

#[test]
fn structural_empty_source_members_is_unknown() {
    // Mirror of the above: the source's shape is unknown to us, so we can't
    // assert it satisfies the target.
    let mut arena = TypeArena::new();
    let src = arena.class("Opaque");
    let tgt = arena.class("Reader");
    let e = arena.class("error");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(tgt, member(1, "Read", "method"));
    types.insert(1, method_types(vec![], e));
    // src deliberately has no members registered.
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown
    );
}

#[test]
fn structural_arm_does_not_override_primitive_disjointness() {
    // The structural arm is gated to Class → Class and runs only after the
    // primitive/inheritance arms. Two disjoint primitives still resolve to No
    // before the structural arm is ever consulted, regardless of members.
    let mut arena = TypeArena::new();
    let s = arena.class("string");
    let n = arena.class("number");
    let num = arena.primitive(PrimKind::Float);
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    // Give both the same shape; the primitive arm must still win with No.
    members.add_direct(s, member(1, "length", "field"));
    members.add_direct(n, member(2, "length", "field"));
    types.insert(1, field_types(num));
    types.insert(2, field_types(num));
    let lookup = SubtypeFixture::new();
    let prims: &[(&str, PrimKind)] = &[("string", PrimKind::Str), ("number", PrimKind::Int)];
    assert_eq!(
        is_assignable_to_typed_with(s, n, &arena, &lookup, &members, &types, prims),
        SubtypeResult::No
    );
}

#[test]
fn structural_arm_does_not_override_nominal_inheritance() {
    // When a nominal inheritance link exists, the inheritance arm answers Yes
    // before the structural arm runs. Proven by giving the child an INCOMPLETE
    // shape (missing a parent member) — nominal Yes must still win.
    let mut arena = TypeArena::new();
    let admin = arena.class("Admin");
    let user = arena.class("User");
    let num = arena.primitive(PrimKind::Float);
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(user, member(1, "id", "field"));
    members.add_direct(user, member(2, "email", "field"));
    members.add_direct(admin, member(3, "id", "field")); // missing `email`
    types.insert(1, field_types(num));
    types.insert(2, field_types(num));
    types.insert(3, field_types(num));
    let lookup = SubtypeFixture::new().with_parent("Admin", "User");
    assert_eq!(
        is_assignable_to_typed(admin, user, &arena, &lookup, &members, &types),
        SubtypeResult::Yes
    );
}

// ---------------------------------------------------------------------------
// Member-type comparison — the soundness fix. Matching a target member by
// (name, kind) alone, without comparing the matched members' parameter / return
// types, is a false Yes. These tests prove the type comparison fires and that a
// member with no recorded type data falls back to Unknown, never Yes.
// ---------------------------------------------------------------------------

#[test]
fn structural_incompatible_param_type_is_not_assignable() {
    // target Writer{ Write(s: string) } vs source S{ Write(n: number) }. Name
    // and kind match, but the parameter types are disjoint primitives, so the
    // contravariant param check fails. Matching on name+kind alone would
    // wrongly declare S assignable to Writer — the arm must stay at Unknown.
    let mut arena = TypeArena::new();
    let src = arena.class("S");
    let tgt = arena.class("Writer");
    let string_ty = arena.primitive(PrimKind::Str);
    let number_ty = arena.primitive(PrimKind::Int);
    let unit = arena.class("void");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(src, member(1, "Write", "method"));
    members.add_direct(tgt, member(2, "Write", "method"));
    // src.Write(number) ; tgt.Write(string)
    types.insert(1, method_types(vec![number_ty], unit));
    types.insert(2, method_types(vec![string_ty], unit));
    let lookup = SubtypeFixture::new();
    assert_ne!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Yes,
        "Write(number) must not satisfy Write(string)"
    );
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown
    );
}

#[test]
fn structural_incompatible_return_type_is_not_assignable() {
    // target Reader{ Read(): string } vs source S{ Read(): number }. The
    // covariant return check fails on disjoint primitives. A (name, kind) match
    // must not stand in for the return-type comparison.
    let mut arena = TypeArena::new();
    let src = arena.class("S");
    let tgt = arena.class("Reader");
    let string_ty = arena.primitive(PrimKind::Str);
    let number_ty = arena.primitive(PrimKind::Int);
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(src, member(1, "Read", "method"));
    members.add_direct(tgt, member(2, "Read", "method"));
    types.insert(1, method_types(vec![], number_ty)); // src.Read(): number
    types.insert(2, method_types(vec![], string_ty)); // tgt.Read(): string
    let lookup = SubtypeFixture::new();
    assert_ne!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Yes,
        "Read(): number must not satisfy Read(): string"
    );
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown
    );
}

#[test]
fn structural_missing_member_type_data_is_unknown_not_yes() {
    // The matched members agree on name and kind, but NEITHER side has recorded
    // SymbolTypeData. The pre-fix code declared this a Yes on name+kind alone —
    // the exact unsound path. With the fix, absent type data → Unknown.
    let mut arena = TypeArena::new();
    let src = arena.class("S");
    let tgt = arena.class("Writer");
    let mut members = MembersIndex::new();
    members.add_direct(src, member(1, "Write", "method"));
    members.add_direct(tgt, member(2, "Write", "method"));
    // No SymbolTypeData inserted for either member id.
    let types = SymbolTypeMap::new();
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown,
        "name+kind match with no recorded type data must be Unknown, never Yes"
    );
}

#[test]
fn structural_one_side_missing_member_type_data_is_unknown() {
    // Only the source member has recorded type data; the target's is absent.
    // The missing-type-info rule applies to EITHER side → Unknown.
    let mut arena = TypeArena::new();
    let src = arena.class("S");
    let tgt = arena.class("Writer");
    let string_ty = arena.primitive(PrimKind::Str);
    let unit = arena.class("void");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(src, member(1, "Write", "method"));
    members.add_direct(tgt, member(2, "Write", "method"));
    types.insert(1, method_types(vec![string_ty], unit));
    // tgt member (id 2) has no recorded type data.
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Unknown,
        "type data on only one side must be Unknown, never Yes"
    );
}

#[test]
fn structural_matching_param_and_return_types_is_yes() {
    // Positive companion to the incompatible cases: target Writer{ Write(s:
    // string): void } and source S{ Write(s: string): void } with matching
    // param and return types — the member-type comparison passes, so the arm
    // yields Yes.
    let mut arena = TypeArena::new();
    let src = arena.class("S");
    let tgt = arena.class("Writer");
    let string_ty = arena.primitive(PrimKind::Str);
    let unit = arena.class("void");
    let mut members = MembersIndex::new();
    let mut types = SymbolTypeMap::new();
    members.add_direct(src, member(1, "Write", "method"));
    members.add_direct(tgt, member(2, "Write", "method"));
    types.insert(1, method_types(vec![string_ty], unit));
    types.insert(2, method_types(vec![string_ty], unit));
    let lookup = SubtypeFixture::new();
    assert_eq!(
        is_assignable_to_typed(src, tgt, &arena, &lookup, &members, &types),
        SubtypeResult::Yes
    );
}
