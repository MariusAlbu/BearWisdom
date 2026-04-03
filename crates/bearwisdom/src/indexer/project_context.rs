// =============================================================================
// indexer/project_context.rs — Data-driven project context for resolution
//
// Scans .csproj files (and equivalents for other languages) to build:
//   1. External namespace prefixes — derived from PackageReference + SDK
//   2. Global usings — from SDK implicit usings + GlobalUsings.cs files
//
// Also scans package.json files for TypeScript/JavaScript projects:
//   1. External package names — from dependencies + devDependencies
//   2. Node.js built-in module names (always added for TS/JS projects)
//
// Also scans go.mod files for Go projects:
//   1. Module path — the project's own import path prefix (internal boundary)
//   2. require block entries — external module paths added to external_prefixes
//
// This replaces hardcoded type/namespace maps with project-level data.
// =============================================================================

use std::collections::HashSet;
use std::path::Path;
use tracing::{debug, info, warn};

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
}

/// .NET SDK type, determines which implicit usings are injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotnetSdkType {
    /// Microsoft.NET.Sdk — console/library projects
    Base,
    /// Microsoft.NET.Sdk.Web — ASP.NET Core projects
    Web,
    /// Microsoft.NET.Sdk.Worker — background service projects
    Worker,
    /// Microsoft.NET.Sdk.BlazorWebAssembly
    Blazor,
    /// Unknown SDK string
    Other,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a `ProjectContext` by scanning the project root for .csproj files,
/// GlobalUsings.cs, and other metadata.
pub fn build_project_context(project_root: &Path) -> ProjectContext {
    let mut ctx = ProjectContext::default();

    // Always-external base prefixes for .NET projects.
    // These are the SDK and runtime namespaces — present in every .NET project.
    let base_prefixes = ["System", "Microsoft"];
    for p in &base_prefixes {
        ctx.external_prefixes.insert(p.to_string());
    }

    // Scan for .csproj files (.NET projects).
    let csproj_files = find_csproj_files(project_root);
    if !csproj_files.is_empty() {
        let mut sdk_types = Vec::new();

        for csproj_path in &csproj_files {
            let content = match std::fs::read_to_string(csproj_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Extract SDK type.
            if let Some(sdk) = parse_sdk_type(&content) {
                sdk_types.push(sdk);
            }

            // Extract PackageReference → external prefixes.
            for pkg in parse_package_references(&content) {
                // NuGet convention: package name = root namespace.
                // Extract the root prefix (first dotted segment or full name).
                ctx.external_prefixes.insert(pkg.clone());
                if let Some(root) = pkg.split('.').next() {
                    ctx.external_prefixes.insert(root.to_string());
                }
            }
        }

        // Determine the most capable SDK type across all projects.
        let sdk = most_capable_sdk(&sdk_types);

        // Add SDK implicit usings.
        let implicit = implicit_usings_for_sdk(sdk);
        for ns in implicit {
            if !ctx.global_usings.contains(&ns.to_string()) {
                ctx.global_usings.push(ns.to_string());
            }
        }

        // Scan for GlobalUsings.cs files.
        let global_using_files = find_global_using_files(project_root);
        for path in &global_using_files {
            if let Ok(content) = std::fs::read_to_string(path) {
                for ns in parse_global_usings(&content) {
                    if !ctx.global_usings.contains(&ns) {
                        ctx.global_usings.push(ns);
                    }
                }
            }
        }

        // All global usings also imply external prefixes (for the namespace itself).
        for ns in &ctx.global_usings {
            if let Some(root) = ns.split('.').next() {
                ctx.external_prefixes.insert(root.to_string());
            }
        }
    }

    // Scan for package.json files (TypeScript/JavaScript projects).
    let package_json_files = find_package_json_files(project_root);
    for path in &package_json_files {
        if let Ok(content) = std::fs::read_to_string(path) {
            for pkg in parse_package_json_deps(&content) {
                // For scoped packages like @tanstack/react-query, also add the scope.
                if pkg.starts_with('@') {
                    if let Some(scope) = pkg.split('/').next() {
                        ctx.ts_packages.insert(scope.to_string());
                    }
                }
                ctx.ts_packages.insert(pkg);
            }
        }
    }

    // Add Node.js built-in module names (always external for TS/JS projects).
    if !package_json_files.is_empty() {
        for builtin in NODE_BUILTINS {
            ctx.ts_packages.insert(builtin.to_string());
        }
        // Also add the node: protocol prefix as a sentinel.
        ctx.ts_packages.insert("node".to_string());
    }

    // Scan for go.mod (Go projects). go.mod always lives at the project root.
    if let Some(go_mod_path) = find_go_mod(project_root) {
        match std::fs::read_to_string(&go_mod_path) {
            Ok(content) => {
                let parsed = parse_go_mod(&content);
                if let Some(module_path) = parsed.module_path {
                    debug!("Go module path: {module_path}");
                    ctx.go_module_path = Some(module_path);
                }
                for external in parsed.require_paths {
                    // The host segment (e.g., "github.com") is not meaningful
                    // as a standalone prefix — store the full module path so
                    // the resolver can do exact-or-prefix matching.
                    ctx.external_prefixes.insert(external);
                }
            }
            Err(e) => warn!("Failed to read go.mod at {}: {e}", go_mod_path.display()),
        }
    }

    // Scan for Cargo.toml (Rust projects).
    scan_cargo_toml(project_root, &mut ctx);

    // Scan for Python manifest files.
    scan_python_manifests(project_root, &mut ctx);

    // Scan for Gemfile (Ruby projects).
    scan_gemfile(project_root, &mut ctx);

    // Scan for composer.json (PHP projects).
    scan_composer_json(project_root, &mut ctx);

    // Scan for pom.xml / build.gradle (Java projects).
    scan_java_manifests(project_root, &mut ctx);

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
// Node.js built-in modules (always external)
// ---------------------------------------------------------------------------

/// Node.js core module names. These are always external regardless of
/// whether they appear in package.json.
const NODE_BUILTINS: &[&str] = &[
    "assert",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "crypto",
    "dgram",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

// ---------------------------------------------------------------------------
// .csproj parsing (lightweight, no XML crate needed)
// ---------------------------------------------------------------------------

/// Find all .csproj files under the project root, excluding bin/obj.
fn find_csproj_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    collect_csproj(root, &mut result, 0);
    result
}

fn collect_csproj(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Skip common non-source directories.
            if matches!(
                name.as_ref(),
                "bin" | "obj" | "node_modules" | ".git" | "target"
                    | "packages" | ".vs" | "TestResults" | "artifacts"
            ) {
                continue;
            }
            collect_csproj(&path, out, depth + 1);
        } else if path.extension().is_some_and(|e| e == "csproj") {
            out.push(path);
        }
    }
}

