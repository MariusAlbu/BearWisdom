// =============================================================================
// indexer/secondary_scan.rs — generic generated-source scan entry point
// =============================================================================

use std::path::Path;

use crate::walker::WalkedFile;

/// Collect additive generated-source candidates from the registered
/// ecosystem adapters. Provenance assignment remains adapter-owned.
pub fn pull_gitignored_imports(project_root: &Path, primary: &[WalkedFile]) -> Vec<WalkedFile> {
    crate::ecosystem::external_policy::scan_secondary_sources(project_root, primary)
}
