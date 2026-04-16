// Dart / pub externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Dart pub cache → `discover_dart_externals` + `walk_dart_external_root`.
///
/// Dart packages are resolved via two strategies tried in order:
///
/// **Primary** — `.dart_tool/package_config.json` (Dart 2.5+). Maps every
/// dependency to its exact on-disk root including git, path, and hosted
/// packages. Requires `dart pub get` to have been run in the project.
///
/// **Fallback** — `pubspec.lock` + pub cache directory walk. `pubspec.lock`
/// carries authoritative resolved versions for every transitive dependency.
/// The pub cache directory (`PUB_CACHE` → `%LOCALAPPDATA%/Pub/Cache` on
/// Windows → `~/.pub-cache` on Unix) contains extracted package sources at
/// `hosted/pub.dev/<name>-<version>/lib/`. Useful when the project has been
/// cloned but `pub get` hasn't been run, which is common for CI-checked-in
/// lock files and standalone code-intelligence use cases.
///
/// Walk: collect `lib/**/*.dart` files, skipping `src/` (Dart convention:
/// `lib/src/` is private implementation, `lib/*.dart` is the public API).
pub struct DartExternalsLocator;

impl ExternalSourceLocator for DartExternalsLocator {
    fn ecosystem(&self) -> &'static str { "dart" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dart_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_external_root(dep)
    }
}

/// Discover external Dart package roots for a project.
///
/// Tries two resolution strategies in order:
///
/// 1. `.dart_tool/package_config.json` — exact on-disk paths, authoritative.
///    Works for any package source (hosted, git, path). Requires `dart pub get`.
///
/// 2. `pubspec.lock` + pub cache walk — version-pinned deps from the lock
///    file matched against `<pub_cache>/hosted/pub.dev/<name>-<version>/lib/`.
///    Works without `dart pub get` when the cache is pre-populated (common in
///    developer machines that have used any Flutter/Dart project).
pub fn discover_dart_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::pubspec::parse_pubspec_deps;

    let pubspec_path = project_root.join("pubspec.yaml");
    if !pubspec_path.is_file() {
        return Vec::new();
    }
    let Ok(pubspec_content) = std::fs::read_to_string(&pubspec_path) else {
        return Vec::new();
    };
    let declared = parse_pubspec_deps(&pubspec_content);
    if declared.is_empty() {
        return Vec::new();
    }

    // --- Strategy 1: package_config.json ---
    let pkg_config = parse_dart_package_config(project_root);
    if !pkg_config.is_empty() {
        let mut result = Vec::new();
        let project_canonical = project_root.canonicalize().unwrap_or_else(|_| project_root.to_path_buf());
        for dep_name in &declared {
            if let Some(entry) = pkg_config.get(dep_name.as_str()) {
                let lib_dir = entry.root.join(&entry.package_uri);
                if !lib_dir.is_dir() {
                    continue;
                }
                if let Ok(canonical) = lib_dir.canonicalize() {
                    if canonical.starts_with(&project_canonical) {
                        continue;
                    }
                }
                result.push(ExternalDepRoot {
                    module_path: dep_name.clone(),
                    version: entry.version.clone(),
                    root: lib_dir,
                    ecosystem: "dart",
                    package_id: None,
                });
            }
        }
        debug!("Dart: discovered {} external package roots via package_config.json", result.len());
        return result;
    }

    // --- Strategy 2: pubspec.lock + pub cache ---
    debug!("Dart: no package_config.json; trying pubspec.lock + pub cache fallback");
    let lock_path = project_root.join("pubspec.lock");
    let locked = if lock_path.is_file() {
        std::fs::read_to_string(&lock_path)
            .map(|c| parse_pubspec_lock(&c))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Discover cache roots once and pass them through so tests can inject a
    // deterministic path without relying on environment-variable mutation
    // (which is not thread-safe across parallel test threads).
    let cache_roots = find_pub_cache();
    discover_dart_externals_from_cache(project_root, &declared, locked, &cache_roots)
}