/// Extract the SDK type from a .csproj file's `<Project Sdk="...">` attribute.
fn parse_sdk_type(content: &str) -> Option<DotnetSdkType> {
    // Match: <Project Sdk="Microsoft.NET.Sdk.Web">
    // Simple text search — no XML parser needed for this.
    let sdk_start = content.find("Sdk=\"")?;
    let rest = &content[sdk_start + 5..];
    let sdk_end = rest.find('"')?;
    let sdk_str = &rest[..sdk_end];

    Some(match sdk_str {
        "Microsoft.NET.Sdk" => DotnetSdkType::Base,
        "Microsoft.NET.Sdk.Web" => DotnetSdkType::Web,
        "Microsoft.NET.Sdk.Worker" => DotnetSdkType::Worker,
        "Microsoft.NET.Sdk.BlazorWebAssembly" => DotnetSdkType::Blazor,
        _ => DotnetSdkType::Other,
    })
}

/// Extract `<PackageReference Include="..." />` names from .csproj content.
fn parse_package_references(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let tag = "PackageReference";

    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(tag) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + tag.len();

        // Find Include="..." after the tag.
        let rest = &content[search_from..];
        // Look for Include= within the next 100 chars (same XML element).
        let window = &rest[..rest.len().min(200)];
        if let Some(inc_pos) = window.find("Include=\"") {
            let after_inc = &window[inc_pos + 9..];
            if let Some(end_quote) = after_inc.find('"') {
                let name = &after_inc[..end_quote];
                if !name.is_empty() {
                    packages.push(name.to_string());
                }
            }
        }
    }

    packages
}

