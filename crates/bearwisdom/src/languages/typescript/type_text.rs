// =============================================================================
// languages/typescript/type_text — the source spellings this plugin owns
//
// One place answering "what does this piece of TypeScript type text mean": the
// surface forms the shared parser may recognize, the atoms that are semantic
// values rather than nominals, and the wrapper declaration that supplies a
// primitive's instance members.
// =============================================================================

use crate::languages::TypeTextPolicy;
use crate::type_checker::core::types::Intrinsic;

/// The two spellings that denote the ABSENCE of a value. They are declared with
/// the language because only it knows how it spells them; every consumer
/// reasons about the semantic atom instead.
///
/// The primitive spellings (`string`, `number`, …) are deliberately not atoms:
/// their instance members come from the wrapper declarations
/// [`primitive_member_head`] names, which the member walk reaches through the
/// nominal head.
const ABSENCE_ATOMS: &[(&str, Intrinsic)] = &[
    ("null", Intrinsic::Null),
    ("undefined", Intrinsic::Undefined),
];

/// The surface forms a TypeScript type expression may use. Every form is opted
/// into explicitly; anything outside the list stays an opaque nominal.
pub(super) fn policy() -> TypeTextPolicy {
    TypeTextPolicy {
        fat_arrow_function: true,
        readonly_modifier: true,
        array_suffix: true,
        union_intersection: true,
        bracket_tuple: true,
        angle_application: true,
        atoms: ABSENCE_ATOMS,
        ..TypeTextPolicy::OPAQUE
    }
}

/// The declaration carrying the instance members of a primitive spelling:
/// `string` values answer to `String`'s members.
pub(super) fn primitive_member_head(head: &str) -> Option<String> {
    match head {
        "string" => Some("String".to_string()),
        "number" => Some("Number".to_string()),
        "bigint" => Some("BigInt".to_string()),
        "boolean" => Some("Boolean".to_string()),
        "symbol" => Some("Symbol".to_string()),
        _ => None,
    }
}

/// Whether an applied head is a sequence whose computed access yields its
/// element type.
pub(super) fn has_homogeneous_computed_access(head: &str) -> bool {
    matches!(head, "Array" | "ReadonlyArray")
}

#[cfg(test)]
#[path = "type_text_tests.rs"]
mod tests;
