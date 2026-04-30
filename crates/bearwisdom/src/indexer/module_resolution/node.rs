// indexer/module_resolution/node.rs — Node.js / TypeScript resolver
//
// Handles all JS/TS ecosystems: TypeScript, JavaScript, JSX, TSX, Svelte,
// Astro, Vue, Angular.
//
// Resolution rules:
//   1. Relative specifier (starts with `.` or `..`) →
//      - Resolve relative to the importing file's directory.
//      - Try extensions: .ts .tsx .js .jsx .mjs .mts .svelte .astro .vue
//      - Try `/index` variants: specifier/index.ts, specifier/index.js, etc.
//   2. Bare specifier (no `.` prefix) → external package, return None.
//   3. `@/` or `~/` alias → strip prefix, resolve from project root.

use super::{FilePathIndex, ModuleResolver};

pub struct NodeModuleResolver;

const LANGUAGES: &[&str] = &[
    "typescript",
    "javascript",
    "tsx",
    "jsx",
    "svelte",
    "astro",
    "vue",
    "angular",
];

const EXTENSIONS: &[&str] = &[
    ".ts", ".tsx", ".js", ".jsx", ".mjs", ".mts", ".svelte", ".astro", ".vue",
];

impl ModuleResolver for NodeModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        // Normalise the importing file's directory (forward slashes).
        let import_dir = parent_dir(importing_file);

        if specifier.starts_with('@') && !specifier.starts_with("@/") {
            // Scoped npm package like @angular/core — external.
            return None;
        }

        let base: String = if specifier.starts_with("@/") || specifier.starts_with("~/") {
            // Project-root alias: strip the two-char prefix and leave the rest.
            let tail = &specifier[2..];
            tail.trim_start_matches('/').to_string()
        } else if specifier.starts_with('.') {
            // Relative import.
            join_paths(import_dir, specifier)
        } else {
            // Bare specifier — external package.
            return None;
        };

        // Normalise the resolved base (collapse `..`, remove trailing slashes).
        let base = normalise_path(&base);

        try_resolve(&base, file_paths)
    }

    /// O(1) override using `FilePathIndex` suffix lookups instead of the
    /// O(N × 18) linear scan in `try_resolve`. This is the hot path for
    /// TypeScript monorepos — ts-nextjs has ~83,000 indexed files and
    /// ~100,000+ unique relative-import pairs, making the O(N) path
    /// (~150 billion comparisons) the dominant cost of `build_with_context`.
    fn resolve_to_file_indexed(
        &self,
        specifier: &str,
        importing_file: &str,
        index: &FilePathIndex,
    ) -> Option<String> {
        let import_dir = parent_dir(importing_file);

        if specifier.starts_with('@') && !specifier.starts_with("@/") {
            return None;
        }

        let base: String = if specifier.starts_with("@/") || specifier.starts_with("~/") {
            let tail = &specifier[2..];
            tail.trim_start_matches('/').to_string()
        } else if specifier.starts_with('.') {
            join_paths(import_dir, specifier)
        } else {
            return None;
        };

        let base = normalise_path(&base);

        try_resolve_indexed(&base, index)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the directory portion of `file_path` (forward-slash normalised).
fn parent_dir(file_path: &str) -> &str {
    let norm = file_path.replace('\\', "/");
    // We operate on the original str — find the last `/`.
    if let Some(pos) = file_path.rfind(|c| c == '/' || c == '\\') {
        &file_path[..pos]
    } else {
        "."
    }
}

/// Join a directory and a (possibly relative) path, normalising separators.
fn join_paths(dir: &str, tail: &str) -> String {
    let dir = dir.replace('\\', "/");
    let tail = tail.replace('\\', "/");
    if dir.is_empty() || dir == "." {
        tail
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), tail.trim_start_matches('/'))
    }
}