/// Pick the "most capable" SDK from a list — Web > Worker > Blazor > Base.
fn most_capable_sdk(sdks: &[DotnetSdkType]) -> DotnetSdkType {
    if sdks.contains(&DotnetSdkType::Web) {
        DotnetSdkType::Web
    } else if sdks.contains(&DotnetSdkType::Worker) {
        DotnetSdkType::Worker
    } else if sdks.contains(&DotnetSdkType::Blazor) {
        DotnetSdkType::Blazor
    } else if sdks.contains(&DotnetSdkType::Base) {
        DotnetSdkType::Base
    } else {
        DotnetSdkType::Other
    }
}

/// Return the implicit usings for a given .NET SDK type.
/// These mirror what the SDK injects via `<ImplicitUsings>enable</ImplicitUsings>`.
pub fn implicit_usings_for_sdk(sdk: DotnetSdkType) -> Vec<&'static str> {
    // Base SDK (Microsoft.NET.Sdk) implicit usings — .NET 6+
    let mut usings = vec![
        "System",
        "System.Collections.Generic",
        "System.IO",
        "System.Linq",
        "System.Net.Http",
        "System.Threading",
        "System.Threading.Tasks",
    ];

    match sdk {
        DotnetSdkType::Web => {
            usings.extend_from_slice(&[
                "System.Net.Http.Json",
                "Microsoft.AspNetCore.Builder",
                "Microsoft.AspNetCore.Hosting",
                "Microsoft.AspNetCore.Http",
                "Microsoft.AspNetCore.Http.HttpResults",
                "Microsoft.AspNetCore.Mvc",
                "Microsoft.AspNetCore.Routing",
                "Microsoft.Extensions.Configuration",
                "Microsoft.Extensions.DependencyInjection",
                "Microsoft.Extensions.Hosting",
                "Microsoft.Extensions.Logging",
            ]);
        }
        DotnetSdkType::Worker => {
            usings.extend_from_slice(&[
                "Microsoft.Extensions.Configuration",
                "Microsoft.Extensions.DependencyInjection",
                "Microsoft.Extensions.Hosting",
                "Microsoft.Extensions.Logging",
            ]);
        }
        DotnetSdkType::Blazor => {
            usings.extend_from_slice(&[
                "System.Net.Http.Json",
                "Microsoft.AspNetCore.Components",
                "Microsoft.AspNetCore.Components.Forms",
                "Microsoft.AspNetCore.Components.Routing",
                "Microsoft.AspNetCore.Components.Web",
                "Microsoft.Extensions.Configuration",
                "Microsoft.Extensions.DependencyInjection",
                "Microsoft.Extensions.Logging",
            ]);
        }
        _ => {}
    }

    usings
}

// ---------------------------------------------------------------------------
// package.json parsing (lightweight, uses serde_json)
// ---------------------------------------------------------------------------

/// Find package.json files under the project root, skipping node_modules.
fn find_package_json_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    collect_package_json(root, &mut result, 0);
    result
}

fn collect_package_json(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Skip directories that definitely don't contain project package.json files.
            if matches!(
                name.as_ref(),
                "node_modules" | ".git" | "target" | "bin" | "obj" | ".next"
                    | "dist" | "build" | ".cache" | "coverage" | ".turbo"
            ) {
                continue;
            }
            collect_package_json(&path, out, depth + 1);
        } else if entry.file_name() == "package.json" {
            out.push(path);
        }
    }
}

/// Extract dependency package names from a package.json file's
/// `dependencies` and `devDependencies` objects.
///
/// Uses `serde_json` for parsing since it's already a workspace dependency.
pub fn parse_package_json_deps(content: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut packages = Vec::new();
    for key in &["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(serde_json::Value::Object(deps)) = obj.get(*key) {
            for pkg_name in deps.keys() {
                if !pkg_name.is_empty() {
                    packages.push(pkg_name.clone());
                }
            }
        }
    }
    packages
}

// ---------------------------------------------------------------------------
// go.mod parsing (lightweight, line-based)
// ---------------------------------------------------------------------------

