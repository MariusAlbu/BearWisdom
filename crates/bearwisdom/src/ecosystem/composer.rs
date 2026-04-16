// =============================================================================
// ecosystem/composer.rs — Composer ecosystem (PHP)
//
// Phase 2 + 3: consolidates `indexer/externals/php.rs` +
// `indexer/manifest/composer.rs`. Packages live at `vendor/<vendor>/<name>/`.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::indexer::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::indexer::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("composer");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["php"];
const LEGACY_ECOSYSTEM_TAG: &str = "php";

pub struct ComposerEcosystem;

impl Ecosystem for ComposerEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("php"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_php_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_php_root(dep)
    }
}

impl ExternalSourceLocator for ComposerEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_php_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_php_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ComposerEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ComposerEcosystem)).clone()
}

// ===========================================================================
// Manifest reader
// ===========================================================================

pub struct ComposerManifest;

impl ManifestReader for ComposerManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Composer }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() { return None }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_composer_files(project_root, &mut paths, 0);
        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };
            let mut data = ManifestData::default();
            let (name, deps) = parse_composer_json(&content);
            for pkg in deps { data.dependencies.insert(pkg); }
            let package_dir = manifest_path
                .parent().map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());
            out.push(ReaderEntry { package_dir, manifest_path, data, name });
        }
        out
    }
}

fn collect_composer_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "vendor" | "node_modules" | "target" | "bin" | "obj"
            ) { continue }
            collect_composer_files(&path, out, depth + 1);
        } else if entry.file_name() == "composer.json" {
            out.push(path);
        }
    }
}

fn parse_composer_json(content: &str) -> (Option<String>, Vec<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return (None, Vec::new());
    };
    let Some(obj) = value.as_object() else { return (None, Vec::new()) };
    let name = obj.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
    let mut packages = Vec::new();
    for key in &["require", "require-dev"] {
        if let Some(serde_json::Value::Object(deps)) = obj.get(*key) {
            for pkg_name in deps.keys() {
                if pkg_name == "php"
                    || pkg_name.starts_with("ext-")
                    || pkg_name.starts_with("lib-")
                { continue }
                if !pkg_name.is_empty() { packages.push(pkg_name.clone()) }
            }
        }
    }
    (name, packages)
}

pub fn parse_composer_json_deps(content: &str) -> Vec<String> {
    parse_composer_json(content).1
}

// ===========================================================================
// Discovery + walk
// ===========================================================================

pub fn discover_php_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let composer_path = project_root.join("composer.json");
    if !composer_path.is_file() { return Vec::new() }
    let Ok(content) = std::fs::read_to_string(&composer_path) else { return Vec::new() };
    let declared = parse_composer_json_deps(&content);
    if declared.is_empty() { return Vec::new() }

    let vendor = project_root.join("vendor");
    if !vendor.is_dir() { return Vec::new() }

    let mut roots = Vec::new();
    for dep in &declared {
        let pkg_dir = vendor.join(dep.replace('/', std::path::MAIN_SEPARATOR_STR));
        if pkg_dir.is_dir() {
            let version = read_composer_version(&pkg_dir);
            roots.push(ExternalDepRoot {
                module_path: dep.clone(),
                version,
                root: pkg_dir,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
            });
        }
    }
    debug!("PHP: {} external package roots", roots.len());
    roots
}

fn read_composer_version(pkg_dir: &Path) -> String {
    let installed = pkg_dir.join("composer.json");
    if let Ok(content) = std::fs::read_to_string(&installed) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(v) = val.get("version").and_then(|v| v.as_str()) {
                return v.to_string();
            }
        }
    }
    String::new()
}

fn walk_php_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let walk_root = if dep.root.join("src").is_dir() {
        dep.root.join("src")
    } else {
        dep.root.clone()
    };
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
                if matches!(name, "tests" | "test" | "Tests" | "Test" | "vendor" | "docs" | "examples")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".php") { continue }
            if name.ends_with("Test.php") || name.ends_with("Tests.php") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:php:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "php",
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

    #[test]
    fn ecosystem_identity() {
        let c = ComposerEcosystem;
        assert_eq!(c.id(), ID);
        assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&c), &["php"]);
    }

    #[test]
    fn legacy_locator_tag_is_php() {
        assert_eq!(ExternalSourceLocator::ecosystem(&ComposerEcosystem), "php");
    }

    #[test]
    fn composer_json_parser_skips_platform_requirements() {
        let content = r#"{"require":{"php":">=8.0","ext-json":"*","laravel/framework":"^11.0"}}"#;
        let deps = parse_composer_json_deps(content);
        assert_eq!(deps, vec!["laravel/framework"]);
    }

    #[test]
    fn php_discovers_composer_deps() {
        let tmp = std::env::temp_dir().join("bw-test-composer-discover");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("composer.json"), r#"{"require":{"laravel/framework":"^11.0"}}"#).unwrap();
        let vendor = tmp.join("vendor").join("laravel").join("framework").join("src");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(vendor.join("Application.php"), "<?php class Application {}\n").unwrap();

        let roots = discover_php_externals(&tmp);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].module_path, "laravel/framework");
        let files = walk_php_root(&roots[0]);
        assert_eq!(files.len(), 1);
        assert!(files[0].relative_path.contains("Application.php"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
