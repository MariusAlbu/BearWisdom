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
    "node_modules", // also in TS/JS descriptors, but critical enough to be here too
    ".cache",
    ".tmp",
    "tmp",
    "temp",
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

/// Build an [`ignore::Walk`]-backed walker rooted at `root`.
///
/// - Respects `.gitignore` files (default `ignore` crate behaviour).
/// - Skips all canonical exclude dirs.
/// - Does not follow symlinks.
pub fn build_walker(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .standard_filters(true)
        .filter_entry(|entry| {
            // entry.file_name() is the bare name component.
            let name = entry.file_name().to_string_lossy();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                return !should_exclude(&name);
            }
            true
        });
    builder
}