/// Parsed data from a go.mod file.
pub struct GoModData {
    /// The `module` directive value (e.g., "code.gitea.io/gitea").
    pub module_path: Option<String>,
    /// All module paths from `require` blocks (e.g., "github.com/gin-gonic/gin").
    pub require_paths: Vec<String>,
}

/// Find go.mod, checking the project root first, then immediate subdirectories
/// (depth 1) to handle monorepos where Go lives in e.g. `server/` or `backend/`.
pub fn find_go_mod(root: &Path) -> Option<std::path::PathBuf> {
    let candidate = root.join("go.mod");
    if candidate.is_file() {
        return Some(candidate);
    }
    // Check one level of subdirectories for monorepo layouts.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let nested = entry.path().join("go.mod");
                if nested.is_file() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

/// Parse the `module` directive and `require` blocks from go.mod content.
///
/// go.mod format:
/// ```text
/// module code.gitea.io/gitea
///
/// go 1.21
///
/// require (
///     github.com/gin-gonic/gin v1.9.1
///     golang.org/x/crypto v0.14.0
/// )
///
/// require github.com/some/pkg v1.0.0
/// ```
pub fn parse_go_mod(content: &str) -> GoModData {
    let mut module_path: Option<String> = None;
    let mut require_paths = Vec::new();
    let mut in_require_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // `module <path>`
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let path = rest.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() {
                module_path = Some(path.to_string());
            }
            continue;
        }

        // `require (` — start of multi-line block.
        if trimmed == "require (" || trimmed.starts_with("require (") {
            in_require_block = true;
            continue;
        }

        // `)` — end of a block.
        if trimmed == ")" {
            in_require_block = false;
            continue;
        }

        // Single-line `require <path> <version>`.
        if let Some(rest) = trimmed.strip_prefix("require ") {
            let path = rest.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() && path != "(" {
                require_paths.push(path.to_string());
            }
            continue;
        }

        // Inside a require block: `<path> <version>` or `<path> <version> // indirect`.
        if in_require_block {
            // Skip replace/exclude directives inside blocks.
            let path = trimmed.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() && !path.starts_with("//") {
                require_paths.push(path.to_string());
            }
        }
    }

    GoModData { module_path, require_paths }
}

// ---------------------------------------------------------------------------
// GlobalUsings.cs parsing
// ---------------------------------------------------------------------------

/// Find GlobalUsings.cs (and similar) files under the project root.
fn find_global_using_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    collect_global_usings(root, &mut result, 0);
    result
}

fn collect_global_usings(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "bin" | "obj" | "node_modules" | ".git" | "target"
                    | "packages" | ".vs" | "TestResults" | "artifacts"
            ) {
                continue;
            }
            collect_global_usings(&path, out, depth + 1);
        } else {
            let name = entry.file_name();
            let name_lower = name.to_string_lossy().to_lowercase();
            if name_lower.contains("globalusing") || name_lower == "usings.cs" {
                out.push(path);
            }
        }
    }
}

/// Parse `global using ...;` statements from a .cs file.
/// Returns namespace strings like "System.Linq", "Microsoft.AspNetCore.Mvc".
fn parse_global_usings(content: &str) -> Vec<String> {
    let mut usings = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("global using") {
            // Skip `global using static ...` for now.
            let rest = rest.trim();
            if rest.starts_with("static ") {
                continue;
            }
            // Strip trailing `;` and whitespace.
            let ns = rest.trim_end_matches(';').trim();
            if !ns.is_empty() {
                usings.push(ns.to_string());
            }
        }
    }
    usings
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
        // Walk up the path segments looking for a known package.
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
            // Internal: exactly matches or is a sub-package of the module.
            if import_path == module_path {
                return false;
            }
            if import_path.starts_with(module_path.as_str())
                && import_path.len() > module_path.len()
                && import_path.as_bytes()[module_path.len()] == b'/'
            {
                return false;
            }
            // Everything else is external.
            return true;
        }

        // No go.mod found — use heuristic: paths with a dot in the first segment
        // look like "github.com/...", "golang.org/..." etc. and are external.
        // Standard library paths have no dot (e.g., "fmt", "net/http").
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
}

