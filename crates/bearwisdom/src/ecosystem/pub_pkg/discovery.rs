// ===========================================================================
// Discovery
// ===========================================================================

use std::path::{Path, PathBuf};

use tracing::debug;

use super::manifest::parse_pubspec_deps;
use super::LEGACY_ECOSYSTEM_TAG;
use crate::ecosystem::externals::ExternalDepRoot;

pub fn discover_dart_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
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

    // Strategy 1: .dart_tool/package_config.json
    //
    // Walks every package the resolver wrote into the config — declared
    // deps AND their transitives. Pub re-export chains routinely hand
    // public types out from a transitive (`hooks_riverpod` re-exports
    // `package:flutter_riverpod/...` which defines `WidgetRef` and
    // `ConsumerWidget`); restricting discovery to direct deps makes those
    // types unindexable because `expand_dart_exports_into` skips
    // cross-package `export` specs by design (each package is its own
    // root). The Strategy 2 lock-file path already includes transitives;
    // this brings package_config.json behavior in line.
    let pkg_config = parse_dart_package_config(project_root);
    if !pkg_config.is_empty() {
        let mut result = Vec::new();
        let project_canonical = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
        for (pkg_name, entry) in &pkg_config {
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
                module_path: pkg_name.clone(),
                version: entry.version.clone(),
                root: lib_dir,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
        debug!(
            "Dart: {} roots via package_config.json (declared+transitive)",
            result.len()
        );
        let _ = declared; // declared retained above for the early-return guard
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
            debug!(
                "Dart: {} locked deps but no pub cache; skipping",
                locked.len()
            );
        }
        return Vec::new();
    }

    let version_map: std::collections::HashMap<String, String> = locked.into_iter().collect();
    let declared_set: std::collections::HashSet<&str> =
        declared.iter().map(|s| s.as_str()).collect();

    let mut result = Vec::new();
    let project_canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep_name in declared {
        let version = version_map
            .get(dep_name.as_str())
            .cloned()
            .unwrap_or_default();
        for cache_root in cache_roots {
            let lib_dir = if version.is_empty() {
                find_latest_in_cache(cache_root, dep_name)
            } else {
                let pkg_dir = cache_root.join(format!("{dep_name}-{version}"));
                if pkg_dir.is_dir() {
                    Some((pkg_dir, version.clone()))
                } else {
                    None
                }
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
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
                break;
            }
        }
    }

    // Transitive deps from lock file
    for (trans_name, trans_version) in &version_map {
        if declared_set.contains(trans_name.as_str()) {
            continue;
        }
        for cache_root in cache_roots {
            let pkg_dir = cache_root.join(format!("{trans_name}-{trans_version}"));
            if pkg_dir.is_dir() {
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
                    module_path: trans_name.clone(),
                    version: trans_version.clone(),
                    root: candidate,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
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
                if let (Some(name), Some(ver), Some(src)) = (
                    current_name.take(),
                    current_version.take(),
                    current_source.take(),
                ) {
                    if src == "hosted" {
                        result.push((name, ver))
                    }
                }
            }
            in_packages = false;
            continue;
        }
        if !in_packages {
            continue;
        }

        let indent = raw_line.len() - raw_line.trim_start().len();
        if indent == 2 {
            if let (Some(name), Some(ver), Some(src)) = (
                current_name.take(),
                current_version.take(),
                current_source.take(),
            ) {
                if src == "hosted" {
                    result.push((name, ver))
                }
            }
            let key = trimmed.trim_end_matches(':').trim();
            if !key.is_empty() {
                current_name = Some(key.to_string())
            }
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

    if let (Some(name), Some(ver), Some(src)) = (current_name, current_version, current_source) {
        if src == "hosted" {
            result.push((name, ver))
        }
    }
    result
}

pub fn find_pub_cache() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(raw) = std::env::var_os("BEARWISDOM_DART_PUB_CACHE") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() {
                continue;
            }
            let hosted = seg.join("hosted").join("pub.dev");
            if hosted.is_dir() {
                out.push(hosted);
            } else if seg.is_dir() {
                out.push(seg);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    if let Some(raw) = std::env::var_os("PUB_CACHE") {
        let base = PathBuf::from(raw);
        let hosted = base.join("hosted").join("pub.dev");
        if hosted.is_dir() {
            out.push(hosted);
            return out;
        }
    }
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
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home {
        let candidate = PathBuf::from(home)
            .join(".pub-cache")
            .join("hosted")
            .join("pub.dev");
        if candidate.is_dir() {
            out.push(candidate)
        }
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

fn parse_dart_package_config(
    project_root: &Path,
) -> std::collections::HashMap<String, DartPackageEntry> {
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
        let root = parse_file_uri(root_uri, config_dir);
        let version = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        map.insert(
            name.to_string(),
            DartPackageEntry {
                root,
                package_uri,
                version,
            },
        );
    }
    map
}

/// Resolve a `file:///`-style URI from `package_config.json` to a usable
/// `PathBuf`. On Windows, `file:///C:/foo/bar` must drop the eight-byte
/// `file:///` prefix to leave `C:/foo/bar`; on Unix the same `file:///`
/// signals an absolute path so we drop seven (`file://`) and keep the
/// remaining leading slash. Earlier code stripped seven on both, leaving
/// `/C:/foo/bar` on Windows, which `PathBuf::from` interpreted as a
/// drive-rooted path on the current drive — every `is_dir()` check then
/// returned false and the entire pub-cache pipeline was a no-op for
/// monorepos with `package_config.json` under non-default project roots.
fn parse_file_uri(uri: &str, fallback_dir: &Path) -> PathBuf {
    if let Some(after_scheme) = uri.strip_prefix("file:///") {
        if cfg!(windows) {
            PathBuf::from(after_scheme)
        } else {
            PathBuf::from(format!("/{}", after_scheme))
        }
    } else if let Some(after_scheme) = uri.strip_prefix("file://") {
        PathBuf::from(after_scheme)
    } else {
        fallback_dir.join(uri)
    }
}
