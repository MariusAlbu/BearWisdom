// =============================================================================
// indexer/project_context.rs — Data-driven project context for resolution
//
// Scans manifest files (package.json, Cargo.toml, .csproj, go.mod, etc.) to
// build per-project knowledge about external dependencies and SDK configuration.
// This replaces hardcoded type/namespace maps with project-level data.
//
// The actual parsing logic lives in `manifest/` submodules. This module owns
// the `ProjectContext` type and the `build_project_context` entry point.
// Public parse functions are re-exported here for backward compatibility with
// existing resolver and test code that imports them via this path.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{debug, info};

use super::manifest::{self, ManifestData, ManifestKind};

// ---------------------------------------------------------------------------
// Re-exports — keep existing `project_context::parse_*` paths working
// ---------------------------------------------------------------------------

pub use super::manifest::cargo::parse_cargo_dependencies;
pub use super::manifest::composer::parse_composer_json_deps;
pub use super::manifest::gemfile::parse_gemfile_gems;
pub use super::manifest::go_mod::{find_go_mod, parse_go_mod, GoModData};
pub use super::manifest::gradle::parse_gradle_dependencies;
pub use super::manifest::maven::{extract_xml_text, parse_pom_xml_dependencies};
pub use super::manifest::npm::parse_package_json_deps;
pub use super::manifest::nuget::{
    implicit_usings_for_sdk, most_capable_sdk, parse_global_usings, parse_package_references,
    parse_sdk_type, DotnetSdkType,
};
pub use super::manifest::pyproject::{
    parse_pipfile_deps, parse_pyproject_deps, parse_requirements_txt,
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Project-level context built once per index, used by language resolvers
/// to classify external references without hardcoded maps.
#[derive(Debug, Clone, Default)]
pub struct ProjectContext {
    /// Namespace prefixes known to be external (from PackageReference + SDK).
    /// e.g., {"System", "Microsoft", "Newtonsoft.Json", "MediatR"}
    pub external_prefixes: HashSet<String>,

    /// Global usings available to all files in the project.
    /// These are namespace strings (e.g., "System.Linq", "System.Threading.Tasks").
    pub global_usings: Vec<String>,

    /// npm package names known to be external (from package.json dependencies).
    /// e.g., {"react", "express", "@tanstack/react-query", "@tanstack"}
    /// Bare specifiers matching any of these are classified as external imports.
    pub ts_packages: HashSet<String>,

    /// Go module path from go.mod (e.g., "code.gitea.io/gitea", "github.com/mattermost/mattermost-server").
    /// Any import path starting with this prefix is internal to the project.
    /// `None` when no go.mod was found.
    pub go_module_path: Option<String>,

    /// Rust crate names from Cargo.toml [dependencies] and [dev-dependencies].
    /// e.g., {"serde", "tokio", "axum", "sqlx"}
    /// Used by the Rust resolver to classify `use` paths as external.
    pub rust_crates: HashSet<String>,

    /// Python package names from pyproject.toml / requirements.txt / Pipfile / setup.py.
    /// e.g., {"django", "fastapi", "sqlalchemy", "pytest"}
    /// Used by the Python resolver to classify imports as external.
    pub python_packages: HashSet<String>,

    /// Ruby gem names from Gemfile (e.g., {"rails", "devise", "sidekiq"}).
    /// Also includes stdlib gem names. Used by the Ruby resolver to classify
    /// require paths as external.
    pub ruby_gems: HashSet<String>,

    /// PHP package names from composer.json require / require-dev
    /// (e.g., {"laravel/framework", "phpunit/phpunit"}).
    /// Used by the PHP resolver to classify use-statement namespaces as external.
    pub php_packages: HashSet<String>,

    /// Raw manifest data keyed by ecosystem, populated by `read_all_manifests`.
    /// Readers for new ecosystems (Swift, Dart, Elixir) can be consumed here
    /// without adding new top-level fields to this struct.
    pub manifests: HashMap<ManifestKind, ManifestData>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a `ProjectContext` by scanning the project root for manifest files.
///
/// Uses the registered `ManifestReader` implementations in `manifest/` to
/// locate and parse all ecosystem manifests, then back-fills the per-language
/// fields on `ProjectContext` for backward compatibility with existing resolvers.
pub fn build_project_context(project_root: &Path) -> ProjectContext {
    let mut ctx = ProjectContext::default();

    // Always-external base prefixes for .NET projects.
    let base_prefixes = ["System", "Microsoft"];
    for p in &base_prefixes {
        ctx.external_prefixes.insert(p.to_string());
    }

    // Run all manifest readers.
    let manifests = manifest::read_all_manifests(project_root);

    // --- NuGet / .NET ---
    if let Some(nuget) = manifests.get(&ManifestKind::NuGet) {
        for pkg in &nuget.dependencies {
            ctx.external_prefixes.insert(pkg.clone());
            if let Some(root) = pkg.split('.').next() {
                ctx.external_prefixes.insert(root.to_string());
            }
        }
        for ns in &nuget.global_usings {
            if !ctx.global_usings.contains(ns) {
                ctx.global_usings.push(ns.clone());
            }
        }
        // All global usings also imply external prefixes.
        for ns in &ctx.global_usings {
            if let Some(root) = ns.split('.').next() {
                ctx.external_prefixes.insert(root.to_string());
            }
        }
    }

    // --- npm / TypeScript / JavaScript ---
    if let Some(npm) = manifests.get(&ManifestKind::Npm) {
        ctx.ts_packages.extend(npm.dependencies.iter().cloned());
    }

    // --- Go ---
    if let Some(go) = manifests.get(&ManifestKind::GoMod) {
        if let Some(ref module_path) = go.module_path {
            debug!("Go module path: {module_path}");
            ctx.go_module_path = Some(module_path.clone());
        }
        for external in &go.dependencies {
            ctx.external_prefixes.insert(external.clone());
        }
    }

    // --- Cargo / Rust ---
    if let Some(cargo) = manifests.get(&ManifestKind::Cargo) {
        ctx.rust_crates.extend(cargo.dependencies.iter().cloned());
    }

    // --- Python ---
    if let Some(py) = manifests.get(&ManifestKind::PyProject) {
        ctx.python_packages.extend(py.dependencies.iter().cloned());
    }

    // --- Gemfile / Ruby ---
    if let Some(gemfile) = manifests.get(&ManifestKind::Gemfile) {
        ctx.ruby_gems.extend(gemfile.dependencies.iter().cloned());
    }

    // --- Composer / PHP ---
    if let Some(composer) = manifests.get(&ManifestKind::Composer) {
        ctx.php_packages.extend(composer.dependencies.iter().cloned());
    }

    // --- Java (Maven + Gradle) ---
    // Always-external Java roots.
    for prefix in &["java", "javax", "jakarta", "sun", "com.sun", "org.junit"] {
        ctx.external_prefixes.insert(prefix.to_string());
    }
    if let Some(maven) = manifests.get(&ManifestKind::Maven) {
        for group_id in &maven.dependencies {
            ctx.external_prefixes.insert(group_id.clone());
            if let Some(root_prefix) = group_id.split('.').next() {
                ctx.external_prefixes.insert(root_prefix.to_string());
            }
        }
    }
    if let Some(gradle) = manifests.get(&ManifestKind::Gradle) {
        for group_id in &gradle.dependencies {
            ctx.external_prefixes.insert(group_id.clone());
            if let Some(root_prefix) = group_id.split('.').next() {
                ctx.external_prefixes.insert(root_prefix.to_string());
            }
        }
    }

    ctx.manifests = manifests;

    info!(
        "ProjectContext: {} external prefixes, {} global usings, {} ts_packages, {} rust_crates, {} python_packages, {} ruby_gems, {} php_packages",
        ctx.external_prefixes.len(),
        ctx.global_usings.len(),
        ctx.ts_packages.len(),
        ctx.rust_crates.len(),
        ctx.python_packages.len(),
        ctx.ruby_gems.len(),
        ctx.php_packages.len(),
    );
    debug!(
        "External prefixes: {:?}",
        {
            let mut sorted: Vec<_> = ctx.external_prefixes.iter().collect();
            sorted.sort();
            sorted
        }
    );

    ctx
}

// ---------------------------------------------------------------------------
// ProjectContext helpers for resolvers
// ---------------------------------------------------------------------------

impl ProjectContext {
    /// Check whether a bare module specifier is an external npm package or Node.js built-in.
    ///
    /// Handles exact matches and scoped package prefix matches:
    /// - `"react"` matches `"react"` exactly
    /// - `"@tanstack/react-query"` matches `"@tanstack/react-query"` exactly
    /// - A bare specifier starting with `"node:"` is always external
    pub fn is_external_ts_package(&self, specifier: &str) -> bool {
        // node: protocol imports are always external.
        if specifier.starts_with("node:") {
            return true;
        }
        // Exact match (covers both bare names and scoped packages).
        if self.ts_packages.contains(specifier) {
            return true;
        }
        // Deep import path: `@mui/material/Box` should match `@mui/material`,
        // `react-dom/client` should match `react-dom`.
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if self.ts_packages.contains(path) {
                return true;
            }
        }
        false
    }

    /// Check whether a Go import path is external to the project.
    ///
    /// An import path is internal if it starts with the project's own module path
    /// (from go.mod). Everything else is external.
    ///
    /// If no go.mod was found, falls back to checking whether the path looks like
    /// a third-party module (contains a dot in the host segment, e.g., "github.com/...").
    pub fn is_external_go_import(&self, import_path: &str) -> bool {
        if let Some(ref module_path) = self.go_module_path {
            if import_path == module_path {
                return false;
            }
            if import_path.starts_with(module_path.as_str())
                && import_path.len() > module_path.len()
                && import_path.as_bytes()[module_path.len()] == b'/'
            {
                return false;
            }
            return true;
        }

        // No go.mod found — use heuristic.
        let first_segment = import_path.split('/').next().unwrap_or(import_path);
        first_segment.contains('.')
    }

    /// Check whether a Rust crate name is external (from Cargo.toml deps).
    ///
    /// Also returns `true` for the standard crate trilogy (std/core/alloc) which
    /// are never in Cargo.toml but are always external.
    pub fn is_external_rust_crate(&self, name: &str) -> bool {
        matches!(name, "std" | "core" | "alloc")
            || self.rust_crates.contains(name)
            // Crate names may use hyphens in Cargo.toml but underscores in source.
            || self.rust_crates.contains(&name.replace('_', "-"))
    }

    /// Check whether a Python package/module name is external (from manifests).
    pub fn is_external_python_package(&self, name: &str) -> bool {
        self.python_packages.contains(name)
            // pip packages may use hyphens; Python imports use underscores.
            || self.python_packages.contains(&name.replace('_', "-"))
    }

    /// Check whether a namespace is external based on the project's package references.
    pub fn is_external_namespace(&self, ns: &str) -> bool {
        // Check exact match first.
        if self.external_prefixes.contains(ns) {
            return true;
        }
        // Check prefix match: "System.Linq" matches prefix "System".
        for prefix in &self.external_prefixes {
            if ns.starts_with(prefix.as_str())
                && ns.len() > prefix.len()
                && ns.as_bytes()[prefix.len()] == b'.'
            {
                return true;
            }
        }
        false
    }

    /// Check whether a Ruby require path refers to an external gem.
    pub fn is_external_ruby_gem(&self, require_path: &str) -> bool {
        let gem_root = require_path.split('/').next().unwrap_or(require_path);
        self.ruby_gems.contains(gem_root) || self.ruby_gems.contains(require_path)
    }

    /// Check whether a PHP composer package name is external.
    pub fn is_external_php_package(&self, package: &str) -> bool {
        self.php_packages.contains(package)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "project_context_tests.rs"]
mod tests;
