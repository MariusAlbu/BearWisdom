// Haskell / cabal externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Haskell cabal → `discover_haskell_externals` + walk.
///
/// Cabal packages are installed to `~/.cabal/store/ghc-<ver>/` or
/// `~/.local/state/cabal/store/`. Stack uses `.stack-work/install/`.
/// Declared deps come from `build-depends:` in `*.cabal` files.
/// Walk: `*.hs` files from exposed-modules directories.
pub struct HaskellExternalsLocator;

impl ExternalSourceLocator for HaskellExternalsLocator {
    fn ecosystem(&self) -> &'static str { "haskell" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_haskell_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_haskell_external_root(dep)
    }
}

pub fn discover_haskell_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = parse_cabal_build_depends(project_root);
    if declared.is_empty() {
        return Vec::new();
    }

    // Stack projects: .stack-work/install/<platform>/<hash>/lib/<ghc-ver>/
    let stack_root = project_root.join(".stack-work");
    if stack_root.is_dir() {
        let roots = find_haskell_stack_deps(&stack_root, &declared);
        if !roots.is_empty() {
            debug!("Haskell: discovered {} external roots via Stack", roots.len());
            return roots;
        }
    }

    // Cabal: ~/.cabal/store/ghc-<ver>/ or ~/.local/state/cabal/store/ghc-<ver>/
    let roots = find_haskell_cabal_deps(&declared);
    debug!("Haskell: discovered {} external roots via Cabal", roots.len());
    roots
}

fn parse_cabal_build_depends(project_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project_root) else {
        return Vec::new();
    };
    let cabal_file = entries.flatten().find(|e| {
        e.path().extension().and_then(|x| x.to_str()) == Some("cabal")
    });
    let Some(cabal_entry) = cabal_file else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(cabal_entry.path()) else {
        return Vec::new();
    };

    let mut deps = Vec::new();
    let mut in_build_depends = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("build-depends:") {
            in_build_depends = true;
            let rest = trimmed["build-depends:".len()..].trim();
            if !rest.is_empty() {
                deps.extend(parse_cabal_dep_list(rest));
            }
            continue;
        }
        if in_build_depends {
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.starts_with(',') {
                in_build_depends = false;
                continue;
            }
            deps.extend(parse_cabal_dep_list(trimmed));
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

fn parse_cabal_dep_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|chunk| {
            chunk.trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .filter(|name| !name.is_empty() && name != "base")
        .collect()
}

fn find_haskell_stack_deps(stack_work: &Path, declared: &[String]) -> Vec<ExternalDepRoot> {
    // .stack-work/install/<platform>/<hash>/<ghc-ver>/lib/<ghc-ver>/<pkg>-<ver>/
    let install = stack_work.join("install");
    if !install.is_dir() { return Vec::new(); }

    let mut roots = Vec::new();
    let Ok(platforms) = std::fs::read_dir(&install) else { return Vec::new(); };
    for platform in platforms.flatten() {
        let Ok(hashes) = std::fs::read_dir(platform.path()) else { continue; };
        for hash in hashes.flatten() {
            let lib = hash.path().join("lib");
            if !lib.is_dir() { continue; }
            let Ok(ghc_vers) = std::fs::read_dir(&lib) else { continue; };
            for ghc_ver in ghc_vers.flatten() {
                find_haskell_pkgs_in_dir(&ghc_ver.path(), declared, &mut roots);
            }
        }
    }
    roots
}

fn find_haskell_cabal_deps(declared: &[String]) -> Vec<ExternalDepRoot> {
    let mut candidates = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let store1 = home.join(".cabal").join("store");
        let store2 = home.join(".local").join("state").join("cabal").join("store");
        for store in [store1, store2] {
            if store.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&store) {
                    for e in entries.flatten() {
                        if e.path().is_dir() {
                            candidates.push(e.path());
                        }
                    }
                }
            }
        }
    }

    let mut roots = Vec::new();
    for ghc_dir in &candidates {
        find_haskell_pkgs_in_dir(ghc_dir, declared, &mut roots);
    }
    roots
}

fn find_haskell_pkgs_in_dir(dir: &Path, declared: &[String], roots: &mut Vec<ExternalDepRoot>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        for dep in declared {
            let prefix = format!("{dep}-");
            if name_str.starts_with(&prefix) && entry.path().is_dir() {
                let version = name_str[prefix.len()..].to_string();
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version,
                    root: entry.path(),
                    ecosystem: "haskell",
                    package_id: None,
                });
                break;
            }
        }
    }
}

pub fn walk_haskell_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_haskell_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_haskell_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_haskell_dir_bounded(dir, root, dep, out, 0);
}

fn walk_haskell_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "bench" | "dist-newstyle" | ".stack-work")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_haskell_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".hs") { continue; }
            if name.ends_with("Spec.hs") || name.ends_with("Test.hs") { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:haskell:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "haskell",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haskell_parses_cabal_build_depends() {
        let tmp = std::env::temp_dir().join("bw-test-haskell-cabal");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("test.cabal"), r#"
cabal-version: 2.0
name: test
version: 1.0
library
  build-depends:
    aeson >= 2.0,
    text,
    bytestring
"#).unwrap();
        let deps = parse_cabal_build_depends(&tmp);
        assert_eq!(deps, vec!["aeson", "bytestring", "text"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
