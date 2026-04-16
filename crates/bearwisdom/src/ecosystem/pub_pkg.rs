// =============================================================================
// ecosystem/pub_pkg.rs — Dart Pub ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/dart.rs` +
// `indexer/manifest/pubspec.rs`. Two resolution strategies:
//   1. `.dart_tool/package_config.json` (Dart 2.5+) — exact paths.
//   2. `pubspec.lock` + pub cache walk (`~/.pub-cache/hosted/pub.dev/`).
//
// Module named `pub_pkg` because `pub` is a Rust keyword.
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

pub const ID: EcosystemId = EcosystemId::new("pub");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["dart"];
const LEGACY_ECOSYSTEM_TAG: &str = "dart";

pub struct PubEcosystem;

impl Ecosystem for PubEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("dart"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_dart_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_root(dep)
    }
}

impl ExternalSourceLocator for PubEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dart_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PubEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PubEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (migrated from indexer/manifest/pubspec.rs)
// ===========================================================================

pub struct PubspecManifest;

impl ManifestReader for PubspecManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Pubspec }

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
        collect_pubspec_files(project_root, &mut paths, 0);
        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };
            let mut data = ManifestData::default();
            for name in parse_pubspec_deps(&content) {
                data.dependencies.insert(name);
            }
            let name = parse_pubspec_name(&content);
            let package_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());
            out.push(ReaderEntry { package_dir, manifest_path, data, name });
        }
        out
    }
}

fn collect_pubspec_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | ".dart_tool" | "build" | "node_modules" | "target" | "bin" | "obj"
            ) { continue }
            collect_pubspec_files(&path, out, depth + 1);
        } else if entry.file_name() == "pubspec.yaml" {
            out.push(path);
        }
    }
}

fn parse_pubspec_name(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("name:") && !line.starts_with(' ') && !line.starts_with('\t') {
            let rest = trimmed["name:".len()..].trim();
            if !rest.is_empty() { return Some(rest.to_string()) }
        }
    }
    None
}

pub fn parse_pubspec_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() { continue }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_deps = trimmed == "dependencies:" || trimmed == "dev_dependencies:";
            continue;
        }
        if !in_deps { continue }
        let indent = line.len() - line.trim_start().len();
        if indent != 2 && !(line.starts_with('\t') && indent == 1) { continue }
        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                packages.push(key.to_string());
            }
        }
    }
    packages
}

// ===========================================================================
// Discovery
// ===========================================================================