/// Inner implementation of the pubspec.lock + pub cache fallback strategy.
/// Separated from `discover_dart_externals` so tests can pass an explicit
/// cache root list without mutating global environment variables.
pub(crate) fn discover_dart_externals_from_cache(
    project_root: &Path,
    declared: &[String],
    locked: Vec<(String, String)>,
    cache_roots: &[PathBuf],
) -> Vec<ExternalDepRoot> {
    if cache_roots.is_empty() {
        if locked.is_empty() {
            debug!("Dart: no pubspec.lock and no pub cache; skipping Dart externals");
        } else {
            debug!("Dart: found {} locked deps but pub cache is absent; skipping Dart externals", locked.len());
        }
        return Vec::new();
    }

    // Build a name→version map from the lock file. Fall back to declared
    // deps with no version when the lock file is absent.
    let version_map: std::collections::HashMap<String, String> = locked
        .into_iter()
        .map(|(name, ver)| (name, ver))
        .collect();

    let declared_set: std::collections::HashSet<&str> = declared.iter().map(|s| s.as_str()).collect();

    let mut result = Vec::new();
    let project_canonical = project_root.canonicalize().unwrap_or_else(|_| project_root.to_path_buf());
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep_name in declared {
        let version = version_map.get(dep_name.as_str()).cloned().unwrap_or_default();

        for cache_root in cache_roots {
            let lib_dir = if version.is_empty() {
                // No pinned version — probe for any cached version by scanning
                // the hosted/pub.dev directory for `<name>-*` directories.
                find_latest_in_cache(cache_root, dep_name)
            } else {
                let pkg_dir = cache_root.join(format!("{dep_name}-{version}"));
                if pkg_dir.is_dir() { Some((pkg_dir, version.clone())) } else { None }
            };

            if let Some((pkg_dir, resolved_version)) = lib_dir {
                let candidate = pkg_dir.join("lib");
                if !candidate.is_dir() {
                    continue;
                }
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
                    ecosystem: "dart",
                    package_id: None,
                });
                break; // found in this cache root; don't probe others
            }
        }
    }

    // Also include transitive deps from pubspec.lock that are referenced by
    // declared deps. The resolver benefits from having the full transitive
    // closure, not just direct deps. We add lock-file entries not already in
    // `declared` if they appear in the cache.
    for (trans_name, trans_version) in &version_map {
        if declared_set.contains(trans_name.as_str()) {
            continue; // already handled above
        }
        for cache_root in cache_roots {
            let pkg_dir = cache_root.join(format!("{trans_name}-{trans_version}"));
            if pkg_dir.is_dir() {
                let candidate = pkg_dir.join("lib");
                if !candidate.is_dir() { continue; }
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
                    ecosystem: "dart",
                    package_id: None,
                });
                break;
            }
        }
    }

    debug!("Dart: discovered {} external package roots via pubspec.lock + pub cache", result.len());
    result
}

/// Parse a `pubspec.lock` file and return (name, version) pairs for all
/// hosted packages. The lock format is YAML but we parse it line-by-line
/// to avoid a YAML dependency — the structure is regular enough.
///
/// ```yaml
/// packages:
///   shelf:
///     dependency: "direct main"
///     description:
///       name: shelf
///       sha256: "..."
///       url: "https://pub.dev"
///     source: hosted
///     version: "1.4.1"
///   path:
///     dependency: transitive
///     ...
///     source: hosted
///     version: "1.9.1"
/// ```
///
/// We emit every package whose `source: hosted` (skipping sdk, git, path deps).
pub fn parse_pubspec_lock(content: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut current_source: Option<String> = None;
    let mut in_packages = false;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim_end();

        // Top-level `packages:` section header.
        if trimmed == "packages:" && !raw_line.starts_with(' ') {
            in_packages = true;
            continue;
        }
        // Any other top-level key resets the section.
        if !raw_line.starts_with(' ') && !raw_line.starts_with('\t') && !trimmed.is_empty() {
            if in_packages {
                // Flush the last package before leaving the section.
                if let (Some(name), Some(ver), Some(src)) =
                    (current_name.take(), current_version.take(), current_source.take())
                {
                    if src == "hosted" {
                        result.push((name, ver));
                    }
                }
            }
            in_packages = false;
            continue;
        }

        if !in_packages {
            continue;
        }

        // Two-space indent = package name key: `  shelf:`.
        let indent = raw_line.len() - raw_line.trim_start().len();
        if indent == 2 {
            // Flush previous package.
            if let (Some(name), Some(ver), Some(src)) =
                (current_name.take(), current_version.take(), current_source.take())
            {
                if src == "hosted" {
                    result.push((name, ver));
                }
            }
            let key = trimmed.trim_end_matches(':').trim();
            if !key.is_empty() {
                current_name = Some(key.to_string());
            }
            continue;
        }

        // Four-space indent = package field.
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

    // Flush the final package.
    if let (Some(name), Some(ver), Some(src)) =
        (current_name, current_version, current_source)
    {
        if src == "hosted" {
            result.push((name, ver));
        }
    }

    result
}

