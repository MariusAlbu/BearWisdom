// =============================================================================
// groovy/predicates.rs — Groovy builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// True when a `method_invocation` name node is the GString interpolation
/// marker rather than a real callee.
///
/// A GString `"...${expr}..."` is parsed as `$` applied to a trailing closure
/// `{ expr }` (the `$` is a valid identifier and `{…}` becomes a closure body),
/// so the interpolation delimiter surfaces as a `method_invocation` whose name
/// is exactly `$`. Such a node never names a callable symbol; emitting a Calls
/// ref for it produces a target that can never resolve. An empty name is the
/// degenerate form of the same artifact.
pub(super) fn is_interpolation_marker(name: &str) -> bool {
    name.is_empty() || name == "$"
}

/// Groovy control flow keywords that the grammar may parse as method_invocation.
/// Used by the extractor to filter parser-noise refs at extract time.
pub(super) fn is_groovy_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "while"
            | "for"
            | "switch"
            | "case"
            | "do"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "return"
            | "break"
            | "continue"
            | "assert"
    )
}
