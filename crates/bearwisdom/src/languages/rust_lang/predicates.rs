// =============================================================================
// rust_lang/predicates.rs — Rust helper predicates for external classification
// and file-context construction.
//
// Kind-compatibility now lives in `RUST_PROFILE.kind_compatible_table`; the
// generic engine drives every binding decision. What remains here are the two
// path helpers the surviving hooks (`classify_external`, `build_file_context`)
// need: `::`→`.` normalization and the file's module-path derivation.
// =============================================================================

use crate::types::ParsedFile;

/// Extract the module path from a parsed Rust file.
/// The Rust extractor sets scope_path on top-level symbols to the module path,
/// e.g., "crate::models" or "crate::api::handlers".
pub(super) fn extract_module_path(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                // scope_path may use `::` or `.` separators — normalize to `.`
                let dot_path = sp.replace("::", ".");
                return Some(dot_path);
            }
        }
        // If no scope_path, check qualified_name prefix.
        if let Some(dot) = sym.qualified_name.rfind('.') {
            let prefix = &sym.qualified_name[..dot];
            if !prefix.is_empty() {
                return Some(prefix.to_string());
            }
        }
    }
    None
}

/// Normalize a Rust `::` path to the `.`-separated form used in the symbol index.
/// "crate::models::User" → "crate.models.User"
/// "serde::Deserialize"  → "serde.Deserialize"
pub(super) fn normalize_path(s: &str) -> String {
    s.replace("::", ".")
}
