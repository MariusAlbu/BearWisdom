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
///
/// Special case: `.pp` is a shared extension between Puppet manifests and
/// Free Pascal source files. The profile matcher returns Puppet by default;
/// we override to Pascal when the file head shows clear Pascal markers
/// (`program`, `unit`, `library`, `{$mode`, etc.).
pub fn detect_language(path: &Path) -> Option<&'static str> {
    let lang = profile_detect_language(path).map(|desc| desc.id);
    if lang == Some("puppet") && is_likely_pascal(path) {
        return Some("pascal");
    }
    lang
}

/// Read the first 512 bytes of a `.pp` file and look for Pascal-source
/// markers. Catches `program`, `unit`, `library`, `{$mode ...}`, and the
/// `(* ... *)` block-comment header style; Puppet manifests use `class`,
/// `define`, `node`, `include`, `$var = ...` instead.
fn is_likely_pascal(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    use std::io::Read;
    let mut head = [0u8; 512];
    let n = match (&file).take(512).read(&mut head) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let text = String::from_utf8_lossy(&head[..n]);
    // Strip leading whitespace/comments aggressively.
    let mut t = text.as_ref();
    loop {
        let trimmed = t.trim_start();
        if let Some(rest) = trimmed.strip_prefix("//") {
            t = rest.split_once('\n').map(|(_, r)| r).unwrap_or("");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("(*") {
            t = rest.split_once("*)").map(|(_, r)| r).unwrap_or("");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('{') {
            // Pascal `{...}` block comment — skip past matching `}`.
            t = rest.split_once('}').map(|(_, r)| r).unwrap_or("");
            continue;
        }
        break;
    }
    let head_lower = t.trim_start().to_ascii_lowercase();
    head_lower.starts_with("program ")
        || head_lower.starts_with("unit ")
        || head_lower.starts_with("library ")
        || head_lower.starts_with("uses ")
        || head_lower.starts_with("interface\n")
        || head_lower.starts_with("interface\r\n")
        || head_lower.contains("{$mode ")
        || head_lower.contains("{$mode}")
        || head_lower.contains("{$mode:")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "walker_tests.rs"]
mod tests;
