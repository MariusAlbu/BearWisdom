// R / library path externals — Phase 1.3

use super::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// R library path → `discover_r_externals` + `walk_r_external_root`.
///
/// R is an unusual ecosystem: installed packages ship as **bytecode**
/// (`.rdb` / `.rdx`) rather than source, alongside an `R/NAMESPACE` file
/// listing the package's public API surface. We can't run the R extractor
/// against bytecode bodies, so the locator's walker targets the NAMESPACE
/// file instead and emits skeleton Function symbols for each exported name.
/// This gives the resolver enough external classification signal to match
/// tidyverse / CRAN package calls without needing source-level bodies.
///
/// Library paths searched (in order):
///   1. `renv/library/*/*/...`         (project-local renv snapshot)
///   2. `$R_LIBS_USER`                 (env override)
///   3. `~/R/x86_64-*-library/<r-ver>` (platform-default user library)
///   4. `~/R/win-library/<r-ver>`      (Windows default)
///   5. System install library         (last resort — varies per platform)
pub struct RExternalsLocator;

impl ExternalSourceLocator for RExternalsLocator {
    fn ecosystem(&self) -> &'static str { "r" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_r_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_r_external_root(dep)
    }
}

/// Discover external R package roots for a project.
///
/// Strategy:
///   1. Parse `DESCRIPTION` at the project root via the new description.rs
///      manifest reader, extracting Imports / Depends / LinkingTo /
///      Suggests package names.
///   2. Build the list of candidate library paths. Order matters — renv
///      project-local wins over user wins over system.
///   3. For each declared package, look for `<lib_path>/<package>/` on
///      disk. Return the first existing match per package.
///
/// Unlike Ruby/Elixir, the returned ExternalDepRoot points at the
/// installed package's top-level directory. The walker then targets the
/// NAMESPACE file inside each root, not a source tree.
pub fn discover_r_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::description::parse_description_deps;

    // Collect declared package names from the best available manifest.
    // Priority: renv.lock (authoritative for installed snapshot) > DESCRIPTION.
    let declared: Vec<String> = {
        let renv_lock = project_root.join("renv.lock");
        if renv_lock.is_file() {
            parse_renv_lock_packages(&renv_lock).unwrap_or_default()
        } else {
            let description_path = project_root.join("DESCRIPTION");
            if description_path.is_file() {
                std::fs::read_to_string(&description_path)
                    .ok()
                    .map(|c| parse_description_deps(&c))
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        }
    };
    if declared.is_empty() {
        return Vec::new();
    }

    let candidates = r_candidate_library_paths(project_root);
    if candidates.is_empty() {
        debug!(
            "No R library path found for {} — install packages via install.packages() or renv::restore()",
            project_root.display()
        );
        return Vec::new();
    }

    let mut result = Vec::with_capacity(declared.len());
    let mut seen = std::collections::HashSet::new();
    for pkg_name in &declared {
        if !seen.insert(pkg_name.clone()) {
            continue;
        }
        for lib_path in &candidates {
            let pkg_dir = lib_path.join(pkg_name);
            if pkg_dir.is_dir() {
                // Probe for DESCRIPTION inside to confirm it's a real
                // installed R package rather than a stale directory.
                if pkg_dir.join("DESCRIPTION").is_file() {
                    let version = read_r_package_version(&pkg_dir).unwrap_or_default();
                    result.push(ExternalDepRoot {
                        module_path: pkg_name.clone(),
                        version,
                        root: pkg_dir,
                        ecosystem: "r",
                        package_id: None,
                    });
                    break;
                }
            }
        }
    }
    result
}

