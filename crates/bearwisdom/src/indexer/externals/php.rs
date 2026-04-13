// PHP / Composer vendor directory externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::Path;
use tracing::debug;

/// Composer vendor dir → `discover_php_externals` + `walk_php_external_root`.
///
/// PHP packages installed via Composer live in `vendor/<vendor>/<package>/`.
/// Declared deps come from `composer.json` `require` + `require-dev`.
/// Walk: `src/**/*.php` (PSR-4 convention), skipping `tests/`, `vendor/`.
pub struct PhpExternalsLocator;

impl ExternalSourceLocator for PhpExternalsLocator {
    fn ecosystem(&self) -> &'static str { "php" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_php_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_php_external_root(dep)
    }
}

pub fn discover_php_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::composer::parse_composer_json_deps;

    let composer_path = project_root.join("composer.json");
    if !composer_path.is_file() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&composer_path) else {
        return Vec::new();
    };
    let declared = parse_composer_json_deps(&content);
    if declared.is_empty() {
        return Vec::new();
    }

    let vendor = project_root.join("vendor");
    if !vendor.is_dir() {
        return Vec::new();
    }

    let mut roots = Vec::new();
    for dep in &declared {
        // Composer packages are vendor/name format: "laravel/framework" → vendor/laravel/framework/
        let pkg_dir = vendor.join(dep.replace('/', std::path::MAIN_SEPARATOR_STR));
        if pkg_dir.is_dir() {
            let version = read_composer_version(&pkg_dir);
            roots.push(ExternalDepRoot {
                module_path: dep.clone(),
                version,
                root: pkg_dir,
                ecosystem: "php",
            });
        }
    }
    debug!("PHP: discovered {} external package roots", roots.len());
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

pub fn walk_php_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    // Prefer src/ if it exists (PSR-4), otherwise walk root
    let walk_root = if dep.root.join("src").is_dir() {
        dep.root.join("src")
    } else {
        dep.root.clone()
    };
    walk_php_dir(&walk_root, &dep.root, dep, &mut out);
    out
}

fn walk_php_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_php_dir_bounded(dir, root, dep, out, 0);
}

fn walk_php_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "Tests" | "Test" | "vendor" | "docs" | "examples")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_php_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".php") { continue; }
            if name.ends_with("Test.php") || name.ends_with("Tests.php") { continue; }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn php_discovers_composer_deps() {
        let tmp = std::env::temp_dir().join("bw-test-php-discover");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("composer.json"), r#"{"require":{"laravel/framework":"^11.0","guzzlehttp/guzzle":"^7.0"}}"#).unwrap();
        let vendor = tmp.join("vendor");
        let laravel = vendor.join("laravel").join("framework").join("src");
        std::fs::create_dir_all(&laravel).unwrap();
        std::fs::write(laravel.join("Application.php"), "<?php class Application {}\n").unwrap();
        let guzzle = vendor.join("guzzlehttp").join("guzzle").join("src");
        std::fs::create_dir_all(&guzzle).unwrap();
        std::fs::write(guzzle.join("Client.php"), "<?php class Client {}\n").unwrap();

        let roots = discover_php_externals(&tmp);
        let mut names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["guzzlehttp/guzzle", "laravel/framework"]);

        let files = walk_php_external_root(&roots.iter().find(|r| r.module_path == "laravel/framework").unwrap());
        assert_eq!(files.len(), 1);
        assert!(files[0].relative_path.contains("Application.php"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