// ---------------------------------------------------------------------------
// Cargo.toml scanning (Rust projects)
// ---------------------------------------------------------------------------

/// Scan the project root for Cargo.toml files and populate `ctx.rust_crates`.
///
/// Handles both workspace roots (with `[workspace]`) and individual crate
/// Cargo.toml files. We parse `[dependencies]` and `[dev-dependencies]` from
/// every Cargo.toml we find.
fn scan_cargo_toml(root: &Path, ctx: &mut ProjectContext) {
    let mut paths = Vec::new();
    collect_cargo_tomls(root, &mut paths, 0);

    for path in &paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for name in parse_cargo_dependencies(&content) {
            ctx.rust_crates.insert(name);
        }
    }
}

fn collect_cargo_tomls(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 8 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "target" | ".git" | "node_modules" | "bin" | "obj" | ".cargo"
            ) {
                continue;
            }
            collect_cargo_tomls(&path, out, depth + 1);
        } else if entry.file_name() == "Cargo.toml" {
            out.push(path);
        }
    }
}

/// Parse crate names from `[dependencies]` and `[dev-dependencies]` sections.
///
/// TOML parsing is done line-by-line to avoid pulling in a full TOML crate.
/// We only need crate names (keys), not version strings.
///
/// Handles:
///   `serde = "1"`
///   `tokio = { version = "1", features = ["full"] }`
///   `my-crate.workspace = true`
pub fn parse_cargo_dependencies(content: &str) -> Vec<String> {
    let mut crates = Vec::new();
    let mut in_dep_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect section headers.
        if trimmed.starts_with('[') {
            // We care about [dependencies], [dev-dependencies],
            // [build-dependencies], and their workspace variants.
            in_dep_section = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            continue;
        }

        if !in_dep_section {
            continue;
        }

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Extract the crate name (the key before `=`).
        // Keys may contain hyphens and underscores but not spaces.
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos]
                .trim()
                // Strip dotted suffixes like `tokio.workspace`
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                crates.push(key.to_string());
            }
        }
    }

    crates
}

// ---------------------------------------------------------------------------
// Python manifest scanning
// ---------------------------------------------------------------------------

/// Scan the project root for Python manifest files and populate `ctx.python_packages`.
fn scan_python_manifests(root: &Path, ctx: &mut ProjectContext) {
    let mut paths = Vec::new();
    collect_python_manifests(root, &mut paths, 0);

    for (path, kind) in &paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let names = match kind.as_str() {
            "pyproject" => parse_pyproject_deps(&content),
            "requirements" => parse_requirements_txt(&content),
            "pipfile" => parse_pipfile_deps(&content),
            _ => Vec::new(),
        };
        for name in names {
            ctx.python_packages.insert(name);
        }
    }
}

fn collect_python_manifests(
    dir: &Path,
    out: &mut Vec<(std::path::PathBuf, String)>,
    depth: usize,
) {
    if depth > 6 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git"
                    | "node_modules"
                    | "target"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
                    | ".tox"
                    | "dist"
                    | "build"
                    | ".eggs"
            ) {
                continue;
            }
            collect_python_manifests(&path, out, depth + 1);
        } else {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            let kind = if name == "pyproject.toml" {
                "pyproject"
            } else if name == "requirements.txt"
                || (name.starts_with("requirements") && name.ends_with(".txt"))
            {
                "requirements"
            } else if name == "Pipfile" {
                "pipfile"
            } else {
                continue;
            };
            out.push((path, kind.to_string()));
        }
    }
}

