// Swift / SPM externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Swift SPM → `discover_swift_externals` + `walk_swift_external_root`.
///
/// Manifest resolution priority:
///   1. `Package.resolved` (authoritative JSON, v2/v3 format) — searched at:
///      a. `<project>/Package.resolved`  (SPM standalone)
///      b. `<project>/*.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved`
///         (Xcode-managed project)
///      c. `<project>/*.xcworkspace/xcshareddata/swiftpm/Package.resolved`
///         (Xcode workspace)
///   2. `Package.swift` line-parse fallback (for projects that have run neither
///      `swift package resolve` nor opened in Xcode).
///
/// Cache resolution (each path probed in order):
///   1. `<project>/.build/checkouts/<identity>/`  — `swift package resolve` / `swift build`
///   2. `<project>/SourcePackages/checkouts/<identity>/`  — older Xcode local cache
///   3. `~/Library/Developer/Xcode/DerivedData/*/SourcePackages/checkouts/<identity>/`
///      — macOS Xcode DerivedData (one glob level for the hashed project name)
///   4. `%LOCALAPPDATA%/swift/SourcePackages/checkouts/<identity>/`  — Swift on Windows
///
/// Walk: `Sources/**/*.swift`, skipping `Tests/`, `Examples/`, `Benchmarks/`.
pub struct SwiftExternalsLocator;

impl ExternalSourceLocator for SwiftExternalsLocator {
    fn ecosystem(&self) -> &'static str { "swift" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_swift_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_swift_external_root(dep)
    }
}

// ---------------------------------------------------------------------------
// Package.resolved JSON structures (v2 and v3 format share the same pin schema)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

pub fn discover_swift_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    // Step 1: resolve the package list — from Package.resolved (preferred) or
    // Package.swift (fallback).
    let pins = find_and_parse_package_resolved(project_root).or_else(|| {
        use crate::indexer::manifest::swift_pm::parse_swift_package_deps;
        let package_swift = project_root.join("Package.swift");
        let content = std::fs::read_to_string(&package_swift).ok()?;
        let deps = parse_swift_package_deps(&content);
        if deps.is_empty() {
            return None;
        }
        debug!(
            "Swift: Package.resolved not found — using {} deps from Package.swift",
            deps.len()
        );
        Some(deps.into_iter().map(|name| (name, String::new())).collect())
    });

    let Some(pins) = pins else {
        debug!(
            "Swift: no Package.resolved or Package.swift found at {}",
            project_root.display()
        );
        return Vec::new();
    };

    if pins.is_empty() {
        return Vec::new();
    }

    // Step 2: locate checkout caches.
    let checkout_roots = find_checkout_roots(project_root);
    if checkout_roots.is_empty() {
        debug!(
            "Swift: no SPM checkout cache found for {}; pipeline correct but no external files to index",
            project_root.display()
        );
        return Vec::new();
    }

    // Step 3: for each pin, probe checkout dirs.
    let mut roots = Vec::new();
    for (identity, version) in &pins {
        for checkout_root in &checkout_roots {
            // SPM uses the identity verbatim as the checkout directory name.
            let dep_dir = checkout_root.join(identity);
            if dep_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: identity.clone(),
                    version: version.clone(),
                    root: dep_dir,
                    ecosystem: "swift",
                    package_id: None,
                });
                break;
            }
        }
    }

    debug!("Swift: discovered {} external package roots", roots.len());
    roots
}

/// Parse `Package.resolved` searching at multiple known paths relative to
/// `project_root`. Returns `Some(Vec<(identity, version)>)` on first match.
fn find_and_parse_package_resolved(project_root: &Path) -> Option<Vec<(String, String)>> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. SPM standalone: `<project>/Package.resolved`
    candidates.push(project_root.join("Package.resolved"));

    // 2. Xcode project / workspace layouts
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".xcodeproj") {
                candidates.push(
                    path.join("project.xcworkspace")
                        .join("xcshareddata")
                        .join("swiftpm")
                        .join("Package.resolved"),
                );
            } else if name.ends_with(".xcworkspace") {
                candidates.push(
                    path.join("xcshareddata")
                        .join("swiftpm")
                        .join("Package.resolved"),
                );
            }
        }
    }

    for path in &candidates {
        if !path.is_file() {
            continue;
        }
        if let Some(pins) = parse_package_resolved(path) {
            debug!(
                "Swift: parsed {} pins from {}",
                pins.len(),
                path.display()
            );
            return Some(pins);
        }
    }
    None
}

