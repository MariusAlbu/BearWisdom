// =============================================================================
// ecosystem/spm.rs — Swift Package Manager ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/swift.rs` +
// `indexer/manifest/swift_pm.rs`. Prefers `Package.resolved` JSON pins
// (v2/v3 format) over line-parsed `Package.swift`; probes multiple
// checkout cache locations (SPM .build/, Xcode DerivedData, Windows
// LOCALAPPDATA).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("spm");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["swift"];
const LEGACY_ECOSYSTEM_TAG: &str = "swift";

pub struct SpmEcosystem;

impl Ecosystem for SpmEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("swift"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_swift_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_swift_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        walk_swift_narrowed(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        walk_swift_narrowed(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_swift_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for SpmEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_swift_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_swift_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<SpmEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(SpmEcosystem)).clone()
}

// ===========================================================================
// Manifest reader
// ===========================================================================

pub struct SwiftPMManifest;

impl ManifestReader for SwiftPMManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::SwiftPM }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let package_swift = project_root.join("Package.swift");
        if !package_swift.is_file() { return None }
        let content = std::fs::read_to_string(&package_swift).ok()?;
        let mut data = ManifestData::default();
        for name in parse_swift_package_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_swift_package_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.contains(".package(") { continue }
        if let Some(name) = extract_swift_string_arg(trimmed, "name:") {
            if is_valid_package_name(&name) { packages.push(name); continue }
        }
        if let Some(url) = extract_swift_string_arg(trimmed, "url:") {
            if let Some(name) = name_from_url(&url) {
                if is_valid_package_name(&name) { packages.push(name); }
            }
        }
    }
    packages
}

fn extract_swift_string_arg(line: &str, arg_name: &str) -> Option<String> {
    let start = line.find(arg_name)?;
    let after_key = &line[start + arg_name.len()..];
    let after_ws = after_key.trim_start();
    let after_quote = after_ws.strip_prefix('"')?;
    let end = after_quote.find('"')?;
    Some(after_quote[..end].to_string())
}

fn name_from_url(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit('/').next()?;
    let name = last.trim_end_matches(".git");
    if name.is_empty() { None } else { Some(name.to_string()) }
}

fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}

// ===========================================================================
// Package.resolved (v2/v3) pin parser
// ===========================================================================

#[derive(serde::Deserialize)]
struct PackageResolved {
    pins: Vec<Pin>,
}

#[derive(serde::Deserialize)]
struct Pin {
    identity: String,
    state: PinState,
}

#[derive(serde::Deserialize)]
struct PinState {
    version: Option<String>,
}

fn parse_package_resolved(path: &Path) -> Option<Vec<(String, String)>> {
    let content = std::fs::read_to_string(path).ok()?;
    let resolved: PackageResolved = serde_json::from_str(&content).ok()?;
    Some(
        resolved
            .pins
            .into_iter()
            .map(|p| (p.identity, p.state.version.unwrap_or_default()))
            .collect(),
    )
}

// ===========================================================================
// Discovery
// ===========================================================================

pub fn discover_swift_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let pins = find_and_parse_package_resolved(project_root).or_else(|| {
        let package_swift = project_root.join("Package.swift");
        let content = std::fs::read_to_string(&package_swift).ok()?;
        let deps = parse_swift_package_deps(&content);
        if deps.is_empty() { return None }
        debug!("Swift: using {} deps from Package.swift (no Package.resolved)", deps.len());
        Some(deps.into_iter().map(|name| (name, String::new())).collect())
    });

    let Some(pins) = pins else {
        debug!("Swift: no Package.resolved or Package.swift at {}", project_root.display());
        return Vec::new();
    };
    if pins.is_empty() { return Vec::new() }

    let checkout_roots = find_checkout_roots(project_root);
    if checkout_roots.is_empty() {
        debug!("Swift: no SPM checkout cache at {}", project_root.display());
        return Vec::new();
    }

    // R3: collect Swift `import Foo` statements once. Each dep root carries
    // the set; walk_swift_narrowed walks only Sources/<TargetName>/ dirs
    // whose name matches an imported module.
    let user_imports: Vec<String> = collect_swift_user_imports(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    for (identity, version) in &pins {
        for checkout_root in &checkout_roots {
            let dep_dir = checkout_root.join(identity);
            if dep_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: identity.clone(),
                    version: version.clone(),
                    root: dep_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: user_imports.clone(),
                });
                break;
            }
        }
    }
    debug!("Swift: discovered {} external package roots", roots.len());
    roots
}

fn find_and_parse_package_resolved(project_root: &Path) -> Option<Vec<(String, String)>> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(project_root.join("Package.resolved"));
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".xcodeproj") {
                candidates.push(
                    path.join("project.xcworkspace").join("xcshareddata")
                        .join("swiftpm").join("Package.resolved"),
                );
            } else if name.ends_with(".xcworkspace") {
                candidates.push(
                    path.join("xcshareddata").join("swiftpm").join("Package.resolved"),
                );
            }
        }
    }
    for path in &candidates {
        if !path.is_file() { continue }
        if let Some(pins) = parse_package_resolved(path) {
            debug!("Swift: parsed {} pins from {}", pins.len(), path.display());
            return Some(pins);
        }
    }
    None
}