/// Parse package names from `pyproject.toml`.
///
/// Handles both PEP 621 (`[project] dependencies`) and Poetry
/// (`[tool.poetry.dependencies]`) formats. Line-based; no full TOML parser.
pub fn parse_pyproject_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps = false;
    let mut in_array = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Section detection.
        if trimmed.starts_with('[') {
            in_deps = matches!(
                trimmed,
                "[project.dependencies]"
                    | "[tool.poetry.dependencies]"
                    | "[tool.poetry.dev-dependencies]"
                    | "[tool.poetry.group.dev.dependencies]"
            )
            // Also detect inline array under [project]: `dependencies = [...]`
            // — handled below without section flag.
            || trimmed == "[project]";

            // Reset inline array state on any section boundary.
            in_array = false;
            continue;
        }

        // PEP 621: `dependencies = ["django>=4.0", "pydantic"]` (may span lines)
        if trimmed.starts_with("dependencies") && trimmed.contains('=') {
            // Single-line or start of multi-line array.
            let rest = trimmed.splitn(2, '=').nth(1).unwrap_or("").trim();
            in_array = rest.starts_with('[') && !rest.contains(']');

            // Extract names from the current line.
            let data = if rest.starts_with('[') {
                let inner = rest.trim_start_matches('[');
                let inner = inner.trim_end_matches(']');
                inner
            } else {
                rest
            };
            for name in extract_pep508_names(data) {
                packages.push(name);
            }
            if rest.contains(']') {
                in_array = false;
            }
            continue;
        }

        if in_array {
            // Only end the array on a standalone `]` — not on a dependency line
            // that contains extras like `"celery[redis]~=5.6.2"`.
            if trimmed.starts_with(']') {
                in_array = false;
            }
            for name in extract_pep508_names(trimmed) {
                packages.push(name);
            }
            continue;
        }

        // Poetry format: `django = "^4.0"` inside a deps section.
        if in_deps && !trimmed.starts_with('[') && trimmed.contains('=') {
            let key = trimmed.split('=').next().unwrap_or("").trim();
            // python is a special Poetry key, not a package.
            if !key.is_empty()
                && key != "python"
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                packages.push(key.to_string());
            }
        }
    }

    packages
}

/// Parse a PEP 508 dependency specifier list and extract package names.
/// `"django>=4.0"`, `"pydantic[email]"`, `'requests ; python_version>="3"'`
fn extract_pep508_names(s: &str) -> Vec<String> {
    let mut names = Vec::new();
    for part in s.split(',') {
        // Strip quotes, brackets, extras, version specs.
        let part = part.trim().trim_matches(|c| c == '"' || c == '\'' || c == ']');
        // Name ends at the first of: `[`, `>`, `<`, `=`, `~`, `!`, `;`, `@`, ` `.
        // `~` handles the PEP 440 compatible-release specifier (`~=`).
        let end = part
            .find(|c: char| matches!(c, '[' | '>' | '<' | '=' | '~' | '!' | ';' | '@' | ' '))
            .unwrap_or(part.len());
        let name = part[..end].trim();
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            names.push(name.to_string());
        }
    }
    names
}

/// Parse package names from a `requirements.txt` file.
///
/// Handles:
///   - Plain package names: `django`
///   - Versioned: `django>=4.0,<5.0`
///   - Extras: `pydantic[email]`
///   - Comments (`#`) and blank lines
///   - `-r other.txt` (skipped — we don't recurse)
///   - VCS/URL installs: `git+https://...` (skipped)
pub fn parse_requirements_txt(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with('-')
            || trimmed.starts_with("git+")
            || trimmed.starts_with("http")
        {
            continue;
        }
        // Strip inline comment.
        let without_comment = trimmed.split('#').next().unwrap_or(trimmed).trim();
        // Extract the name portion (up to version/extras/env markers).
        let end = without_comment
            .find(|c: char| matches!(c, '[' | '>' | '<' | '=' | '!' | ';' | '@' | ' '))
            .unwrap_or(without_comment.len());
        let name = without_comment[..end].trim();
        if !name.is_empty() {
            packages.push(name.to_string());
        }
    }
    packages
}

/// Parse package names from a `Pipfile`.
///
/// Format:
/// ```toml
/// [packages]
/// django = "*"
/// requests = ">=2.28"
///
/// [dev-packages]
/// pytest = "*"
/// ```
pub fn parse_pipfile_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            in_section = matches!(trimmed, "[packages]" | "[dev-packages]");
            continue;
        }

        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                packages.push(key.to_string());
            }
        }
    }

    packages
}

// ---------------------------------------------------------------------------
// Gemfile parsing (Ruby projects)
// ---------------------------------------------------------------------------

