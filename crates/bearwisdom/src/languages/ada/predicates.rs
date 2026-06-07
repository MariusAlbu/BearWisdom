// =============================================================================
// ada/predicates.rs — Ada builtin predicates
// =============================================================================

/// Names defined by the Ada language runtime that are implicitly visible bare
/// and are never project symbols: the modular-type primitive operations from
/// `Interfaces` (RM 13.7) and the predefined numeric/character/string scalar
/// types from `Standard`. Declined before the resolution ladder (via the
/// profile's `builtin_skip`) so they are classified builtin rather than counted
/// as unresolved project refs. The common scalars (`Integer`, `Float`,
/// `Boolean`, `Character`, `String`) are handled by `primitive_mapping`, not
/// here.
/// A predefined Ada scalar/string type (`Standard`) or a numeric variant the
/// runtime supplies — never a project record the chain walker should index a
/// field-type edge to. The common scalars are also in `primitive_mapping`; the
/// rest fold in via [`is_ada_builtin`]. Used to suppress field/object TypeRef
/// emission for primitive-typed declarations, which carry no members to walk.
pub(super) fn is_ada_predefined_type(name: &str) -> bool {
    matches!(
        name,
        "Integer" | "Natural" | "Positive" | "Float" | "Boolean" | "Character" | "String"
    ) || is_ada_builtin(name)
}

pub(super) fn is_ada_builtin(name: &str) -> bool {
    matches!(
        name,
        // Interfaces — modular-type primitive operations (RM 13.7).
        "Shift_Left"
            | "Shift_Right"
            | "Shift_Right_Arithmetic"
            | "Rotate_Left"
            | "Rotate_Right"
            // Standard — predefined scalar / character / string types.
            | "Long_Integer"
            | "Long_Long_Integer"
            | "Short_Integer"
            | "Short_Short_Integer"
            | "Integer_8"
            | "Integer_16"
            | "Integer_32"
            | "Integer_64"
            | "Unsigned_8"
            | "Unsigned_16"
            | "Unsigned_32"
            | "Unsigned_64"
            | "Long_Float"
            | "Long_Long_Float"
            | "Short_Float"
            | "Duration"
            | "Wide_Character"
            | "Wide_Wide_Character"
            | "Wide_String"
            | "Wide_Wide_String"
    )
}