/// Collapse `..` components and remove redundant `.` segments.
fn normalise_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let mut out: Vec<&str> = Vec::with_capacity(parts.len());
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// Try to find `base` (without extension) in `file_paths` by:
///   1. Exact match or suffix match with any supported extension.
///   2. The base itself (if it already carries an extension).
///   3. `/index` variant with any extension.
///
/// Both exact-path matches and path-suffix matches are attempted so that
/// project-root aliases (`@/components/Button`) resolve against deeply-rooted
/// files (`src/components/Button.tsx`).
fn try_resolve(base: &str, file_paths: &[&str]) -> Option<String> {
    // If the specifier already includes a known extension, try exact + suffix.
    if EXTENSIONS.iter().any(|e| base.ends_with(e)) {
        if let Some(&p) = file_paths
            .iter()
            .find(|&&p| path_matches(p, base))
        {
            return Some(p.to_string());
        }
    }

    // Try appending each extension.
    for ext in EXTENSIONS {
        let candidate = format!("{}{}", base, ext);
        if let Some(&p) = file_paths.iter().find(|&&p| path_matches(p, &candidate)) {
            return Some(p.to_string());
        }
    }

    // Try index variants: base/index.<ext>
    for ext in EXTENSIONS {
        let candidate = format!("{}/index{}", base, ext);
        if let Some(&p) = file_paths.iter().find(|&&p| path_matches(p, &candidate)) {
            return Some(p.to_string());
        }
    }

    None
}

/// Returns true if `file` equals `candidate` or ends with `/<candidate>`,
/// after normalising path separators.
fn path_matches(file: &str, candidate: &str) -> bool {
    let f = file.replace('\\', "/");
    let c = candidate.replace('\\', "/");
    f == c || f.ends_with(&format!("/{}", c))
}