fn find_checkout_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let build_checkouts = project_root.join(".build").join("checkouts");
    if build_checkouts.is_dir() { roots.push(build_checkouts) }
    let local_sp = project_root.join("SourcePackages").join("checkouts");
    if local_sp.is_dir() { roots.push(local_sp) }
    if let Some(home) = dirs::home_dir() {
        let derived_data = home.join("Library").join("Developer").join("Xcode").join("DerivedData");
        if derived_data.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&derived_data) {
                for entry in entries.flatten() {
                    let sp = entry.path().join("SourcePackages").join("checkouts");
                    if sp.is_dir() { roots.push(sp) }
                }
            }
        }
    }
    if let Some(local_app) = std::env::var_os("LOCALAPPDATA") {
        let win_sp = PathBuf::from(local_app).join("swift").join("SourcePackages").join("checkouts");
        if win_sp.is_dir() { roots.push(win_sp) }
    }
    roots
}

// ---------------------------------------------------------------------------
// R3 reachability — module-level narrowing
// ---------------------------------------------------------------------------
//
// Swift imports are module-granular (`import Foundation` brings in an entire
// SPM target). We scan project .swift files for `import X`/`@_exported import X`
// statements, collect the module set, and at walk time keep only files
// under Sources/<TargetName>/ whose target name matches an imported module.
// Sub-target file scoping doesn't apply — within a target every type is
// visible without explicit qualification — so the granularity of "either
// walk this whole target or none of it" matches Swift semantics.

fn collect_swift_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_swift_imports_recursive(project_root, &mut out, 0);
    out
}

fn scan_swift_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    ".git" | ".build" | "DerivedData" | "Carthage" | "Pods"
                        | "build" | "node_modules"
                ) || name.starts_with('.') { continue }
            }
            scan_swift_imports_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".swift") { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_swift_imports_from_source(&content, out);
        }
    }
}

/// Parse `import Foo`, `import struct Foo.Bar`, `@_exported import Foo`,
/// `@testable import Foo`. Stores just the top-level module name.
fn extract_swift_imports_from_source(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for raw in content.lines() {
        let mut line = raw.trim();
        // Drop attribute prefixes: `@_exported`, `@testable`.
        while let Some(attr_end) = line.strip_prefix('@') {
            let after = attr_end.split_whitespace().next().unwrap_or("");
            line = line.strip_prefix(&format!("@{after}")).unwrap_or(line).trim();
        }
        let Some(rest) = line.strip_prefix("import ") else { continue };
        let rest = rest
            .trim_start_matches("struct ")
            .trim_start_matches("class ")
            .trim_start_matches("enum ")
            .trim_start_matches("protocol ")
            .trim_start_matches("typealias ")
            .trim_start_matches("func ")
            .trim_start_matches("var ")
            .trim_start_matches("let ")
            .trim();
        let module = rest.split('.').next().unwrap_or("").trim();
        if module.is_empty() { continue }
        if !module.chars().next().map_or(false, |c| c.is_alphabetic()) { continue }
        out.insert(module.to_string());
    }
}

fn walk_swift_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        return walk_swift_root(dep);
    }
    let modules: std::collections::HashSet<&String> = dep.requested_imports.iter().collect();

    let sources = dep.root.join("Sources");
    if !sources.is_dir() {
        // Flat-layout package: SPM permits omitting Sources/ for single-target
        // packages; in that case the whole package = one target and the user
        // either imported it (walk it) or didn't (skip).
        let pkg_name_match = modules.iter().any(|m| {
            m.eq_ignore_ascii_case(&dep.module_path)
                || dep.module_path
                    .trim_end_matches(".git")
                    .ends_with(m.as_str())
        });
        if !pkg_name_match { return Vec::new() }
        return walk_swift_root(dep);
    }

    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&sources) else { return walk_swift_root(dep) };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(target_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !path.is_dir() { continue }
        // Match by exact name OR case-insensitive (a few packages camelCase
        // their target dir while user imports the module by exact name).
        if !modules.contains(&target_name.to_string())
            && !modules.iter().any(|m| m.eq_ignore_ascii_case(target_name))
        { continue }
        walk_dir_bounded(&path, &dep.root, dep, &mut out, 0);
    }
    out
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_swift_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let sources = dep.root.join("Sources");
    let walk_root = if sources.is_dir() { sources } else { dep.root.clone() };
    walk_dir_bounded(&walk_root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "Tests" | "tests" | "Examples" | "Benchmarks")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".swift") { continue }
            if name.ends_with("Tests.swift") || name.ends_with("Test.swift") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:swift:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "swift",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

pub(crate) fn build_swift_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_swift_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_swift_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();
    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Header-only tree-sitter scan of a Swift source file. Records top-level