/// Locate the Dart pub cache `hosted/pub.dev/` directory.
///
/// Resolution order:
/// 1. `BEARWISDOM_DART_PUB_CACHE` — explicit override (test support, CI).
/// 2. `PUB_CACHE` env var — Dart SDK official override.
/// 3. `%LOCALAPPDATA%\Pub\Cache\hosted\pub.dev` — Windows default.
/// 4. `~/.pub-cache/hosted/pub.dev` — Unix/macOS default.
///
/// Returns the list of `hosted/pub.dev/` directories that exist on disk.
/// Multiple entries are theoretically possible if the env var is a
/// path-separator-delimited list (though Dart itself only supports one).
pub fn find_pub_cache() -> Vec<PathBuf> {
    let mut out = Vec::new();

    // 1. BearWisdom-specific override.
    if let Some(raw) = std::env::var_os("BEARWISDOM_DART_PUB_CACHE") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue; }
            let hosted = seg.join("hosted").join("pub.dev");
            if hosted.is_dir() {
                out.push(hosted);
            } else if seg.is_dir() {
                // Caller passed the hosted/pub.dev dir directly.
                out.push(seg);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    // 2. PUB_CACHE env var — Dart SDK official.
    if let Some(raw) = std::env::var_os("PUB_CACHE") {
        let base = PathBuf::from(raw);
        let hosted = base.join("hosted").join("pub.dev");
        if hosted.is_dir() {
            out.push(hosted);
            return out;
        }
    }

    // 3. Windows default: %LOCALAPPDATA%\Pub\Cache\hosted\pub.dev
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let candidate = PathBuf::from(local_app_data)
            .join("Pub")
            .join("Cache")
            .join("hosted")
            .join("pub.dev");
        if candidate.is_dir() {
            out.push(candidate);
            return out;
        }
    }

    // 4. Unix/macOS default: ~/.pub-cache/hosted/pub.dev
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home {
        let candidate = PathBuf::from(home)
            .join(".pub-cache")
            .join("hosted")
            .join("pub.dev");
        if candidate.is_dir() {
            out.push(candidate);
        }
    }

    out
}

/// Scan a `hosted/pub.dev/` cache root for any version of `dep_name`.
/// Returns `(pkg_dir, version)` for the lexicographically largest version
/// found (i.e. newest by semver string sorting — good enough for cache lookup).
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
        let Some(name) = pkg.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(root_uri) = pkg.get("rootUri").and_then(|v| v.as_str()) else {
            continue;
        };
        let package_uri = pkg
            .get("packageUri")
            .and_then(|v| v.as_str())
            .unwrap_or("lib/")
            .to_string();

        let root = if root_uri.starts_with("file:///") {
            PathBuf::from(&root_uri[7..].replace('/', std::path::MAIN_SEPARATOR_STR))
        } else if root_uri.starts_with("file://") {
            PathBuf::from(&root_uri[7..].replace('/', std::path::MAIN_SEPARATOR_STR))
        } else {
            config_dir.join(root_uri.replace('/', std::path::MAIN_SEPARATOR_STR))
        };

        let version = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        map.insert(name.to_string(), DartPackageEntry {
            root,
            package_uri,
            version,
        });
    }
    map
}

/// Walk a Dart package's public API directory (`lib/`).
///
/// Collects `*.dart` files, skipping `src/` (Dart convention: `lib/src/`
/// is private implementation, `lib/*.dart` is the public API). Also skips
/// test, build, and hidden directories.
pub fn walk_dart_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dart_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_dart_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_dart_dir_bounded(dir, root, dep, out, 0);
}