/// Scan the project root for a Gemfile and populate `ctx.ruby_gems`.
fn scan_gemfile(root: &Path, ctx: &mut ProjectContext) {
    let gemfile_path = root.join("Gemfile");
    if !gemfile_path.is_file() {
        return;
    }
    let content = match std::fs::read_to_string(&gemfile_path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read Gemfile at {}: {e}", gemfile_path.display());
            return;
        }
    };
    for name in parse_gemfile_gems(&content) {
        ctx.ruby_gems.insert(name);
    }
}

/// Parse gem names from a Gemfile.
///
/// Handles:
///   `gem 'rails', '~> 7.0'`
///   `gem "devise"`
///   `gem 'sidekiq', require: false`
///
/// Returns the gem name only (first argument, without quotes or version).
pub fn parse_gemfile_gems(content: &str) -> Vec<String> {
    let mut gems = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and blank lines.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Match lines starting with `gem ` (possibly indented for group blocks).
        let rest = if let Some(r) = trimmed.strip_prefix("gem ") {
            r.trim()
        } else {
            continue;
        };

        // Extract the gem name — the first quoted string argument.
        // Handles both single and double quotes.
        let name = if let Some(r) = rest.strip_prefix('\'') {
            r.split('\'').next().unwrap_or("").trim()
        } else if let Some(r) = rest.strip_prefix('"') {
            r.split('"').next().unwrap_or("").trim()
        } else {
            // Unquoted gem name (rare but valid in some DSL forms).
            rest.split(|c: char| c == ',' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim()
        };

        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            gems.push(name.to_string());
        }
    }
    gems
}

// ---------------------------------------------------------------------------
// composer.json parsing (PHP projects)
// ---------------------------------------------------------------------------

/// Scan the project root for a composer.json and populate `ctx.php_packages`.
fn scan_composer_json(root: &Path, ctx: &mut ProjectContext) {
    let composer_path = root.join("composer.json");
    if !composer_path.is_file() {
        return;
    }
    let content = match std::fs::read_to_string(&composer_path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read composer.json at {}: {e}", composer_path.display());
            return;
        }
    };
    for name in parse_composer_json_deps(&content) {
        ctx.php_packages.insert(name);
    }
}

/// Extract package names from a composer.json file's `require` and `require-dev` objects.
///
/// Uses `serde_json` for parsing since it's already a workspace dependency.
/// Skips the `php` and `ext-*` entries (platform requirements, not packages).
pub fn parse_composer_json_deps(content: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };

    let mut packages = Vec::new();
    for key in &["require", "require-dev"] {
        if let Some(serde_json::Value::Object(deps)) = obj.get(*key) {
            for pkg_name in deps.keys() {
                // Skip platform requirements.
                if pkg_name == "php"
                    || pkg_name.starts_with("ext-")
                    || pkg_name.starts_with("lib-")
                {
                    continue;
                }
                if !pkg_name.is_empty() {
                    packages.push(pkg_name.clone());
                }
            }
        }
    }
    packages
}

// ---------------------------------------------------------------------------
// pom.xml / build.gradle parsing (Java projects)
// ---------------------------------------------------------------------------

/// Scan the project root for pom.xml and build.gradle files, populating
/// `ctx.external_prefixes` with Java dependency groupId prefixes.
fn scan_java_manifests(root: &Path, ctx: &mut ProjectContext) {
    // Always-external Java roots — present in every Java project.
    for prefix in &["java", "javax", "jakarta", "sun", "com.sun", "org.junit"] {
        ctx.external_prefixes.insert(prefix.to_string());
    }

    // Scan for pom.xml files.
    let mut pom_paths = Vec::new();
    collect_java_manifests(root, &mut pom_paths, 0);

    for (path, kind) in &pom_paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let group_ids = match kind.as_str() {
            "pom" => parse_pom_xml_dependencies(&content),
            "gradle" => parse_gradle_dependencies(&content),
            _ => Vec::new(),
        };
        for group_id in group_ids {
            // Store the full groupId and the root prefix.
            ctx.external_prefixes.insert(group_id.clone());
            if let Some(root_prefix) = group_id.split('.').next() {
                ctx.external_prefixes.insert(root_prefix.to_string());
            }
        }
    }
}