/// Parse a `Package.resolved` JSON file (v2 and v3 formats).
fn parse_package_resolved(path: &Path) -> Option<Vec<(String, String)>> {
    let content = std::fs::read_to_string(path).ok()?;
    let resolved: PackageResolved = serde_json::from_str(&content).ok()?;
    let pins = resolved
        .pins
        .into_iter()
        .map(|p| {
            let version = p.state.version.unwrap_or_default();
            (p.identity, version)
        })
        .collect::<Vec<_>>();
    Some(pins)
}

/// Build the ordered list of `checkouts/` directories to search.
///
/// Search order:
///   1. `<project>/.build/checkouts/`  — standard SPM CLI
///   2. `<project>/SourcePackages/checkouts/`  — older Xcode local layout
///   3. `~/Library/Developer/Xcode/DerivedData/*/SourcePackages/checkouts/`  — macOS Xcode
///   4. `%LOCALAPPDATA%/swift/SourcePackages/checkouts/`  — Swift on Windows
fn find_checkout_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    // 1. SPM CLI
    let build_checkouts = project_root.join(".build").join("checkouts");
    if build_checkouts.is_dir() {
        roots.push(build_checkouts);
    }

    // 2. Xcode local SourcePackages
    let local_sp = project_root.join("SourcePackages").join("checkouts");
    if local_sp.is_dir() {
        roots.push(local_sp);
    }

    // 3. macOS Xcode DerivedData (one hashed project-name level)
    if let Some(home) = dirs::home_dir() {
        let derived_data = home
            .join("Library")
            .join("Developer")
            .join("Xcode")
            .join("DerivedData");
        if derived_data.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&derived_data) {
                for entry in entries.flatten() {
                    let sp = entry.path().join("SourcePackages").join("checkouts");
                    if sp.is_dir() {
                        roots.push(sp);
                    }
                }
            }
        }
    }

    // 4. Swift on Windows
    if let Some(local_app) = std::env::var_os("LOCALAPPDATA") {
        let win_sp = PathBuf::from(local_app)
            .join("swift")
            .join("SourcePackages")
            .join("checkouts");
        if win_sp.is_dir() {
            roots.push(win_sp);
        }
    }

    roots
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

pub fn walk_swift_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let sources = dep.root.join("Sources");
    let walk_root = if sources.is_dir() { sources } else { dep.root.clone() };
    walk_swift_dir(&walk_root, &dep.root, dep, &mut out);
    out
}

fn walk_swift_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_swift_dir_bounded(dir, root, dep, out, 0);
}

