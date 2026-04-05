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
pub mod rust_mod;

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
