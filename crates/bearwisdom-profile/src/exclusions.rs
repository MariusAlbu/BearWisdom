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
/// **Prefer [`project_exclude_dirs`] for walks that have a project root.**
/// This function unions every language's exclude list — useful as a fallback
/// when the project context is unknown, but it leaks Java/Kotlin/Dart's
/// `build/` exclusion into pure Rust projects (where `build/` may legitimately
/// hold build-script source — e.g. scryer-prolog's `build/main.rs`).
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

/// Detect which languages are active for `root` based on the presence of
/// each language's `entry_point_files` (Cargo.toml, pom.xml, pubspec.yaml,
/// etc.). Probes both `root` itself and every immediate child directory so
/// monorepo layouts like `frontend/package.json` + `backend/Cargo.toml` are
/// covered.
///
/// Returns a deduplicated set of language ids.
pub fn project_active_languages(root: &Path) -> Vec<&'static str> {
    let mut active: BTreeSet<&'static str> = BTreeSet::new();

    let probe = |dir: &Path, out: &mut BTreeSet<&'static str>| {
        for lang in LANGUAGES {
            if out.contains(&lang.id) {
                continue;
            }
            for marker in lang.entry_point_files {
                if dir.join(marker).is_file() {
                    out.insert(lang.id);
                    break;
                }
            }
        }
    };

    probe(root, &mut active);

    // Immediate children — handles monorepo layouts. Skip COMMON exclusions
    // (`.git`, `node_modules`, etc.) so we don't probe inside vendored trees.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else { continue };
            if COMMON_EXCLUDE_DIRS.contains(&name_str) {
                continue;
            }
            probe(&entry.path(), &mut active);
        }
    }

    active.into_iter().collect()
}

/// Returns the project-scoped exclusion set: `COMMON_EXCLUDE_DIRS` plus the
/// `exclude_dirs` of every language detected at `root` (or its immediate
/// children). For a pure Rust project this excludes `target/` but NOT
/// `build/` — Cargo doesn't put output in `build/` and many Rust crates
/// (scryer-prolog, prost-build downstreams) put build-script source there.
///
/// Falls back to [`canonical_exclude_dirs`] when no language could be
/// detected (rare — the project has no manifest and we have no signal).
pub fn project_exclude_dirs(root: &Path) -> Vec<&'static str> {
    let active = project_active_languages(root);
    if active.is_empty() {
        return canonical_exclude_dirs();
    }

    let mut set: BTreeSet<&'static str> = BTreeSet::new();
    for &dir in COMMON_EXCLUDE_DIRS {
        set.insert(dir);
    }
    for lang in LANGUAGES {
        if !active.contains(&lang.id) {
            continue;
        }
        for &dir in lang.exclude_dirs {
            set.insert(dir);
        }
    }
    set.into_iter().collect()
}

/// Returns true if `name` matches any canonical exclude dir.
///
/// `name` should be the bare directory name (not a full path).
///
/// **This is the global-union check** — used by call sites that don't carry
/// a project root. Prefer [`should_exclude_in_project`] when you have one;
/// the global form excludes `build/`, `out/`, `.gradle` for every project,
/// even pure Rust ones, which is wrong (see [`project_exclude_dirs`]).
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

/// Project-aware variant of [`should_exclude`]. Excludes only directories
/// the active languages claim — `target/` for Rust, `build/`/`out/` for
/// Java, `.dart_tool/` for Dart, etc. Plus `COMMON_EXCLUDE_DIRS`.
pub fn should_exclude_in_project(name: &str, exclude_dirs: &[&'static str]) -> bool {
    if COMMON_EXCLUDE_DIRS.contains(&name) {
        return true;
    }
    exclude_dirs.iter().any(|d| *d == name)
}

/// Directory names that are too ambiguous to exclude at any depth.
/// `vendor`, `lib`, `libs` mean "package-manager output" only when they
/// sit directly under the project root (Bundler's `vendor/`, Composer's
/// `vendor/`, Go's `vendor/`). Nested copies often hold real source —
/// e.g. Jekyll themes ship `_sass/<theme>/vendor/<lib>/` with mixin
/// libraries (breakpoint, susy, bourbon) that the project's own
/// `@import`s pull from.
///
/// `should_exclude_in_project_path` checks this set against the component
/// position: only the FIRST path component (depth 1) gets the exclusion;
/// nested copies pass through.
pub static ROOT_ONLY_EXCLUDE_NAMES: &[&str] = &[
    "vendor",
    "lib",
    "libs",
];

/// Path-aware variant of [`should_exclude_in_project`]. Returns true
/// when ANY component of `rel_path` is excluded — but for names in
/// [`ROOT_ONLY_EXCLUDE_NAMES`], only the first (depth-1) component
/// triggers the exclusion. Nested directories with the same name are
/// indexed.
///
/// `rel_path_components` is the sequence of path-component basenames
/// from the project root, e.g. `["_sass", "minimal-mistakes", "vendor",
/// "breakpoint"]`.
pub fn should_exclude_in_project_path(
    rel_path_components: &[&str],
    exclude_dirs: &[&'static str],
) -> bool {
    for (i, comp) in rel_path_components.iter().enumerate() {
        let is_root_level = i == 0;
        if COMMON_EXCLUDE_DIRS.contains(comp) {
            return true;
        }
        if exclude_dirs.contains(comp) {
            // Loose name (`vendor`, `lib`, ...) — only exclude when it
            // sits directly under the project root.
            if ROOT_ONLY_EXCLUDE_NAMES.contains(comp) {
                if is_root_level {
                    return true;
                }
                continue;
            }
            // Strict name (`target`, `node_modules`, `__pycache__`, ...)
            // — exclude at any depth.
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
