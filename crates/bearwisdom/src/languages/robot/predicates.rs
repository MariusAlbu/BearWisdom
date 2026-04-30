// =============================================================================
// robot/predicates.rs — Robot Framework library name helpers
// =============================================================================

use crate::types::EdgeKind;

/// Normalize a Robot Framework keyword name for comparison.
/// Robot treats spaces and underscores as equivalent and is case-insensitive.
pub(super) fn normalize_robot_name(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '_' { '_' } else { c })
        .collect()
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

