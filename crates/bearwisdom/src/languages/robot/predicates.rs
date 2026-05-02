// =============================================================================
// robot/predicates.rs — Robot Framework library name helpers
// =============================================================================

use crate::types::EdgeKind;

/// Normalize a Robot Framework keyword name for comparison.
/// Robot treats spaces and underscores as equivalent and is
/// case-insensitive. The BDD-style prefixes `Given`, `When`, `Then`,
/// `And`, `But` are stripped per the Robot Framework spec — these are
/// runtime decorators that are part of the call site syntax, not part
/// of the keyword's identity (`When I park` calls the keyword `I park`).
pub(super) fn normalize_robot_name(name: &str) -> String {
    let stripped = strip_bdd_prefix(name);
    stripped
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '_' { '_' } else { c })
        .collect()
}

/// Strip a leading `Given `, `When `, `Then `, `And `, or `But ` from a
/// Robot Framework keyword name (case-insensitive). Returns the stripped
/// suffix or the input unchanged if no BDD prefix matches.
///
/// Per the Robot Framework spec, these prefixes are runtime decorators
/// that don't participate in keyword identity. A call written
/// `When I park ...` invokes a keyword DEFINED as `I park`. Without
/// stripping, every BDD-style call lands in unresolved_refs because no
/// `When I park` keyword exists.
pub(super) fn strip_bdd_prefix(name: &str) -> &str {
    const PREFIXES: &[&str] = &["Given ", "When ", "Then ", "And ", "But "];
    let lower = name.trim_start();
    let leading_ws_len = name.len() - lower.len();
    for prefix in PREFIXES {
        // Case-insensitive prefix match.
        if lower.len() >= prefix.len()
            && lower[..prefix.len()].eq_ignore_ascii_case(prefix)
        {
            // Return the slice past the prefix, preserving any leading
            // whitespace from the original input (rare but possible).
            return &name[leading_ws_len + prefix.len()..];
        }
    }
    name
}

/// Edge-kind / symbol-kind compatibility for Robot Framework.
#[allow(dead_code)]
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        _ => true,
    }
}

/// Well-known Robot Framework library names (external, not project code).
/// Used to classify qualified `Library.Keyword` references as external.
pub(super) fn is_robot_builtin_library(name: &str) -> bool {
    let norm = normalize_robot_name(name);
    matches!(
        norm.as_str(),
        "builtin"
            | "collections"
            | "string"
            | "operatingsystem"
            | "process"
            | "datetime"
            | "xml"
            | "json"
            | "browser"
            | "requestslibrary"
            | "seleniumlibrary"
            | "appiumlibrary"
            | "playwrightlibrary"
            | "browserlibrary"
            | "ftplibrary"
            | "imaplibrary"
            | "databaselibrary"
            | "exceldatalibrary"
            | "arquillian"
            | "robotframework_requests"
            | "robotframework_selenium2library"
    )
}

