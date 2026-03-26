// =============================================================================
// walker.rs  —  gitignore-aware file discovery
//
// Delegates entirely to `bearwisdom_profile::walk_files`, which owns the
// walk logic (UNC stripping, OverrideBuilder exclusions, gitignore, sorting).
// This module exists to provide the `WalkedFile` type used by the indexer and
// the `detect_language` helper used by the parser layer.
// =============================================================================

use bearwisdom_profile::detect_language as profile_detect_language;
use bearwisdom_profile::ScannedFile;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// A file that was found and is ready to be parsed.
#[derive(Debug, Clone)]
pub struct WalkedFile {
    /// Path as stored in the DB (relative to project root, forward slashes).
    pub relative_path: String,
    /// Absolute path — used for reading file contents.
    pub absolute_path: PathBuf,
    /// Language identifier (e.g. "csharp", "typescript").
    pub language: &'static str,
}

impl From<&ScannedFile> for WalkedFile {
    fn from(sf: &ScannedFile) -> Self {
        WalkedFile {
            relative_path: sf.relative_path.clone(),
            absolute_path: sf.absolute_path.clone(),
            language: sf.language_id,
        }
    }
}

/// Walk `project_root` and return all indexable source files.
///
/// Delegates to `bearwisdom_profile::walk_files` for all discovery logic.
/// Files are sorted by relative path for deterministic output across OSes.
pub fn walk(project_root: &Path) -> Result<Vec<WalkedFile>> {
    let scanned = bearwisdom_profile::walk_files(project_root);
    Ok(scanned.iter().map(WalkedFile::from).collect())
}

/// Map a file path to a language identifier.
///
/// Returns `None` for paths we don't support so the caller can skip the file.
/// Delegates to `bearwisdom-profile` for all detection logic, preserving
/// the `Option<&'static str>` return type expected by callers.
pub fn detect_language(path: &Path) -> Option<&'static str> {
    profile_detect_language(path).map(|desc| desc.id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "walker_tests.rs"]
mod tests;
