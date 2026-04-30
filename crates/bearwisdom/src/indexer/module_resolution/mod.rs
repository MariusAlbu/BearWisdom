// =============================================================================
// indexer/module_resolution/mod.rs — ModuleResolver trait and dispatch
//
// Each language ecosystem has a dedicated resolver that maps a module specifier
// (as it appears in an import statement) to an actual indexed file path.
//
// `all_resolvers` returns one instance per ecosystem. `resolve_module_to_file`
// is a convenience wrapper for one-off lookups.
// =============================================================================

pub mod dotnet;
pub mod go_mod;
pub mod jvm;
pub mod node;
pub mod php_mod;
pub mod python_mod;
pub mod ruby_mod;
pub mod rust_mod;

// ---------------------------------------------------------------------------
// FilePathIndex — O(1) suffix lookup replacing O(N) linear scans
// ---------------------------------------------------------------------------

/// Pre-built index of all known file paths that replaces the O(N × M) linear
/// `file_paths.iter().find(…)` pattern used by all `ModuleResolver` impls.
///
/// Construction: O(N × depth) where `depth` is the average number of path
/// segments per file (typically 4-8). Lookup: O(1) hash lookup per candidate.
///
/// The suffix multi-map indexes every file by all of its suffix segments so
/// that `"src/components/Button.tsx"` is reachable via lookups for any of:
/// - `"Button.tsx"`
/// - `"components/Button.tsx"`
/// - `"src/components/Button.tsx"`
///
/// When multiple paths share the same suffix, the first insertion wins for
/// the map-lookup path; the Vec form exposes all candidates for callers
/// that need to enumerate them.
pub struct FilePathIndex {
    /// Full normalized (forward-slash) paths in the original order.
    pub normalized: Vec<String>,
    /// suffix_to_first: the first indexed file that ends with "/" + suffix,
    /// or equals the suffix exactly. Used for single-hit lookups.
    suffix_to_first: rustc_hash::FxHashMap<String, usize>,
    /// Exact full-path lookup: normalized path → Vec index. O(1) for exact matches.
    exact: rustc_hash::FxHashMap<String, usize>,
}

impl FilePathIndex {
    /// Build from a slice of raw file paths (may use `\` or `/`).
    pub fn build(file_paths: &[&str]) -> Self {
        let mut normalized: Vec<String> = Vec::with_capacity(file_paths.len());
        let mut suffix_to_first: rustc_hash::FxHashMap<String, usize> =
            rustc_hash::FxHashMap::default();
        let mut exact: rustc_hash::FxHashMap<String, usize> =
            rustc_hash::FxHashMap::default();

        for &raw in file_paths {
            let norm: String = raw.replace('\\', "/");
            let idx = normalized.len();

            exact.entry(norm.clone()).or_insert(idx);

            // Index every non-trivial suffix: split on '/', build all suffixes
            // from length 1 to full path, insert the first occurrence only.
            let parts: Vec<&str> = norm.split('/').collect();
            for start in 0..parts.len() {
                let suffix = parts[start..].join("/");
                suffix_to_first.entry(suffix).or_insert(idx);
            }

            normalized.push(norm);
        }

        Self {
            normalized,
            suffix_to_first,
            exact,
        }
    }

    /// Return the original (non-normalized) path string for index `idx`.
    #[inline]
    pub fn original(&self, idx: usize) -> &str {
        // `normalized` carries the forward-slash form. This is used as the
        // return value from resolvers — the callers already normalize or
        // accept forward slashes.
        &self.normalized[idx]
    }

    /// Find the first indexed path that exactly equals `candidate` (after
    /// forward-slash normalization).
    #[inline]
    pub fn find_exact(&self, candidate: &str) -> Option<&str> {
        let c = candidate.replace('\\', "/");
        self.exact.get(c.as_str()).map(|&i| self.original(i))
    }

    /// Find the first indexed path that equals `candidate` OR ends with
    /// `"/" + candidate` (path-suffix match, forward-slash normalized).
    /// This is the O(1) replacement for the `ends_with` loops that were
    /// previously O(N) across all file paths.
    #[inline]
    pub fn find_suffix(&self, candidate: &str) -> Option<&str> {
        let c: String = candidate.replace('\\', "/");
        // The suffix map keys are already normalized; strip leading slashes
        // that sometimes appear in callers' constructed candidates.
        let c = c.trim_start_matches('/');
        self.suffix_to_first.get(c).map(|&i| self.original(i))
    }

