// =============================================================================
// walker.rs  —  gitignore-aware file discovery
//
// Uses the `ignore` crate (same library that powers ripgrep) which handles:
//   - .gitignore at every directory level
//   - .git/info/exclude
//   - Global gitignore (~/.config/git/ignore)
//   - Hidden files and directories (excluded by default)
//
// Only files with a recognised language extension are included.
// Results are sorted by relative_path for deterministic output.
// =============================================================================

use crate::detect::detect_language;
use crate::exclusions::{canonical_exclude_dirs, should_exclude};
use crate::types::ScannedFile;
use ignore::WalkBuilder;
use std::path::Path;

/// Remove the Windows UNC extended-length prefix (`\\?\` or `//?/`) from a
/// path string and return the plain absolute path.
///
/// `std::fs::canonicalize()` on Windows always returns a UNC path like
/// `\\?\F:\Work\Projects\...`.  The `ignore` crate walker, however, yields
/// entries under the *original* root (no UNC prefix).  This mismatch causes
/// `strip_prefix(root_canonical)` to fail silently, storing the full absolute
/// path instead of the relative path.
fn strip_unc_prefix(s: &str) -> &str {
    s.strip_prefix(r"\\?\")
        .or_else(|| s.strip_prefix("//?/"))
        .unwrap_or(s)
}

/// Walk `root` and return all indexable source files.
///
/// Files are sorted by `relative_path` for deterministic output across OSes.
pub fn walk_files(root: &Path) -> Vec<ScannedFile> {
    let mut files = Vec::new();

    // Canonicalize and strip UNC prefix so strip_prefix succeeds on Windows.
    let root_canonical = {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let s = canonical.to_string_lossy();
        let stripped = strip_unc_prefix(&s);
        std::path::PathBuf::from(stripped)
    };

    // Build override rules to exclude canonical dirs at the walker level.
    // This prevents the walker from entering these directories at all, which
    // is critical for performance (venv/ can have 10,000+ files).
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    for dir in canonical_exclude_dirs() {
        // "!dir/" means "exclude this directory"
        let _ = overrides.add(&format!("!{dir}/"));
    }
    let overrides = overrides.build().unwrap_or_else(|_| {
        ignore::overrides::OverrideBuilder::new(root).build().unwrap()
    });

    let walker = WalkBuilder::new(root)
        .hidden(true) // skip dot-files and dot-dirs
        .git_ignore(true) // respect .gitignore
        .git_global(true) // respect global gitignore
        .git_exclude(true) // respect .git/info/exclude
        .follow_links(false) // never follow symlinks (security + no infinite loops)
        .max_depth(Some(50)) // safety cap
        .overrides(overrides)
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                tracing::debug!("Walk error (skipping): {err}");
                continue;
            }
        };

        // Skip directories — we only want files.
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        // entry.path() may return a Windows UNC path when the walker was
        // initialized with a UNC-prefixed root. Strip the prefix so it matches
        // root_canonical (which was already stripped).
        let abs_path = {
            let raw = entry.path().to_string_lossy();
            let stripped = strip_unc_prefix(&raw);
            std::path::PathBuf::from(stripped)
        };

        // Belt-and-suspenders: check path components against should_exclude
        // even though the OverrideBuilder rules should have caught most cases.
        let should_skip = {
            let rel = abs_path.strip_prefix(&root_canonical).unwrap_or(&abs_path);
            rel.components().any(|c| {
                c.as_os_str()
                    .to_str()
                    .is_some_and(should_exclude)
            })
        } || {
            // Fallback: check each ancestor directory name directly.
            abs_path.ancestors().any(|ancestor| {
                ancestor
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(should_exclude)
            })
        };
        if should_skip {
            continue;
        }

        // Detect language — skip files with no recognised extension.
        let language_id = match detect_language(&abs_path) {
            Some(desc) => desc.id,
            None => continue,
        };

        // Build relative path with forward-slash normalization.
        let relative_path = abs_path
            .strip_prefix(&root_canonical)
            .unwrap_or(&abs_path)
            .to_string_lossy()
            .replace('\\', "/");

        files.push(ScannedFile {
            relative_path,
            absolute_path: abs_path,
            language_id,
        });
    }

    // Deterministic order across OSes.
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    files
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_finds_source_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.ts"), "export const x = 1;").unwrap();
        std::fs::write(dir.path().join("image.png"), "binary").unwrap();

        let files = walk_files(dir.path());
        assert_eq!(files.len(), 2); // .rs and .ts, not .png
        assert!(files.iter().any(|f| f.language_id == "rust"));
        assert!(files.iter().any(|f| f.language_id == "typescript"));
    }

    #[test]
    fn walk_excludes_node_modules() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();
        let nm = dir.path().join("node_modules");
        std::fs::create_dir(&nm).unwrap();
        std::fs::write(nm.join("dep.ts"), "const y = 2;").unwrap();

        let files = walk_files(dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative_path, "app.ts");
    }

    #[test]
    fn walk_results_are_sorted() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("z.rs"), "").unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("m.rs"), "").unwrap();

        let files = walk_files(dir.path());
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn walk_normalizes_paths_to_forward_slashes() {
        let dir = tempfile::TempDir::new().unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("lib.rs"), "").unwrap();

        let files = walk_files(dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative_path, "src/lib.rs");
    }
}
