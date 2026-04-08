use crate::registry::LANGUAGES;
use ignore::WalkBuilder;
use std::collections::BTreeSet;
use std::path::Path;

/// Directories that are always excluded, regardless of language.
pub static COMMON_EXCLUDE_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".idea",
    ".vscode",
    ".DS_Store",
    "__MACOSX",
    "node_modules",
    "bower_components",
    ".cache",
    ".tmp",
    "tmp",
    "temp",
];

/// File extensions that are always skipped (minified/bundled artifacts and source maps).
static SKIP_EXTENSIONS: &[&str] = &[
    ".min.js", ".min.css", ".min.mjs",
    ".bundle.js", ".bundle.css",
    ".chunk.js", ".chunk.css",
];

/// Returns the deduplicated, sorted union of `COMMON_EXCLUDE_DIRS` and all
/// `exclude_dirs` declared on every registered `LanguageDescriptor`.
///
/// Computed once at call-time; callers can cache if needed (cheap BTreeSet
/// dedup on ~35 string literals — no heap pressure in practice).
pub fn canonical_exclude_dirs() -> Vec<&'static str> {
    let mut set: BTreeSet<&'static str> = BTreeSet::new();

    for &dir in COMMON_EXCLUDE_DIRS {
        set.insert(dir);
    }

    for lang in LANGUAGES {
        for &dir in lang.exclude_dirs {
            set.insert(dir);
        }
    }

    set.into_iter().collect()
}

/// Returns true if `name` matches any canonical exclude dir.
///
/// `name` should be the bare directory name (not a full path).
pub fn should_exclude(name: &str) -> bool {
    // Common dirs are checked first (likely to match early).
    if COMMON_EXCLUDE_DIRS.contains(&name) {
        return true;
    }
    for lang in LANGUAGES {
        if lang.exclude_dirs.contains(&name) {
            return true;
        }
    }
    false
}

/// Returns true if the file name ends with a skippable extension (e.g. `.min.js`).
pub fn should_skip_file(name: &str) -> bool {
    SKIP_EXTENSIONS.iter().any(|ext| name.ends_with(ext))
}

/// Vendor library directories that should be excluded when nested under a
/// web-root directory (e.g. `wwwroot/lib/`, `public/vendor/`).
/// Checked via [`is_vendor_lib_dir`].
pub static WEB_ROOT_DIRS: &[&str] = &["wwwroot", "public"];
pub static VENDOR_CHILD_DIRS: &[&str] = &["lib", "vendor", "libs", "third_party", "third-party"];

/// Returns true if `path` looks like a vendored JS/CSS library directory
/// (e.g. `wwwroot/lib`, `public/vendor`).
fn is_vendor_lib_dir(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !VENDOR_CHILD_DIRS.contains(&name) {
        return false;
    }
    // Check if parent is a web-root directory.
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|parent| WEB_ROOT_DIRS.contains(&parent))
        .unwrap_or(false)
}

/// Build an [`ignore::Walk`]-backed walker rooted at `root`.
///
/// - Respects `.gitignore` files (default `ignore` crate behaviour).
/// - Skips all canonical exclude dirs and vendor library dirs.
/// - Skips minified files (`.min.js`, `.min.css`, `.bundle.js`).
/// - Does not follow symlinks.
pub fn build_walker(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .standard_filters(true)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if should_exclude(&name) {
                    return false;
                }
                // Exclude vendor library dirs (wwwroot/lib, public/vendor, etc.)
                if is_vendor_lib_dir(entry.path()) {
                    return false;
                }
                return true;
            }
            // Skip minified/bundled files.
            !should_skip_file(&name)
        });
    builder
}
