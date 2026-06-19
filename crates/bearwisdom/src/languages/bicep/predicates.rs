// =============================================================================
// bicep/predicates.rs — Bicep edge/symbol kind compatibility
//
// Builtin classification has moved out of this module. Bicep system-namespace
// functions, decorators, and ARM resource API methods are discovered at index
// time by the `bicep-runtime` ecosystem (see
// `crates/bearwisdom/src/ecosystem/bicep_runtime.rs`) — names come from a
// local Azure/bicep source clone. When no clone is present the names are
// genuinely unindexable from this machine and refs to them stay unresolved.
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "variable" | "function"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Returns true when `name` is an Azure resource type string — a slash-separated
/// provider/type reference, optional Bicep registry prefix, or AZ spec form.
pub(crate) fn is_azure_resource_type(name: &str) -> bool {
    let stripped = name.trim_matches('\'');
    if !stripped.contains('/') {
        return false;
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with("br:") || lower.starts_with("br/") || lower.starts_with("az:") {
        return true;
    }
    let head = stripped.split('/').next().unwrap_or("");
    if head.is_empty() {
        return false;
    }
    head.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '$' | '{' | '}'))
}
