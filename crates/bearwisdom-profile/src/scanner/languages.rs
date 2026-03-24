use std::collections::HashMap;
use std::path::Path;

/// Walk the tree and count files per language id.
/// Returns `(language_id → file_count)`.
pub fn count_files_by_language(root: &Path) -> HashMap<String, usize> {
    let (counts, _) = count_files_with_manifest(root);
    counts
}

/// Walk the tree, count files per language id, and return the full file manifest.
/// Returns `(language_id → file_count, Vec<ScannedFile>)`.
pub fn count_files_with_manifest(
    root: &Path,
) -> (HashMap<String, usize>, Vec<crate::types::ScannedFile>) {
    let files = crate::walker::walk_files(root);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for f in &files {
        *counts.entry(f.language_id.to_owned()).or_insert(0) += 1;
    }
    (counts, files)
}