pub fn discover_dart_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let pubspec_path = project_root.join("pubspec.yaml");
    if !pubspec_path.is_file() { return Vec::new() }
    let Ok(pubspec_content) = std::fs::read_to_string(&pubspec_path) else {
        return Vec::new();
    };
    let declared = parse_pubspec_deps(&pubspec_content);
    if declared.is_empty() { return Vec::new() }

    // Strategy 1: .dart_tool/package_config.json
    let pkg_config = parse_dart_package_config(project_root);
    if !pkg_config.is_empty() {
        let mut result = Vec::new();
        let project_canonical = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
        for dep_name in &declared {
            if let Some(entry) = pkg_config.get(dep_name.as_str()) {
                let lib_dir = entry.root.join(&entry.package_uri);
                if !lib_dir.is_dir() { continue }
                if let Ok(canonical) = lib_dir.canonicalize() {
                    if canonical.starts_with(&project_canonical) { continue }
                }
                result.push(ExternalDepRoot {
                    module_path: dep_name.clone(),
                    version: entry.version.clone(),
                    root: lib_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
            }
        }
        debug!("Dart: {} roots via package_config.json", result.len());
        return result;
    }

    // Strategy 2: pubspec.lock + pub cache fallback
    debug!("Dart: no package_config.json; trying pubspec.lock + pub cache");
    let lock_path = project_root.join("pubspec.lock");
    let locked = if lock_path.is_file() {
        std::fs::read_to_string(&lock_path)
            .map(|c| parse_pubspec_lock(&c))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let cache_roots = find_pub_cache();
    discover_dart_externals_from_cache(project_root, &declared, locked, &cache_roots)
}

pub(crate) fn discover_dart_externals_from_cache(
    project_root: &Path,
    declared: &[String],
    locked: Vec<(String, String)>,
    cache_roots: &[PathBuf],
) -> Vec<ExternalDepRoot> {
    if cache_roots.is_empty() {
        if locked.is_empty() {
            debug!("Dart: no pubspec.lock and no pub cache; skipping");
        } else {
            debug!("Dart: {} locked deps but no pub cache; skipping", locked.len());
        }
        return Vec::new();
    }

    let version_map: std::collections::HashMap<String, String> = locked.into_iter().collect();
    let declared_set: std::collections::HashSet<&str> = declared.iter().map(|s| s.as_str()).collect();

    let mut result = Vec::new();
    let project_canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep_name in declared {
        let version = version_map.get(dep_name.as_str()).cloned().unwrap_or_default();
        for cache_root in cache_roots {
            let lib_dir = if version.is_empty() {
                find_latest_in_cache(cache_root, dep_name)
            } else {
                let pkg_dir = cache_root.join(format!("{dep_name}-{version}"));
                if pkg_dir.is_dir() { Some((pkg_dir, version.clone())) } else { None }
            };
            if let Some((pkg_dir, resolved_version)) = lib_dir {
                let candidate = pkg_dir.join("lib");
                if !candidate.is_dir() { continue }
                if let Ok(canonical) = candidate.canonicalize() {
                    if canonical.starts_with(&project_canonical) || seen.contains(&canonical) {
                        continue;
                    }
                    seen.insert(canonical);
                }
                result.push(ExternalDepRoot {
                    module_path: dep_name.clone(),
                    version: resolved_version,
                    root: candidate,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                break;
            }
        }
    }

    // Transitive deps from lock file
    for (trans_name, trans_version) in &version_map {
        if declared_set.contains(trans_name.as_str()) { continue }
        for cache_root in cache_roots {
            let pkg_dir = cache_root.join(format!("{trans_name}-{trans_version}"));
            if pkg_dir.is_dir() {
                let candidate = pkg_dir.join("lib");
                if !candidate.is_dir() { continue }
                if let Ok(canonical) = candidate.canonicalize() {
                    if canonical.starts_with(&project_canonical) || seen.contains(&canonical) {
                        continue;
                    }
                    seen.insert(canonical);
                }
                result.push(ExternalDepRoot {
                    module_path: trans_name.clone(),
                    version: trans_version.clone(),
                    root: candidate,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                break;
            }
        }
    }

    debug!("Dart: {} roots via pubspec.lock + pub cache", result.len());
    result
}

pub fn parse_pubspec_lock(content: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut current_source: Option<String> = None;
    let mut in_packages = false;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim_end();
        if trimmed == "packages:" && !raw_line.starts_with(' ') {
            in_packages = true;
            continue;
        }
        if !raw_line.starts_with(' ') && !raw_line.starts_with('\t') && !trimmed.is_empty() {
            if in_packages {
                if let (Some(name), Some(ver), Some(src)) =
                    (current_name.take(), current_version.take(), current_source.take())
                {
                    if src == "hosted" { result.push((name, ver)) }
                }
            }
            in_packages = false;
            continue;
        }
        if !in_packages { continue }

        let indent = raw_line.len() - raw_line.trim_start().len();
        if indent == 2 {
            if let (Some(name), Some(ver), Some(src)) =
                (current_name.take(), current_version.take(), current_source.take())
            {
                if src == "hosted" { result.push((name, ver)) }
            }
            let key = trimmed.trim_end_matches(':').trim();
            if !key.is_empty() { current_name = Some(key.to_string()) }
            continue;
        }
        if indent == 4 {
            if let Some(colon) = trimmed.find(':') {
                let key = trimmed[..colon].trim();
                let val = trimmed[colon + 1..].trim().trim_matches('"').to_string();
                match key {
                    "version" => current_version = Some(val),
                    "source" => current_source = Some(val),
                    _ => {}
                }
            }
        }
    }

    if let (Some(name), Some(ver), Some(src)) =
        (current_name, current_version, current_source)
    {
        if src == "hosted" { result.push((name, ver)) }
    }
    result
}

pub fn find_pub_cache() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(raw) = std::env::var_os("BEARWISDOM_DART_PUB_CACHE") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            let hosted = seg.join("hosted").join("pub.dev");
            if hosted.is_dir() { out.push(hosted); }
            else if seg.is_dir() { out.push(seg); }
        }
        if !out.is_empty() { return out }
    }
    if let Some(raw) = std::env::var_os("PUB_CACHE") {
        let base = PathBuf::from(raw);
        let hosted = base.join("hosted").join("pub.dev");
        if hosted.is_dir() { out.push(hosted); return out }
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let candidate = PathBuf::from(local_app_data)
            .join("Pub").join("Cache").join("hosted").join("pub.dev");
        if candidate.is_dir() { out.push(candidate); return out }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home {
        let candidate = PathBuf::from(home).join(".pub-cache").join("hosted").join("pub.dev");
        if candidate.is_dir() { out.push(candidate) }
    }
    out
}

fn find_latest_in_cache(cache_root: &Path, dep_name: &str) -> Option<(PathBuf, String)> {
    let prefix = format!("{dep_name}-");
    let entries = std::fs::read_dir(cache_root).ok()?;
    let mut candidates: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && e.path().is_dir() {
                let version = name[prefix.len()..].to_string();
                Some((version, e.path()))
            } else {
                None
            }
        })
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    let (version, dir) = candidates.into_iter().next_back()?;
    Some((dir, version))
}

struct DartPackageEntry {
    root: PathBuf,
    package_uri: String,
    version: String,
}

fn parse_dart_package_config(project_root: &Path) -> std::collections::HashMap<String, DartPackageEntry> {
    let config_path = project_root.join(".dart_tool").join("package_config.json");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return std::collections::HashMap::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return std::collections::HashMap::new();
    };
    let Some(packages) = json.get("packages").and_then(|v| v.as_array()) else {
        return std::collections::HashMap::new();
    };
    let config_dir = config_path.parent().unwrap_or(project_root);
    let mut map = std::collections::HashMap::new();
    for pkg in packages {
        let Some(name) = pkg.get("name").and_then(|v| v.as_str()) else { continue };
        let Some(root_uri) = pkg.get("rootUri").and_then(|v| v.as_str()) else { continue };
        let package_uri = pkg.get("packageUri").and_then(|v| v.as_str()).unwrap_or("lib/").to_string();
        let root = if root_uri.starts_with("file:///") {
            PathBuf::from(&root_uri[7..].replace('/', std::path::MAIN_SEPARATOR_STR))
        } else if root_uri.starts_with("file://") {
            PathBuf::from(&root_uri[7..].replace('/', std::path::MAIN_SEPARATOR_STR))
        } else {
            config_dir.join(root_uri.replace('/', std::path::MAIN_SEPARATOR_STR))
        };
        let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
        map.insert(name.to_string(), DartPackageEntry { root, package_uri, version });
    }
    map
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_dart_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
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
                if matches!(name, "src" | "test" | "tests" | "example" | "build" | "doc")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".dart") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:dart:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "dart",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (migrated from externals/dart.rs, uses internal parse_pubspec_deps)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let p = PubEcosystem;
        assert_eq!(p.id(), ID);
        assert_eq!(Ecosystem::kind(&p), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&p), &["dart"]);
    }

    #[test]
    fn legacy_locator_tag_is_dart() {
        assert_eq!(ExternalSourceLocator::ecosystem(&PubEcosystem), "dart");
    }

    fn make_dart_fixture(root: &Path, deps: &[&str]) {
        std::fs::create_dir_all(root).unwrap();
        let mut pubspec = "name: test_app\ndependencies:\n".to_string();
        let dart_tool = root.join(".dart_tool");
        std::fs::create_dir_all(&dart_tool).unwrap();

        let cache_dir = root.parent().unwrap().join("_dart_pub_cache");
        let mut packages = Vec::new();
        for dep in deps {
            pubspec.push_str(&format!("  {dep}: ^1.0.0\n"));
            let pkg_dir = cache_dir.join(format!("{dep}-1.0.0"));
            let lib_dir = pkg_dir.join("lib");
            std::fs::create_dir_all(&lib_dir).unwrap();
            std::fs::write(lib_dir.join(format!("{dep}.dart")), format!("class {dep}Widget {{}}\n")).unwrap();
            std::fs::create_dir_all(lib_dir.join("src")).unwrap();
            std::fs::write(lib_dir.join("src").join("internal.dart"), "class _Internal {}\n").unwrap();

            let root_uri = format!("../../_dart_pub_cache/{dep}-1.0.0");
            packages.push(serde_json::json!({
                "name": dep,
                "rootUri": root_uri,
                "packageUri": "lib/",
                "version": "1.0.0"
            }));
        }
        std::fs::write(root.join("pubspec.yaml"), &pubspec).unwrap();
        let config = serde_json::json!({ "configVersion": 2, "packages": packages });
        std::fs::write(dart_tool.join("package_config.json"), config.to_string()).unwrap();
    }

    fn cleanup_dart(name: &str) {
        let tmp = std::env::temp_dir().join(name);
        let cache = std::env::temp_dir().join("_dart_pub_cache");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn dart_discovers_declared_deps() {
        let tmp = std::env::temp_dir().join("bw-test-pub-discover");
        cleanup_dart("bw-test-pub-discover");
        make_dart_fixture(&tmp, &["http", "provider"]);

        let roots = discover_dart_externals(&tmp);
        let mut names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["http", "provider"]);
        cleanup_dart("bw-test-pub-discover");
    }

    #[test]
    fn dart_walks_lib_skips_src() {
        let tmp = std::env::temp_dir().join("bw-test-pub-walk");
        cleanup_dart("bw-test-pub-walk");
        make_dart_fixture(&tmp, &["provider"]);

        let roots = discover_dart_externals(&tmp);
        assert_eq!(roots.len(), 1);
        let files = walk_dart_root(&roots[0]);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["ext:dart:provider/provider.dart"]);
        cleanup_dart("bw-test-pub-walk");
    }

    #[test]
    fn parse_pubspec_lock_extracts_hosted_deps() {
        let content = r#"
packages:
  shelf:
    dependency: "direct main"
    description:
      name: shelf
      url: "https://pub.dev"
    source: hosted
    version: "1.4.1"
  flutter:
    dependency: "direct main"
    description: flutter
    source: sdk
    version: "0.0.0"
"#;
        let result = parse_pubspec_lock(content);
        let names: Vec<&str> = result.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"shelf"));
        assert!(!names.contains(&"flutter"));
    }

    #[test]
    fn dart_lock_cache_fallback_finds_packages() {
        let tmp = std::env::temp_dir().join("bw-test-pub-lock-fallback");
        let cache_dir = std::env::temp_dir().join("bw-test-pub-lock-fallback-cache");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&cache_dir);

        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("pubspec.yaml"),
            "name: app\ndependencies:\n  shelf: ^1.4.0\n"
        ).unwrap();
        let lock_content = "packages:
  shelf:
    dependency: \"direct main\"
    description:
      name: shelf
    source: hosted
    version: \"1.4.1\"
";
        // Build the cache fixture
        let hosted = cache_dir.join("hosted").join("pub.dev");
        let pkg_dir = hosted.join("shelf-1.4.1");
        let lib_dir = pkg_dir.join("lib");
        std::fs::create_dir_all(&lib_dir).unwrap();
        std::fs::write(lib_dir.join("shelf.dart"), "class Shelf {}").unwrap();

        let cache_roots = vec![hosted];
        let declared = vec!["shelf".to_string()];
        let locked = parse_pubspec_lock(lock_content);
        let roots = discover_dart_externals_from_cache(&tmp, &declared, locked, &cache_roots);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].module_path, "shelf");
        assert_eq!(roots[0].version, "1.4.1");

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&cache_dir);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