/// Build the ordered list of R library directories that could contain
/// installed packages for this project.
fn r_candidate_library_paths(project_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 0. BEARWISDOM_R_LIBS environment override — highest priority.
    //    Semicolon-separated on Windows, colon-separated on Unix.
    //    Used for CI/test environments where R is not installed system-wide.
    if let Ok(override_libs) = std::env::var("BEARWISDOM_R_LIBS") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for entry in override_libs.split(sep) {
            let p = PathBuf::from(entry);
            if p.is_dir() {
                candidates.push(p);
            }
        }
        // When the override is set, skip all other discovery — the caller
        // explicitly told us where to look.
        if !candidates.is_empty() {
            return candidates;
        }
    }

    // 1. renv project-local library — `renv/library/<platform>/<r-ver>/`.
    //    renv nests two levels deep for platform / R version, but package
    //    directories live directly under the innermost level.
    let renv = project_root.join("renv").join("library");
    if renv.is_dir() {
        if let Ok(platform_entries) = std::fs::read_dir(&renv) {
            for platform in platform_entries.flatten() {
                let ppath = platform.path();
                if ppath.is_dir() {
                    if let Ok(version_entries) = std::fs::read_dir(&ppath) {
                        for ver in version_entries.flatten() {
                            let vpath = ver.path();
                            if vpath.is_dir() {
                                candidates.push(vpath);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. $R_LIBS_USER environment override. R honours a colon-separated
    //    (or semicolon on Windows) list.
    if let Ok(user_libs) = std::env::var("R_LIBS_USER") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for entry in user_libs.split(sep) {
            let p = PathBuf::from(entry);
            if p.is_dir() {
                candidates.push(p);
            }
        }
    }

    // 3. Platform-default user libraries.
    if let Some(home) = dirs::home_dir() {
        // Linux/macOS: ~/R/<platform>-library/<r-ver>/
        let r_dir = home.join("R");
        if r_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&r_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    // Either a `-library` suffix (linux/mac) or `win-library`
                    // (Windows) — walk its version subdirectories.
                    if p.is_dir()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.contains("library") || n.starts_with("win-"))
                            .unwrap_or(false)
                    {
                        if let Ok(sub) = std::fs::read_dir(&p) {
                            for ver in sub.flatten() {
                                let vpath = ver.path();
                                if vpath.is_dir() {
                                    candidates.push(vpath);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Windows default: ~/Documents/R/win-library/<r-ver>/
        let docs_r = home.join("Documents").join("R").join("win-library");
        if docs_r.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&docs_r) {
                for ver in sub.flatten() {
                    let vpath = ver.path();
                    if vpath.is_dir() {
                        candidates.push(vpath);
                    }
                }
            }
        }
    }

    // 4. System install library (best-effort; varies per platform).
    #[cfg(target_os = "linux")]
    {
        for p in ["/usr/lib/R/library", "/usr/local/lib/R/library", "/usr/lib/R/site-library"] {
            let path = PathBuf::from(p);
            if path.is_dir() {
                candidates.push(path);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for p in [
            "/Library/Frameworks/R.framework/Resources/library",
            "/opt/homebrew/lib/R/library",
        ] {
            let path = PathBuf::from(p);
            if path.is_dir() {
                candidates.push(path);
            }
        }
    }

    candidates
}

/// Read the `Version:` field from an installed R package's DESCRIPTION.
fn read_r_package_version(pkg_root: &Path) -> Option<String> {
    let description = pkg_root.join("DESCRIPTION");
    let content = std::fs::read_to_string(&description).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Version:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Walk an R package root and emit WalkedFile entries for the NAMESPACE
/// file. R packages ship their API surface as a plain-text NAMESPACE
/// containing `export()`, `exportPattern()`, `S3method()`, and similar
/// directives — the R extractor parses these and emits Function/Method
/// skeleton symbols that the resolver can match against.
///
/// We intentionally do NOT walk `R/*.rdb` / `R/*.rdx` — those are
/// bytecode compilation artefacts, not source. A later phase can add
/// an R-bytecode reader if full-body indexing becomes necessary.
pub fn walk_r_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let namespace_path = dep.root.join("NAMESPACE");
    if !namespace_path.is_file() {
        return Vec::new();
    }
    let virtual_path = format!("ext:r:{}/NAMESPACE", dep.module_path);
    vec![WalkedFile {
        relative_path: virtual_path,
        absolute_path: namespace_path,
        language: "r",
    }]
}

/// Parse package names from an `renv.lock` JSON file.
///
/// renv.lock structure:
/// ```json
/// {
///   "R": { "Version": "4.3.0", ... },
///   "Packages": {
///     "rlang": { "Package": "rlang", "Version": "1.2.0", ... },
///     ...
///   }
/// }
/// ```
/// Returns the set of package names listed under `"Packages"`.
pub fn parse_renv_lock_packages(renv_lock: &std::path::Path) -> Option<Vec<String>> {
    let content = std::fs::read_to_string(renv_lock).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let packages = val.get("Packages")?.as_object()?;
    let mut names: Vec<String> = packages.keys().cloned().collect();
    names.sort();
    Some(names)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// `r_candidate_library_paths` honours `BEARWISDOM_R_LIBS` when set.
    #[test]
    fn bearwisdom_r_libs_env_override_is_picked_up() {
        let tmp = TempDir::new().unwrap();
        let lib_dir = tmp.path().to_path_buf();

        let old = std::env::var("BEARWISDOM_R_LIBS").ok();
        std::env::set_var("BEARWISDOM_R_LIBS", lib_dir.to_str().unwrap());

        let candidates = r_candidate_library_paths(std::path::Path::new("/nonexistent"));

        match old {
            Some(v) => std::env::set_var("BEARWISDOM_R_LIBS", v),
            None => std::env::remove_var("BEARWISDOM_R_LIBS"),
        }

        assert!(
            candidates.contains(&lib_dir),
            "Expected BEARWISDOM_R_LIBS path in candidates; got {:?}",
            candidates
        );
    }

    /// `discover_r_externals` finds packages under a `BEARWISDOM_R_LIBS` cache.
    #[test]
    fn discovers_r_externals_from_lib_cache() {
        let tmp = TempDir::new().unwrap();

        // Fake project with DESCRIPTION declaring a dep on "mypkg"
        fs::write(
            tmp.path().join("DESCRIPTION"),
            "Package: testpkg\nVersion: 1.0\nImports:\n    mypkg\n",
        )
        .unwrap();

        // Fake R library with mypkg installed
        let lib_dir = tmp.path().join("r-lib");
        let pkg_dir = lib_dir.join("mypkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("DESCRIPTION"), "Package: mypkg\nVersion: 2.0\n").unwrap();
        fs::write(pkg_dir.join("NAMESPACE"), "export(hello)\nexport(world)\n").unwrap();

        let old = std::env::var("BEARWISDOM_R_LIBS").ok();
        std::env::set_var("BEARWISDOM_R_LIBS", lib_dir.to_str().unwrap());

        let roots = discover_r_externals(tmp.path());

        match old {
            Some(v) => std::env::set_var("BEARWISDOM_R_LIBS", v),
            None => std::env::remove_var("BEARWISDOM_R_LIBS"),
        }

        assert_eq!(roots.len(), 1, "expected 1 root; got {:?}", roots);
        assert_eq!(roots[0].module_path, "mypkg");
        assert_eq!(roots[0].version, "2.0");
    }

    /// `walk_r_external_root` emits the NAMESPACE file as a WalkedFile.
    #[test]
    fn walk_emits_namespace_as_walked_file() {
        let tmp = TempDir::new().unwrap();
        let pkg_dir = tmp.path().join("mypkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("NAMESPACE"), "export(foo)\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "mypkg".to_string(),
            version: "1.0".to_string(),
            root: pkg_dir,
            ecosystem: "r",
            package_id: None,
        };

        let walked = walk_r_external_root(&dep);
        assert_eq!(walked.len(), 1);
        assert_eq!(walked[0].relative_path, "ext:r:mypkg/NAMESPACE");
        assert_eq!(walked[0].language, "r");
    }

    /// `parse_renv_lock_packages` extracts package names from renv.lock JSON.
    #[test]
    fn parse_renv_lock_extracts_package_names() {
        let tmp = TempDir::new().unwrap();
        let lock = tmp.path().join("renv.lock");
        fs::write(
            &lock,
            r#"{"R":{"Version":"4.3.0"},"Packages":{"rlang":{"Package":"rlang","Version":"1.2.0"},"vctrs":{"Package":"vctrs","Version":"0.6.5"}}}"#,
        )
        .unwrap();

        let names = parse_renv_lock_packages(&lock).unwrap();
        assert!(names.contains(&"rlang".to_string()));
        assert!(names.contains(&"vctrs".to_string()));
        assert_eq!(names.len(), 2);
    }
}