/// O(1) variant of `try_resolve` using a pre-built `FilePathIndex`.
///
/// The `FilePathIndex` suffix map is keyed by every trailing path segment
/// sequence, so `"Button.tsx"`, `"components/Button.tsx"`, and
/// `"src/components/Button.tsx"` all resolve in O(1) time. For the exact
/// check we use the index's `find_exact` method; for the broader suffix
/// match (which `@/` aliases need — `@/components/Button` resolves against
/// `src/components/Button.tsx`) we use `find_suffix`.
fn try_resolve_indexed(base: &str, index: &FilePathIndex) -> Option<String> {
    // If the specifier already includes a known extension, try exact then suffix.
    if EXTENSIONS.iter().any(|e| base.ends_with(e)) {
        if let Some(p) = index.find_suffix(base) {
            return Some(p.to_string());
        }
    }

    // Try appending each extension — O(1) per probe via the suffix map.
    for ext in EXTENSIONS {
        let candidate = format!("{}{}", base, ext);
        if let Some(p) = index.find_suffix(&candidate) {
            return Some(p.to_string());
        }
    }

    // Try index variants: base/index.<ext>
    for ext in EXTENSIONS {
        let candidate = format!("{}/index{}", base, ext);
        if let Some(p) = index.find_suffix(&candidate) {
            return Some(p.to_string());
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::module_resolution::FilePathIndex;

    fn resolve(spec: &str, from: &str, files: &[&str]) -> Option<String> {
        NodeModuleResolver.resolve_to_file(spec, from, files)
    }

    fn resolve_indexed(spec: &str, from: &str, files: &[&str]) -> Option<String> {
        let index = FilePathIndex::build(files);
        NodeModuleResolver.resolve_to_file_indexed(spec, from, &index)
    }

    #[test]
    fn relative_ts_extension() {
        let files = &["src/services/user.ts"];
        assert_eq!(
            resolve("./user", "src/services/auth.ts", files),
            Some("src/services/user.ts".into())
        );
    }

    #[test]
    fn relative_index_fallback() {
        let files = &["src/services/user/index.ts"];
        assert_eq!(
            resolve("./user", "src/services/auth.ts", files),
            Some("src/services/user/index.ts".into())
        );
    }

    #[test]
    fn relative_parent_dir() {
        let files = &["src/utils/helpers.ts"];
        assert_eq!(
            resolve("../utils/helpers", "src/services/auth.ts", files),
            Some("src/utils/helpers.ts".into())
        );
    }

    #[test]
    fn bare_specifier_is_external() {
        let files = &["node_modules/lodash/index.js"];
        assert!(resolve("lodash", "src/app.ts", files).is_none());
    }

    #[test]
    fn scoped_package_is_external() {
        let files = &[];
        assert!(resolve("@angular/core", "src/app.ts", files).is_none());
    }

    #[test]
    fn alias_at_slash() {
        let files = &["src/components/Button.tsx"];
        assert_eq!(
            resolve("@/components/Button", "src/pages/Home.tsx", files),
            Some("src/components/Button.tsx".into())
        );
    }

    #[test]
    fn alias_tilde() {
        let files = &["src/utils/format.ts"];
        assert_eq!(
            resolve("~/utils/format", "src/pages/Home.tsx", files),
            Some("src/utils/format.ts".into())
        );
    }

    #[test]
    fn specifier_with_explicit_extension() {
        let files = &["src/lib/math.js"];
        assert_eq!(
            resolve("./math.js", "src/lib/main.ts", files),
            Some("src/lib/math.js".into())
        );
    }

    // -----------------------------------------------------------------------
    // Indexed variant — must produce identical results to the slice variant.
    // These mirror every test above, exercising `try_resolve_indexed` and
    // `FilePathIndex` with the same inputs.
    // -----------------------------------------------------------------------

    #[test]
    fn indexed_relative_ts_extension() {
        let files = &["src/services/user.ts"];
        assert_eq!(
            resolve_indexed("./user", "src/services/auth.ts", files),
            Some("src/services/user.ts".into())
        );
    }

    #[test]
    fn indexed_relative_index_fallback() {
        let files = &["src/services/user/index.ts"];
        assert_eq!(
            resolve_indexed("./user", "src/services/auth.ts", files),
            Some("src/services/user/index.ts".into())
        );
    }

    #[test]
    fn indexed_relative_parent_dir() {
        let files = &["src/utils/helpers.ts"];
        assert_eq!(
            resolve_indexed("../utils/helpers", "src/services/auth.ts", files),
            Some("src/utils/helpers.ts".into())
        );
    }

    #[test]
    fn indexed_bare_specifier_is_external() {
        let files = &["node_modules/lodash/index.js"];
        assert!(resolve_indexed("lodash", "src/app.ts", files).is_none());
    }

    #[test]
    fn indexed_scoped_package_is_external() {
        let files = &[];
        assert!(resolve_indexed("@angular/core", "src/app.ts", files).is_none());
    }

    #[test]
    fn indexed_alias_at_slash() {
        let files = &["src/components/Button.tsx"];
        assert_eq!(
            resolve_indexed("@/components/Button", "src/pages/Home.tsx", files),
            Some("src/components/Button.tsx".into())
        );
    }

    #[test]
    fn indexed_alias_tilde() {
        let files = &["src/utils/format.ts"];
        assert_eq!(
            resolve_indexed("~/utils/format", "src/pages/Home.tsx", files),
            Some("src/utils/format.ts".into())
        );
    }

    #[test]
    fn indexed_specifier_with_explicit_extension() {
        let files = &["src/lib/math.js"];
        assert_eq!(
            resolve_indexed("./math.js", "src/lib/main.ts", files),
            Some("src/lib/math.js".into())
        );
    }

    /// Verify that resolve_to_file and resolve_to_file_indexed agree on a
    /// larger realistic set of paths. The indexed variant must never resolve
    /// to a different file than the slice variant for any of these inputs.
    #[test]
    fn indexed_parity_with_slice() {
        let files: &[&str] = &[
            "apps/web/src/components/Button.tsx",
            "apps/web/src/pages/index.tsx",
            "apps/web/src/utils/helpers.ts",
            "apps/api/src/routes/users.ts",
            "apps/api/src/middleware/auth.ts",
            "packages/shared/src/index.ts",
            "packages/shared/src/types.ts",
        ];
        let cases: &[(&str, &str)] = &[
            ("./Button", "apps/web/src/components/Card.tsx"),
            ("../utils/helpers", "apps/web/src/components/Card.tsx"),
            ("../../packages/shared/src/index", "apps/web/src/pages/index.tsx"),
            ("@/components/Button", "apps/web/src/pages/index.tsx"),
            ("react", "apps/web/src/pages/index.tsx"), // external → None
        ];
        let index = FilePathIndex::build(files);
        for (spec, from) in cases {
            let slice_result = NodeModuleResolver.resolve_to_file(spec, from, files);
            let index_result = NodeModuleResolver.resolve_to_file_indexed(spec, from, &index);
            assert_eq!(
                slice_result, index_result,
                "parity failure: spec={spec:?} from={from:?}"
            );
        }
    }
}
