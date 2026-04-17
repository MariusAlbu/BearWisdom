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

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
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
}