fn walk_swift_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
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
                if matches!(name, "Tests" | "tests" | "Examples" | "Benchmarks")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_swift_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".swift") {
                continue;
            }
            if name.ends_with("Tests.swift") || name.ends_with("Test.swift") {
                continue;
            }
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
  "originHash" : "abc123",
  "pins" : [
    {
      "identity" : "bodega",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/mergesort/Bodega",
      "state" : {
        "revision" : "bfd8871e9c2590d31b200e54c75428a71483afdf",
        "version" : "2.1.3"
      }
    },
    {
      "identity" : "gifu",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/kaishin/Gifu.git",
      "state" : {
        "revision" : "f19726eaf0dfa4dbce4d3f80293c9d38c2acba53",
        "version" : "4.0.1"
      }
    },
    {
      "identity" : "emojitext",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/Dimillian/EmojiText",
      "state" : {
        "branch" : "fix-ios26",
        "revision" : "3b11459a19c9406176a08b0f0599b98f88113296"
      }
    }
  ],
  "version" : 3
}"#;

    #[test]
    fn parse_package_resolved_v3_with_version() {
        let tmp = std::env::temp_dir().join("bw-test-swift-resolved-v3");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let resolved_path = tmp.join("Package.resolved");
        std::fs::write(&resolved_path, PACKAGE_RESOLVED_V3).unwrap();

        let pins = parse_package_resolved(&resolved_path).unwrap();
        assert_eq!(pins.len(), 3);

        let bodega = pins.iter().find(|(id, _)| id == "bodega").unwrap();
        assert_eq!(bodega.1, "2.1.3");

        let gifu = pins.iter().find(|(id, _)| id == "gifu").unwrap();
        assert_eq!(gifu.1, "4.0.1");

        // Branch pin — no version field.
        let emoji = pins.iter().find(|(id, _)| id == "emojitext").unwrap();
        assert_eq!(emoji.1, "", "branch pin should yield empty version string");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_finds_xcodeproj_package_resolved() {
        let tmp = std::env::temp_dir().join("bw-test-swift-xcodeproj-resolved");
        let _ = std::fs::remove_dir_all(&tmp);

        // Simulate: <project>/MyApp.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
        let resolved_dir = tmp
            .join("MyApp.xcodeproj")
            .join("project.xcworkspace")
            .join("xcshareddata")
            .join("swiftpm");
        std::fs::create_dir_all(&resolved_dir).unwrap();
        std::fs::write(resolved_dir.join("Package.resolved"), PACKAGE_RESOLVED_V3).unwrap();

        // SPM checkouts alongside the project.
        let checkouts = tmp.join(".build").join("checkouts");
        for pkg in &["bodega", "gifu", "emojitext"] {
            let src_dir = checkouts.join(pkg).join("Sources").join(pkg);
            std::fs::create_dir_all(&src_dir).unwrap();
            std::fs::write(src_dir.join("Main.swift"), format!("// {pkg}\n")).unwrap();
        }

        let roots = discover_swift_externals(&tmp);
        assert_eq!(roots.len(), 3, "expected one root per pin");

        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        assert!(names.contains("bodega"));
        assert!(names.contains("gifu"));
        assert!(names.contains("emojitext"));

        let bodega = roots.iter().find(|r| r.module_path == "bodega").unwrap();
        assert_eq!(bodega.version, "2.1.3");

        let emoji = roots.iter().find(|r| r.module_path == "emojitext").unwrap();
        assert_eq!(emoji.version, "");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn walk_excludes_test_files_and_dirs() {
        let tmp = std::env::temp_dir().join("bw-test-swift-walk-filter");
        let _ = std::fs::remove_dir_all(&tmp);

        let pkg_root = tmp.join("gifu");
        let src = pkg_root.join("Sources").join("Gifu");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("Animator.swift"), "class Animator {}\n").unwrap();
        std::fs::write(src.join("AnimatorTests.swift"), "class AnimatorTests {}\n").unwrap();

        let tests_dir = pkg_root.join("Tests").join("GifuTests");
        std::fs::create_dir_all(&tests_dir).unwrap();
        std::fs::write(tests_dir.join("AnimatorTest.swift"), "class AnimatorTest {}\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "gifu".to_string(),
            version: "4.0.1".to_string(),
            root: pkg_root,
            ecosystem: "swift",
            package_id: None,
        };
        let walked = walk_swift_external_root(&dep);
        assert_eq!(walked.len(), 1, "only Animator.swift should be walked");
        assert!(walked[0].relative_path.ends_with("Animator.swift"));
        assert!(walked[0].relative_path.starts_with("ext:swift:gifu/"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_returns_empty_when_no_checkout_cache() {
        let tmp = std::env::temp_dir().join("bw-test-swift-no-cache");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Package.resolved"), PACKAGE_RESOLVED_V3).unwrap();
        // No .build/checkouts — should return empty, not error.
        let roots = discover_swift_externals(&tmp);
        assert!(roots.is_empty(), "no cache = empty roots, not an error");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