/// class / struct / enum / protocol / extension / function / typealias names.
/// Bodies are never walked.
fn scan_swift_header(source: &str) -> Vec<String> {
    let language = tree_sitter_swift::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_swift_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_swift_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_declaration"
        | "struct_declaration"
        | "enum_declaration"
        | "protocol_declaration"
        | "function_declaration"
        | "typealias_declaration"
        | "extension_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(t) = name_node.utf8_text(bytes) {
                    out.push(t.to_string());
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE_RESOLVED_V3: &str = r#"{
  "originHash" : "abc",
  "pins" : [
    {
      "identity" : "bodega",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/mergesort/Bodega",
      "state" : { "revision" : "abc", "version" : "2.1.3" }
    },
    {
      "identity" : "emojitext",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/Dimillian/EmojiText",
      "state" : { "branch" : "fix", "revision" : "def" }
    }
  ],
  "version" : 3
}"#;

    #[test]
    fn ecosystem_identity() {
        let s = SpmEcosystem;
        assert_eq!(s.id(), ID);
        assert_eq!(Ecosystem::kind(&s), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&s), &["swift"]);
    }

    #[test]
    fn legacy_locator_tag_is_swift() {
        assert_eq!(ExternalSourceLocator::ecosystem(&SpmEcosystem), "swift");
    }

    #[test]
    fn parse_package_resolved_v3_with_version() {
        let tmp = std::env::temp_dir().join("bw-test-spm-resolved");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let resolved_path = tmp.join("Package.resolved");
        std::fs::write(&resolved_path, PACKAGE_RESOLVED_V3).unwrap();
        let pins = parse_package_resolved(&resolved_path).unwrap();
        assert_eq!(pins.len(), 2);
        let bodega = pins.iter().find(|(id, _)| id == "bodega").unwrap();
        assert_eq!(bodega.1, "2.1.3");
        let emoji = pins.iter().find(|(id, _)| id == "emojitext").unwrap();
        assert_eq!(emoji.1, "");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_swift_package_deps_from_url() {
        let content = r#"
            dependencies: [
                .package(url: "https://github.com/vapor/vapor.git", from: "4.0.0"),
                .package(name: "Argon2Kit", url: "https://github.com/x/y", branch: "main"),
            ]
        "#;
        let deps = parse_swift_package_deps(content);
        assert!(deps.contains(&"vapor".to_string()));
        assert!(deps.contains(&"Argon2Kit".to_string()));
    }

    #[test]
    fn discover_returns_empty_when_no_cache() {
        let tmp = std::env::temp_dir().join("bw-test-spm-no-cache");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Package.resolved"), PACKAGE_RESOLVED_V3).unwrap();
        let roots = discover_swift_externals(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    // -----------------------------------------------------------------
    // R3 — module narrowing
    // -----------------------------------------------------------------

    #[test]
    fn swift_import_extracts_module() {
        let mut out = std::collections::HashSet::new();
        extract_swift_imports_from_source(
            "import Foundation\n@_exported import Combine\n@testable import MyModule\nimport struct OtherModule.Thing\n",
            &mut out,
        );
        assert!(out.contains("Foundation"));
        assert!(out.contains("Combine"));
        assert!(out.contains("MyModule"));
        assert!(out.contains("OtherModule"));
    }

    #[test]
    fn swift_narrowed_walk_keeps_only_imported_targets() {
        let tmp = std::env::temp_dir().join("bw-test-spm-r3-narrow");
        let _ = std::fs::remove_dir_all(&tmp);
        let dep_root = tmp.join("vapor");
        let sources = dep_root.join("Sources");
        std::fs::create_dir_all(sources.join("Vapor")).unwrap();
        std::fs::create_dir_all(sources.join("RoutingKit")).unwrap();
        std::fs::create_dir_all(sources.join("Internal")).unwrap();
        std::fs::write(sources.join("Vapor/Application.swift"), "class Application {}\n").unwrap();
        std::fs::write(sources.join("RoutingKit/Router.swift"), "class Router {}\n").unwrap();
        std::fs::write(sources.join("Internal/Hidden.swift"), "class Hidden {}\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "vapor".to_string(),
            version: "4.0".to_string(),
            root: dep_root.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: vec!["Vapor".to_string()],
        };
        let files = walk_swift_narrowed(&dep);
        let paths: std::collections::HashSet<_> =
            files.iter().map(|f| f.absolute_path.clone()).collect();
        assert!(paths.contains(&sources.join("Vapor/Application.swift")));
        assert!(!paths.contains(&sources.join("RoutingKit/Router.swift")));
        assert!(!paths.contains(&sources.join("Internal/Hidden.swift")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn swift_narrowed_walk_falls_back_when_no_imports() {
        let tmp = std::env::temp_dir().join("bw-test-spm-r3-fallback");
        let _ = std::fs::remove_dir_all(&tmp);
        let dep_root = tmp.join("foo");
        let sources = dep_root.join("Sources");
        std::fs::create_dir_all(sources.join("Foo")).unwrap();
        std::fs::write(sources.join("Foo/A.swift"), "class A {}\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "foo".to_string(),
            version: "1.0".to_string(),
            root: dep_root.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = walk_swift_narrowed(&dep);
        assert_eq!(files.len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
