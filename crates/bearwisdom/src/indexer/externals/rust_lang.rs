// Rust / Cargo externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Rust Cargo externals pipeline.
///
/// Crate source cache layout:
///   `$CARGO_HOME/registry/src/<index-hash>/<name>-<version>/src/`
///
/// Package list: prefer `Cargo.lock` (full resolved tree, exact versions) over
/// `Cargo.toml` declared deps. `Cargo.lock` is searched upward from
/// `project_root` (workspace root pattern) and as a fallback descends into
/// immediate subdirectories (mixed-language repos like sql-pgmq where the
/// Rust sub-project lives under a nested directory).
///
/// Walk: `src/**/*.rs`, skipping `tests/`, `benches/`, `examples/`, `target/`.
pub struct RustExternalsLocator;

impl ExternalSourceLocator for RustExternalsLocator {
    fn ecosystem(&self) -> &'static str { "rust" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_rust_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_rust_external_root(dep)
    }
}

// ---------------------------------------------------------------------------
// Cargo.lock parser
// ---------------------------------------------------------------------------

/// A resolved crate entry from `Cargo.lock`.
#[derive(Debug, Clone)]
struct CargoLockEntry {
    name: String,
    version: String,
}

/// Parse `[[package]]` entries from a `Cargo.lock` file.
///
/// Only returns packages with `source = "registry+..."` — workspace members
/// and git deps are omitted (no crates.io cache entry to walk).
pub fn parse_cargo_lock(content: &str) -> Vec<CargoLockEntry> {
    let mut entries = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut current_is_registry = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "[[package]]" {
            if current_is_registry {
                if let (Some(name), Some(version)) =
                    (current_name.take(), current_version.take())
                {
                    entries.push(CargoLockEntry { name, version });
                }
            } else {
                current_name = None;
                current_version = None;
            }
            current_is_registry = false;
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Parse `key = "value"` assignments — skip list lines.
        let Some(eq) = trimmed.find(" = ") else { continue };
        let key = trimmed[..eq].trim();
        let rest = trimmed[eq + 3..].trim();
        let value = rest.trim_matches('"');

        match key {
            "name" => { current_name = Some(value.to_string()); }
            "version" => { current_version = Some(value.to_string()); }
            "source" => { current_is_registry = value.starts_with("registry+"); }
            _ => {}
        }
    }

    // Flush the last entry.
    if current_is_registry {
        if let (Some(name), Some(version)) = (current_name, current_version) {
            entries.push(CargoLockEntry { name, version });
        }
    }

    entries
}

// ---------------------------------------------------------------------------
// Lock-file discovery
// ---------------------------------------------------------------------------

/// Find `Cargo.lock` by walking up from `start` to filesystem root.
/// Caps at 8 levels — covers any realistic monorepo nesting depth.
fn find_cargo_lock(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    for _ in 0..8 {
        let lock = current.join("Cargo.lock");
        if lock.is_file() {
            return Some(lock);
        }
        current = current.parent()?;
    }
    None
}

/// Find `Cargo.lock` by descending into immediate subdirectories (depth <= 2).
///
/// Handles mixed-language repos where the Rust sub-project lives in a nested
/// directory (e.g. `sql-pgmq/pgmq-rs/Cargo.lock`) but the indexer is called
/// on the parent root. Only used when `find_cargo_lock` returns `None`.
fn find_cargo_lock_descend(start: &Path) -> Option<PathBuf> {
    find_cargo_lock_descend_bounded(start, 0)
}

fn find_cargo_lock_descend_bounded(dir: &Path, depth: u8) -> Option<PathBuf> {
    if depth > 2 {
        return None;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return None };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.lock") {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "target" | ".git" | "node_modules") || name.starts_with('.') {
                    continue;
                }
            }
            if let Some(found) = find_cargo_lock_descend_bounded(&path, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Registry source directory discovery
// ---------------------------------------------------------------------------

/// Return all `registry/src/<index-hash>/` subdirectories under CARGO_HOME.
/// Multiple index-hash dirs can coexist; search all of them.
fn cargo_registry_src_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let src_root = if let Ok(home) = std::env::var("CARGO_HOME") {
        PathBuf::from(home).join("registry").join("src")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".cargo").join("registry").join("src")
    } else {
        return dirs;
    };

    if !src_root.is_dir() {
        return dirs;
    }

    if let Ok(entries) = std::fs::read_dir(&src_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    dirs
}

/// Split a crate directory name like `proc-macro2-1.0.91` into `(name, version)`.
/// Finds the last `-<digit>` boundary — correctly handles hyphenated crate names.
fn split_crate_dir_name(s: &str) -> Option<(String, String)> {
    let bytes = s.as_bytes();
    let mut i = s.len();
    while let Some(pos) = s[..i].rfind('-') {
        if bytes.get(pos + 1).map_or(false, |b| b.is_ascii_digit()) {
            return Some((s[..pos].to_string(), s[pos + 1..].to_string()));
        }
        i = pos;
    }
    None
}