fn walk_dart_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "src" | "test" | "tests" | "example" | "build" | "doc"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_dart_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".dart") {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:dart:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "dart",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let tmp = std::env::temp_dir().join("bw-test-dart-discover");
        cleanup_dart("bw-test-dart-discover");
        make_dart_fixture(&tmp, &["http", "provider"]);

        let roots = discover_dart_externals(&tmp);
        let mut names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["http", "provider"]);
        assert_eq!(roots[0].ecosystem, "dart");

        cleanup_dart("bw-test-dart-discover");
    }

    #[test]
    fn dart_walks_lib_skips_src() {
        let tmp = std::env::temp_dir().join("bw-test-dart-walk");
        cleanup_dart("bw-test-dart-walk");
        make_dart_fixture(&tmp, &["provider"]);

        let roots = discover_dart_externals(&tmp);
        assert_eq!(roots.len(), 1);

        let files = walk_dart_external_root(&roots[0]);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["ext:dart:provider/provider.dart"]);

        cleanup_dart("bw-test-dart-walk");
    }

    #[test]
    fn dart_skips_undeclared_packages() {
        let tmp = std::env::temp_dir().join("bw-test-dart-undeclared");
        cleanup_dart("bw-test-dart-undeclared");
        make_dart_fixture(&tmp, &["http"]);
        let dart_tool = tmp.join(".dart_tool");
        let config_str = std::fs::read_to_string(dart_tool.join("package_config.json")).unwrap();
        let mut config: serde_json::Value = serde_json::from_str(&config_str).unwrap();
        config["packages"].as_array_mut().unwrap().push(serde_json::json!({
            "name": "rogue_pkg",
            "rootUri": "../../_dart_pub_cache/rogue_pkg-0.1.0",
            "packageUri": "lib/",
            "version": "0.1.0"
        }));
        std::fs::write(dart_tool.join("package_config.json"), config.to_string()).unwrap();

        let roots = discover_dart_externals(&tmp);
        let names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        assert_eq!(names, vec!["http".to_string()]);

        cleanup_dart("bw-test-dart-undeclared");
    }

    #[test]
    fn dart_empty_without_package_config() {
        let tmp = std::env::temp_dir().join("bw-test-dart-no-config");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("pubspec.yaml"), "name: app\ndependencies:\n  http: ^1.0.0\n").unwrap();

        // No package_config.json and no pub cache on disk → empty.
        let roots = discover_dart_externals(&tmp);
        assert!(roots.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_pubspec_lock_extracts_hosted_deps() {
        let content = r#"
# Generated by pub tool; do not edit.
packages:
  equatable:
    dependency: "direct main"
    description:
      name: equatable
      sha256: "abc123"
      url: "https://pub.dev"
    source: hosted
    version: "2.0.5"
  shelf:
    dependency: "direct main"
    description:
      name: shelf
      sha256: "def456"
      url: "https://pub.dev"
    source: hosted
    version: "1.4.1"
  flutter:
    dependency: "direct main"
    description: flutter
    source: sdk
    version: "0.0.0"
  some_path_pkg:
    dependency: "direct main"
    description:
      path: "../some_path_pkg"
      relative: true
    source: path
    version: "1.0.0"
  mocktail:
    dependency: "direct dev"
    description:
      name: mocktail
      sha256: "ghi789"
      url: "https://pub.dev"
    source: hosted
    version: "1.0.4"
sdks:
  dart: ">=3.0.0 <4.0.0"
"#;
        let result = parse_pubspec_lock(content);
        let names: Vec<&str> = result.iter().map(|(n, _)| n.as_str()).collect();
        // sdk and path deps excluded; hosted deps included
        assert!(names.contains(&"equatable"), "missing equatable");
        assert!(names.contains(&"shelf"), "missing shelf");
        assert!(names.contains(&"mocktail"), "missing mocktail");
        assert!(!names.contains(&"flutter"), "sdk dep should be excluded");
        assert!(!names.contains(&"some_path_pkg"), "path dep should be excluded");

        // Check versions
        let version_map: std::collections::HashMap<&str, &str> =
            result.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        assert_eq!(version_map["equatable"], "2.0.5");
        assert_eq!(version_map["shelf"], "1.4.1");
        assert_eq!(version_map["mocktail"], "1.0.4");
    }

    /// Build a fake pub cache fixture at `cache_root/hosted/pub.dev/`.
    fn make_pub_cache_fixture(cache_root: &Path, pkgs: &[(&str, &str)]) {
        let hosted = cache_root.join("hosted").join("pub.dev");
        for (name, ver) in pkgs {
            let pkg_dir = hosted.join(format!("{name}-{ver}"));
            let lib_dir = pkg_dir.join("lib");
            std::fs::create_dir_all(&lib_dir).unwrap();
            std::fs::write(
                lib_dir.join(format!("{name}.dart")),
                format!("class {}Class {{}}\n", name),
            ).unwrap();
        }
    }

    #[test]
    fn dart_lock_cache_fallback_finds_packages() {
        let tmp = std::env::temp_dir().join("bw-test-dart-lock-fallback");
        let cache_dir = std::env::temp_dir().join("bw-test-dart-lock-fallback-cache");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&cache_dir);

        // Set up project with pubspec.yaml + pubspec.lock but NO package_config.json.
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("pubspec.yaml"),
            "name: app\ndependencies:\n  shelf: ^1.4.0\n  equatable: ^2.0.0\n"
        ).unwrap();
        let lock_content = "packages:
  shelf:
    dependency: \"direct main\"
    description:
      name: shelf
      url: \"https://pub.dev\"
    source: hosted
    version: \"1.4.1\"
  equatable:
    dependency: \"direct main\"
    description:
      name: equatable
      url: \"https://pub.dev\"
    source: hosted
    version: \"2.0.5\"
