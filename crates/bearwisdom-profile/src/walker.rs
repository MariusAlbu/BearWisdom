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
use crate::exclusions::{
    project_exclude_dirs, should_exclude_in_project, should_skip_file,
};
use crate::types::ScannedFile;
use ignore::WalkBuilder;
use std::path::Path;

/// Remove the Windows UNC extended-length prefix (`\\?\` or `//?/`) from a
/// path string and return the plain absolute path.
///
/// `std::fs::canonicalize()` on Windows always returns a UNC path like
/// `\\?\F:\Work\Projects\...`.  The `ignore` crate walker, however, yields
/// entries under the *original* root (no UNC prefix).  This mismatch causes
/// `strip_prefix(root_normalized)` to fail silently, storing the full absolute
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

    // Use the original root (UNC-stripped) for prefix stripping.
    // The `ignore` crate walker returns entries based on the original root,
    // so using canonicalize() here would introduce mismatches on Windows
    // (e.g., 8.3 short names like RUNNER~1 vs. long names from canonicalize).
    let root_normalized = {
        let s = root.to_string_lossy();
        let stripped = strip_unc_prefix(&s);
        std::path::PathBuf::from(stripped)
    };

    // Build override rules to exclude project-scoped dirs at the walker
    // level. The exclusion set is the union of `COMMON_EXCLUDE_DIRS` and the
    // `exclude_dirs` of every language detected at `root` — NOT the global
    // union of every registered language. The global union excludes `build/`
    // (Java/Kotlin/Dart/C/C++) which incorrectly hides Cargo build-script
    // source in pure Rust projects (scryer-prolog, prost-build downstreams).
    //
    // This prevents the walker from entering these directories at all, which
    // is critical for performance (venv/ can have 10,000+ files).
    let project_excludes: Vec<&'static str> = project_exclude_dirs(root);
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    for dir in &project_excludes {
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
        // root_normalized (which was already stripped).
        let abs_path = {
            let raw = entry.path().to_string_lossy();
            let stripped = strip_unc_prefix(&raw);
            std::path::PathBuf::from(stripped)
        };

        // Belt-and-suspenders: check path components against the
        // project-scoped exclusion list even though the OverrideBuilder
        // rules should have caught most cases. Also checks for vendor
        // library directories (wwwroot/lib, public/vendor).
        let should_skip = {
            let rel = abs_path.strip_prefix(&root_normalized).unwrap_or(&abs_path);
            let components: Vec<_> = rel.components().collect();
            components.iter().any(|c| {
                c.as_os_str()
                    .to_str()
                    .is_some_and(|n| should_exclude_in_project(n, &project_excludes))
            }) || {
                // Check for vendor lib dirs: parent in WEB_ROOT + child in VENDOR_CHILD
                components.windows(2).any(|pair| {
                    let parent = pair[0].as_os_str().to_str().unwrap_or("");
                    let child = pair[1].as_os_str().to_str().unwrap_or("");
                    super::exclusions::WEB_ROOT_DIRS.contains(&parent)
                        && super::exclusions::VENDOR_CHILD_DIRS.contains(&child)
                })
            }
        };
        if should_skip {
            continue;
        }

        // Skip minified/bundled files (.min.js, .min.css, .bundle.js).
        if abs_path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(should_skip_file)
        {
            continue;
        }

        // Detect language — skip files with no recognised extension.
        let language_id = match detect_language(&abs_path) {
            Some(desc) => desc.id,
            None => continue,
        };

        // Build relative path with forward-slash normalization.
        let relative_path = abs_path
            .strip_prefix(&root_normalized)
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
#[path = "walker_tests.rs"]
mod tests;
