//! Opaque virtual identities for demand-pulled external source files.
//!
//! Path-layout recognition belongs to the dedicated language plugin. The
//! indexer only normalizes filesystem separators and dispatches through the
//! registry; unrecognized paths fall back to `ext:idx:` at the caller.

use std::path::Path;

/// Ask the active language owner for the virtual identity corresponding to a
/// pulled external file. Unknown languages and unrecognized layouts decline.
pub(crate) fn virtual_path_for_pulled(abs: &Path, language: &str) -> Option<String> {
    let normalized = abs.to_string_lossy().replace('\\', "/");
    crate::languages::default_registry().external_virtual_path(language, &normalized)
}

#[cfg(test)]
#[path = "ext_virtual_path_tests.rs"]
mod tests;

/// Whether `path` is a virtual external path (the `ext:<ecosystem>:...` scheme
/// every locator assigns to supply files) rather than a project-relative one.
pub(crate) fn is_virtual_external(path: &str) -> bool {
    path.starts_with("ext:")
}