sdks:
  dart: \">=3.0.0 <4.0.0\"
";
        std::fs::write(tmp.join("pubspec.lock"), lock_content).unwrap();

        // Set up fake pub cache with the two packages.
        make_pub_cache_fixture(&cache_dir, &[("shelf", "1.4.1"), ("equatable", "2.0.5")]);
        let cache_roots = vec![cache_dir.join("hosted").join("pub.dev")];

        // Call the inner function directly — avoids env-var mutation races in
        // parallel test threads.
        use crate::indexer::manifest::pubspec::parse_pubspec_deps;
        let pubspec_content = std::fs::read_to_string(tmp.join("pubspec.yaml")).unwrap();
        let declared = parse_pubspec_deps(&pubspec_content);
        let locked = parse_pubspec_lock(lock_content);

        let roots = discover_dart_externals_from_cache(&tmp, &declared, locked, &cache_roots);

        let mut names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["equatable", "shelf"]);

        let version_map: std::collections::HashMap<&str, &str> =
            roots.iter().map(|r| (r.module_path.as_str(), r.version.as_str())).collect();
        assert_eq!(version_map["shelf"], "1.4.1");
        assert_eq!(version_map["equatable"], "2.0.5");

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&cache_dir);
    }

    #[test]
    fn dart_lock_fallback_includes_transitive_deps() {
        let tmp = std::env::temp_dir().join("bw-test-dart-transitive");
        let cache_dir = std::env::temp_dir().join("bw-test-dart-transitive-cache");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&cache_dir);

        // Direct dep: shelf. Transitive dep: shelf_web_socket (in lock, not pubspec).
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("pubspec.yaml"),
            "name: app\ndependencies:\n  shelf: ^1.4.0\n"
        ).unwrap();
        let lock_content = "packages:
  shelf:
    dependency: \"direct main\"
    description:
      name: shelf
      url: \"https://pub.dev\"
    source: hosted
    version: \"1.4.1\"
  shelf_web_socket:
    dependency: transitive
    description:
      name: shelf_web_socket
      url: \"https://pub.dev\"
    source: hosted
    version: \"2.0.0\"
sdks:
  dart: \">=3.0.0 <4.0.0\"
";
        std::fs::write(tmp.join("pubspec.lock"), lock_content).unwrap();
        make_pub_cache_fixture(&cache_dir, &[("shelf", "1.4.1"), ("shelf_web_socket", "2.0.0")]);
        let cache_roots = vec![cache_dir.join("hosted").join("pub.dev")];

        use crate::indexer::manifest::pubspec::parse_pubspec_deps;
        let pubspec_content = std::fs::read_to_string(tmp.join("pubspec.yaml")).unwrap();
        let declared = parse_pubspec_deps(&pubspec_content);
        let locked = parse_pubspec_lock(lock_content);

        let roots = discover_dart_externals_from_cache(&tmp, &declared, locked, &cache_roots);

        let names: std::collections::HashSet<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        assert!(names.contains("shelf"), "direct dep shelf should be found");
        assert!(names.contains("shelf_web_socket"), "transitive dep should be included");

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&cache_dir);
    }
}