    /// Return the original (un-normalized) path slice for use in the legacy
    /// `&[&str]` API. Callers that haven't been migrated yet can build this
    /// from the index at near-zero cost.
    pub fn as_str_slice(&self) -> Vec<&str> {
        self.normalized.iter().map(|s| s.as_str()).collect()
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Maps module specifiers to indexed file paths, grouped by language ecosystem.
///
/// Implementations are stateless — construct once, call `resolve_to_file` many
/// times. The resolver does **not** touch the filesystem; it matches specifiers
/// against the `file_paths` slice (all known indexed paths).
pub trait ModuleResolver: Send + Sync {
    /// Language identifiers handled by this resolver.
    /// Must match the language strings produced by the file-detection layer.
    fn language_ids(&self) -> &[&str];

    /// Attempt to resolve `specifier` (as written in an import) to an actual
    /// indexed file path.
    ///
    /// `importing_file` — the path of the file that contains the import.
    /// `file_paths`     — all indexed file paths (used for matching).
    ///
    /// Returns `None` when the specifier is external (third-party package) or
    /// cannot be resolved against the known file set.
    fn resolve_to_file(
        &self,
        specifier: &str,
        importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String>;

    /// O(1) variant of `resolve_to_file` using a pre-built `FilePathIndex`.
    ///
    /// The default implementation calls `resolve_to_file` with a slice
    /// derived from the index — callers pay only the O(N) scan cost for
    /// resolvers that haven't overridden this method.
    ///
    /// Language plugins with dense relative-import traffic (TypeScript, JS)
    /// should override this to use `index.find_exact` / `index.find_suffix`
    /// and avoid the O(N) scan entirely.
    fn resolve_to_file_indexed(
        &self,
        specifier: &str,
        importing_file: &str,
        index: &FilePathIndex,
    ) -> Option<String> {
        let slice = index.as_str_slice();
        self.resolve_to_file(specifier, importing_file, &slice)
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// All registered `ModuleResolver` implementations in stable order.
pub fn all_resolvers() -> Vec<Box<dyn ModuleResolver>> {
    vec![
        Box::new(node::NodeModuleResolver),
        Box::new(rust_mod::RustModuleResolver),
        Box::new(python_mod::PythonModuleResolver),
        Box::new(go_mod::GoModuleResolver::new(None)),
        Box::new(jvm::JvmModuleResolver),
        Box::new(dotnet::DotNetModuleResolver),
        Box::new(php_mod::PhpModuleResolver),
        Box::new(ruby_mod::RubyModuleResolver),
    ]
}

/// All resolvers with an optional Go module path override.
///
/// The Go resolver needs the go.mod `module` directive to distinguish internal
/// imports from external ones. Pass `None` when not available.
pub fn all_resolvers_with_go_module(go_module_path: Option<&str>) -> Vec<Box<dyn ModuleResolver>> {
    vec![
        Box::new(node::NodeModuleResolver),
        Box::new(rust_mod::RustModuleResolver),
        Box::new(python_mod::PythonModuleResolver),
        Box::new(go_mod::GoModuleResolver::new(go_module_path.map(str::to_string))),
        Box::new(jvm::JvmModuleResolver),
        Box::new(dotnet::DotNetModuleResolver),
        Box::new(php_mod::PhpModuleResolver),
        Box::new(ruby_mod::RubyModuleResolver),
    ]
}

// ---------------------------------------------------------------------------
// Convenience
// ---------------------------------------------------------------------------

/// Resolve a single module specifier using the first resolver that matches
/// `language`. Returns `None` if no resolver handles the language or the
/// specifier is external/unresolvable.
pub fn resolve_module_to_file(
    language: &str,
    specifier: &str,
    importing_file: &str,
    file_paths: &[&str],
    resolvers: &[Box<dyn ModuleResolver>],
) -> Option<String> {
    resolvers
        .iter()
        .find(|r| r.language_ids().contains(&language))
        .and_then(|r| r.resolve_to_file(specifier, importing_file, file_paths))
}

/// Indexed variant of `resolve_module_to_file`. Uses `FilePathIndex` for O(1)
/// lookups in resolvers that override `resolve_to_file_indexed`.
pub fn resolve_module_to_file_indexed(
    language: &str,
    specifier: &str,
    importing_file: &str,
    index: &FilePathIndex,
    resolvers: &[Box<dyn ModuleResolver>],
) -> Option<String> {
    resolvers
        .iter()
        .find(|r| r.language_ids().contains(&language))
        .and_then(|r| r.resolve_to_file_indexed(specifier, importing_file, index))
}