fn collect_java_manifests(
    dir: &Path,
    out: &mut Vec<(std::path::PathBuf, String)>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "target" | "build" | "node_modules" | ".gradle" | "bin" | "obj"
            ) {
                continue;
            }
            collect_java_manifests(&path, out, depth + 1);
        } else {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name == "pom.xml" {
                out.push((path, "pom".to_string()));
            } else if name == "build.gradle" || name == "build.gradle.kts" {
                out.push((path, "gradle".to_string()));
            }
        }
    }
}

/// Parse `<dependency><groupId>...</groupId><artifactId>...</artifactId>` from pom.xml.
///
/// Returns a list of groupId strings (e.g., "org.springframework", "com.google.guava").
/// Lightweight line-based parsing — no XML library needed.
pub fn parse_pom_xml_dependencies(content: &str) -> Vec<String> {
    let mut group_ids = Vec::new();
    let mut in_dependency = false;
    let mut current_group_id: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("<dependency>") {
            in_dependency = true;
            current_group_id = None;
            continue;
        }
        if trimmed.contains("</dependency>") {
            if let Some(gid) = current_group_id.take() {
                group_ids.push(gid);
            }
            in_dependency = false;
            continue;
        }

        if !in_dependency {
            continue;
        }

        // Extract <groupId>...</groupId>
        if let Some(value) = extract_xml_text(trimmed, "groupId") {
            current_group_id = Some(value);
        }
    }

    group_ids
}

/// Extract the text content of a simple XML element on a single line.
/// e.g., `<groupId>org.springframework</groupId>` → `Some("org.springframework")`
fn extract_xml_text(line: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = line.find(&open)?;
    let after_open = &line[start + open.len()..];
    let end = after_open.find(&close)?;
    let value = after_open[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parse dependency declarations from build.gradle / build.gradle.kts.
///
/// Handles the common forms:
///   `implementation 'group:artifact:version'`
///   `implementation("group:artifact:version")`
///   `testImplementation 'group:artifact:version'`
///   `api 'group:artifact:version'`
///
/// Returns a list of groupId strings extracted from the coordinates.
pub fn parse_gradle_dependencies(content: &str) -> Vec<String> {
    let mut group_ids = Vec::new();

    // Keywords that introduce a dependency coordinate string.
    let dependency_keywords = [
        "implementation",
        "testImplementation",
        "api",
        "compileOnly",
        "runtimeOnly",
        "testCompileOnly",
        "annotationProcessor",
        "kapt",
    ];

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }

        let mut found_keyword = false;
        let mut rest = trimmed;
        for kw in &dependency_keywords {
            if let Some(r) = trimmed.strip_prefix(kw) {
                let r = r.trim();
                // Must be followed by a space, '(', or quote.
                if r.is_empty() || r.starts_with(' ') || r.starts_with('(') || r.starts_with('"') || r.starts_with('\'') {
                    rest = r.trim_start_matches(['(', ' ']);
                    found_keyword = true;
                    break;
                }
            }
        }
        if !found_keyword {
            continue;
        }

        // Extract the coordinate string from quotes.
        let coord = if let Some(r) = rest.strip_prefix('\'') {
            r.split('\'').next().unwrap_or("").trim()
        } else if let Some(r) = rest.strip_prefix('"') {
            r.split('"').next().unwrap_or("").trim()
        } else {
            continue;
        };

        // Maven coordinate: "group:artifact:version"
        // Extract the groupId (first segment before ':').
        if let Some(group_id) = coord.split(':').next() {
            let group_id = group_id.trim();
            if !group_id.is_empty()
                && group_id.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
            {
                group_ids.push(group_id.to_string());
            }
        }
    }

    group_ids
}

// ---------------------------------------------------------------------------
// ProjectContext helpers for Java/Ruby/PHP resolvers
// ---------------------------------------------------------------------------

impl ProjectContext {
    /// Check whether a Ruby require path refers to an external gem.
    pub fn is_external_ruby_gem(&self, require_path: &str) -> bool {
        let gem_root = require_path.split('/').next().unwrap_or(require_path);
        self.ruby_gems.contains(gem_root) || self.ruby_gems.contains(require_path)
    }

    /// Check whether a PHP composer package name is external.
    ///
    /// Handles both vendor/package form and bare package names.
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