// ---------------------------------------------------------------------------
// Public discovery + walk
// ---------------------------------------------------------------------------

pub fn discover_rust_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::cargo::parse_cargo_dependencies;

    // 1. Resolve package list. Prefer Cargo.lock (full resolved tree with
    //    exact versions). Search upward first, then descend for mixed repos.
    let lock_path = find_cargo_lock(project_root)
        .or_else(|| find_cargo_lock_descend(project_root));

    let packages: Vec<CargoLockEntry> = if let Some(ref lp) = lock_path {
        if let Ok(content) = std::fs::read_to_string(lp) {
            let parsed = parse_cargo_lock(&content);
            if !parsed.is_empty() {
                debug!(
                    "Rust: loaded {} packages from Cargo.lock at {}",
                    parsed.len(),
                    lp.display()
                );
                parsed
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Fall back to declared-only deps from Cargo.toml when no lockfile.
    let use_fallback = packages.is_empty();
    let toml_names: Vec<String> = if use_fallback {
        let cargo_toml = project_root.join("Cargo.toml");
        if !cargo_toml.is_file() {
            return Vec::new();
        }
        match std::fs::read_to_string(&cargo_toml) {
            Ok(content) => {
                let deps = parse_cargo_dependencies(&content);
                if deps.is_empty() {
                    return Vec::new();
                }
                debug!(
                    "Rust: no Cargo.lock; using {} declared deps from Cargo.toml",
                    deps.len()
                );
                deps
            }
            Err(_) => return Vec::new(),
        }
    } else {
        Vec::new()
    };

    // 2. Find all registry source directories.
    let src_dirs = cargo_registry_src_dirs();
    if src_dirs.is_empty() {
        debug!("Rust: no ~/.cargo/registry/src found; skipping Rust externals");
        return Vec::new();
    }

    // Collect all crate directories across every index-hash dir.
    let mut all_crate_dirs: Vec<PathBuf> = Vec::new();
    for src_dir in &src_dirs {
        if let Ok(entries) = std::fs::read_dir(src_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    all_crate_dirs.push(path);
                }
            }
        }
    }

    // 3. Match packages to cache directories.
    let mut roots = Vec::new();

    if use_fallback {
        // No lockfile — name-only prefix scan, picks lexicographically newest.
        for crate_name in &toml_names {
            let prefix = format!("{crate_name}-");
            let mut matches: Vec<PathBuf> = all_crate_dirs
                .iter()
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| {
                            s.starts_with(&prefix)
                                && s[prefix.len()..]
                                    .chars()
                                    .next()
                                    .map_or(false, |c| c.is_ascii_digit())
                        })
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            matches.sort();
            if let Some(best) = matches.pop() {
                let version = best
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_prefix(&prefix))
                    .unwrap_or("")
                    .to_string();
                roots.push(ExternalDepRoot {
                    module_path: crate_name.clone(),
                    version,
                    root: best,
                    ecosystem: "rust",
                    package_id: None,
                });
            }
        }
    } else {
        // Lockfile path — exact-version lookup via HashMap (O(packages) cost).
        let mut dir_index: std::collections::HashMap<(String, String), PathBuf> =
            std::collections::HashMap::with_capacity(all_crate_dirs.len());

        for path in &all_crate_dirs {
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some((name, version)) = split_crate_dir_name(dir_name) {
                dir_index.entry((name, version)).or_insert_with(|| path.clone());
            }
        }

        for entry in &packages {
            // Cargo sometimes normalises hyphens to underscores in dir names.
            let key = (entry.name.clone(), entry.version.clone());
            let under_key = (entry.name.replace('-', "_"), entry.version.clone());

            let found = dir_index
                .get(&key)
                .or_else(|| dir_index.get(&under_key))
                .cloned();

            if let Some(crate_root) = found {
                roots.push(ExternalDepRoot {
                    module_path: entry.name.clone(),
                    version: entry.version.clone(),
                    root: crate_root,
                    ecosystem: "rust",
                    package_id: None,
                });
            }
        }
    }

    debug!("Rust: discovered {} external crate roots", roots.len());
    roots
}

pub fn walk_rust_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let src = dep.root.join("src");
    let walk_root = if src.is_dir() { src } else { dep.root.clone() };
    walk_rust_dir(&walk_root, &dep.root, dep, &mut out);
    out
}

fn walk_rust_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_rust_dir_bounded(dir, root, dep, out, 0);
}

fn walk_rust_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "benches" | "examples" | "target")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_rust_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".rs") {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:rust:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "rust",
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
    fn parse_cargo_lock_registry_only() {
        let lock = concat!(
            "version = 3\n\n",
            "[[package]]\n",
            "name = \"anyhow\"\n",
            "version = \"1.0.82\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"abc\"\n\n",
            "[[package]]\n",
            "name = \"workspace-crate\"\n",
            "version = \"0.1.0\"\n\n",
            "[[package]]\n",
            "name = \"tokio\"\n",
            "version = \"1.38.0\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"def\"\n\n",
            "[[package]]\n",
            "name = \"git-dep\"\n",
            "version = \"0.5.0\"\n",
            "source = \"git+https://github.com/example/crate.git#abc\"\n",
        );
        let entries = parse_cargo_lock(lock);
        assert_eq!(entries.len(), 2, "should include only registry packages");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"anyhow"));
        assert!(names.contains(&"tokio"));
        assert!(!names.contains(&"workspace-crate"));
        assert!(!names.contains(&"git-dep"));
        let anyhow = entries.iter().find(|e| e.name == "anyhow").unwrap();
        assert_eq!(anyhow.version, "1.0.82");
    }

    #[test]
    fn split_crate_dir_name_handles_hyphenated_names() {
        assert_eq!(
            split_crate_dir_name("tokio-1.38.0"),
            Some(("tokio".into(), "1.38.0".into()))
        );
        assert_eq!(
            split_crate_dir_name("proc-macro2-1.0.91"),
            Some(("proc-macro2".into(), "1.0.91".into()))
        );
        assert_eq!(
            split_crate_dir_name("tokio-util-0.7.9"),
            Some(("tokio-util".into(), "0.7.9".into()))
        );
        assert_eq!(
            split_crate_dir_name("tracing-subscriber-0.3.18"),
            Some(("tracing-subscriber".into(), "0.3.18".into()))
        );
        assert_eq!(split_crate_dir_name("no-version"), None);
    }

    #[test]
    fn discover_rust_externals_uses_lockfile() {
        let tmp = std::env::temp_dir().join("bw-test-rust-lock");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let lock = concat!(
            "version = 3\n\n",
            "[[package]]\n",
            "name = \"serde\"\n",
            "version = \"1.0.200\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"abc\"\n",
        );
        std::fs::write(tmp.join("Cargo.lock"), lock).unwrap();

        let fake_home = tmp.join("fake_cargo_home");
        let serde_src = fake_home
            .join("registry")
            .join("src")
            .join("index-abc")
            .join("serde-1.0.200")
            .join("src");
        std::fs::create_dir_all(&serde_src).unwrap();
        std::fs::write(serde_src.join("lib.rs"), "pub trait Serialize {}").unwrap();

        std::env::set_var("CARGO_HOME", fake_home.to_str().unwrap());
        let roots = discover_rust_externals(&tmp);
        std::env::remove_var("CARGO_HOME");

        assert_eq!(roots.len(), 1, "should find serde from lockfile");
        assert_eq!(roots[0].module_path, "serde");
        assert_eq!(roots[0].version, "1.0.200");
        let walked = walk_rust_external_root(&roots[0]);
        assert_eq!(walked.len(), 1);
        assert!(walked[0].relative_path.starts_with("ext:rust:serde/"));
        assert!(walked[0].relative_path.ends_with("lib.rs"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_rust_externals_workspace_lockfile_at_parent() {
        let tmp = std::env::temp_dir().join("bw-test-rust-ws-lock");
        let _ = std::fs::remove_dir_all(&tmp);
        let crate_dir = tmp.join("crates").join("my-crate");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        let lock = concat!(
            "version = 3\n\n",
            "[[package]]\n",
            "name = \"anyhow\"\n",
            "version = \"1.0.82\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"abc\"\n",
        );
        std::fs::write(tmp.join("Cargo.lock"), lock).unwrap();

        let fake_home = tmp.join("fake_cargo_home");
        let anyhow_src = fake_home
            .join("registry")
            .join("src")
            .join("index-abc")
            .join("anyhow-1.0.82")
            .join("src");
        std::fs::create_dir_all(&anyhow_src).unwrap();
        std::fs::write(anyhow_src.join("lib.rs"), "pub struct Error;").unwrap();

        std::env::set_var("CARGO_HOME", fake_home.to_str().unwrap());
        let roots = discover_rust_externals(&crate_dir);
        std::env::remove_var("CARGO_HOME");

        assert_eq!(roots.len(), 1, "should find anyhow via parent Cargo.lock");
        assert_eq!(roots[0].module_path, "anyhow");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_rust_externals_empty_without_cargo_toml() {
        let tmp = std::env::temp_dir().join("bw-test-rust-no-toml");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let roots = discover_rust_externals(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
