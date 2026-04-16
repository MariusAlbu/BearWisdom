// R / library path externals -- Phase 1.3

use super::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// R library path -> `discover_r_externals` + `walk_r_external_root`.
///
/// Library paths searched (in order):
///   0. `BEARWISDOM_R_LIBS`                        (explicit override -- highest priority)
///   1. `renv/library/*/*/...`                     (project-local renv snapshot)
///   2. `$R_LIBS_USER`                             (R env var)
///   3. `%LOCALAPPDATA%/R/win-library/<r-ver>/`    (Windows default since R 4.0)
///   4. `~/R/win-library/<r-ver>/`                 (older Windows / some R versions)
///   5. `~/Documents/R/win-library/<r-ver>/`       (Windows alternate)
///   6. `~/R/<platform>-library/<r-ver>/`          (Linux/macOS user lib)
///   7. Windows registry `HKLM\SOFTWARE\R-core\R`  (system R_HOME -> library/)
///   8. `C:/Program Files/R/R-*/library/`          (Windows system install glob)
///   9. Platform system paths                      (Linux/macOS system install)
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
pub fn discover_r_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::description::parse_description_deps;

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
        warn!(
            project = %project_root.display(),
            "R: no library paths found -- R may not be installed; set BEARWISDOM_R_LIBS to the library directory"
        );
        return Vec::new();
    }

    info!(
        project = %project_root.display(),
        paths = ?candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "R: using library search paths"
    );

    let mut result = Vec::with_capacity(declared.len());
    let mut seen = std::collections::HashSet::new();
    let mut not_found: Vec<&str> = Vec::new();

    for pkg_name in &declared {
        if !seen.insert(pkg_name.clone()) {
            continue;
        }
        let mut found = false;
        for lib_path in &candidates {
            let pkg_dir = lib_path.join(pkg_name);
            if pkg_dir.is_dir() && pkg_dir.join("DESCRIPTION").is_file() {
                let version = read_r_package_version(&pkg_dir).unwrap_or_default();
                debug!(
                    pkg = %pkg_name,
                    version = %version,
                    path = %pkg_dir.display(),
                    "R: package found"
                );
                result.push(ExternalDepRoot {
                    module_path: pkg_name.clone(),
                    version,
                    root: pkg_dir,
                    ecosystem: "r",
                    package_id: None,
                });
                found = true;
                break;
            }
        }
        if !found {
            not_found.push(pkg_name.as_str());
        }
    }

    info!(
        project = %project_root.display(),
        found = result.len(),
        declared = declared.len(),
        not_found = not_found.len(),
        missing = ?not_found,
        "R: package discovery complete"
    );

    result
}

/// Build the ordered list of R library directories that could contain
/// installed packages for this project.
fn r_candidate_library_paths(project_root: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 0. BEARWISDOM_R_LIBS environment override -- highest priority.
    if let Ok(override_libs) = std::env::var("BEARWISDOM_R_LIBS") {
        debug!("R: probing BEARWISDOM_R_LIBS override: {:?}", override_libs);
        let sep = if cfg!(windows) { ';' } else { ':' };
        for entry in override_libs.split(sep) {
            let p = PathBuf::from(entry);
            if p.is_dir() {
                debug!("R: BEARWISDOM_R_LIBS path accepted: {}", p.display());
                candidates.push(p);
            } else {
                debug!("R: BEARWISDOM_R_LIBS path not found: {}", p.display());
            }
        }
        if !candidates.is_empty() {
            return candidates;
        }
        warn!("R: BEARWISDOM_R_LIBS was set but no valid paths found -- falling through to auto-discovery");
    }

    // 1. renv project-local library.
    let renv = project_root.join("renv").join("library");
    debug!("R: probing renv library: {}", renv.display());
    if renv.is_dir() {
        if let Ok(platform_entries) = std::fs::read_dir(&renv) {
            for platform in platform_entries.flatten() {
                let ppath = platform.path();
                if ppath.is_dir() {
                    if let Ok(version_entries) = std::fs::read_dir(&ppath) {
                        for ver in version_entries.flatten() {
                            let vpath = ver.path();
                            if vpath.is_dir() {
                                debug!("R: renv library candidate: {}", vpath.display());
                                candidates.push(vpath);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. R_LIBS_USER env var.
    if let Ok(user_libs) = std::env::var("R_LIBS_USER") {
        debug!("R: probing R_LIBS_USER: {:?}", user_libs);
        let sep = if cfg!(windows) { ';' } else { ':' };
        for entry in user_libs.split(sep) {
            let p = PathBuf::from(entry);
            if p.is_dir() {
                debug!("R: R_LIBS_USER path accepted: {}", p.display());
                candidates.push(p);
            } else {
                debug!("R: R_LIBS_USER path not found: {}", p.display());
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        // 3. Windows primary: %LOCALAPPDATA%/R/win-library/<r-ver>/
        //    This is the default since R 4.0 when no R_LIBS_USER is set.
        if let Some(local_app_data) = dirs::data_local_dir() {
            let winlib = local_app_data.join("R").join("win-library");
            debug!("R: probing %LOCALAPPDATA%/R/win-library: {}", winlib.display());
            push_r_version_subdirs(&winlib, &mut candidates);
        }

        // 4. Windows older default: ~/R/win-library/<r-ver>/
        let home_r_winlib = home.join("R").join("win-library");
        debug!("R: probing ~/R/win-library: {}", home_r_winlib.display());
        push_r_version_subdirs(&home_r_winlib, &mut candidates);

        // 5. Windows alternate: ~/Documents/R/win-library/<r-ver>/
        let docs_r = home.join("Documents").join("R").join("win-library");
        debug!("R: probing ~/Documents/R/win-library: {}", docs_r.display());
        push_r_version_subdirs(&docs_r, &mut candidates);

        // 6. Linux/macOS: ~/R/<platform>-library/<r-ver>/
        let r_dir = home.join("R");
        if r_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&r_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if p.is_dir() && name.ends_with("-library") && !name.starts_with("win") {
                        debug!("R: probing ~/R/{}: {}", name, p.display());
                        push_r_version_subdirs(&p, &mut candidates);
                    }
                }
            }
        }
    }

    // 7. Windows registry: HKLM\SOFTWARE\R-core\R InstallPath -> library/
    #[cfg(windows)]
    {
        if let Some(r_home) = read_r_home_from_registry() {
            let lib = PathBuf::from(&r_home).join("library");
            debug!("R: registry R_HOME library: {}", lib.display());
            if lib.is_dir() {
                candidates.push(lib);
            }
        }
    }

    // 8. Windows: glob C:/Program Files/R/R-<ver>/library/
    #[cfg(windows)]
    {
        for root in ["C:/Program Files/R", "C:/Program Files (x86)/R"] {
            let base = PathBuf::from(root);
            debug!("R: probing program files glob: {}", base.display());
            if base.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&base) {
                    for entry in entries.flatten() {
                        let lib = entry.path().join("library");
                        if lib.is_dir() {
                            debug!("R: program files library found: {}", lib.display());
                            candidates.push(lib);
                        }
                    }
                }
            }
        }
    }

    // 9. System install library (Linux/macOS).
    #[cfg(target_os = "linux")]
    {
        for p in ["/usr/lib/R/library", "/usr/local/lib/R/library", "/usr/lib/R/site-library"] {
            debug!("R: probing system path: {p}");
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
            "/opt/local/lib/R/library",
        ] {
            debug!("R: probing system path: {p}");
            let path = PathBuf::from(p);
            if path.is_dir() {
                candidates.push(path);
            }
        }
    }

    // Deduplicate while preserving priority order.
    let mut seen_set = std::collections::HashSet::new();
    candidates.retain(|p| seen_set.insert(p.clone()));

    candidates
}

/// Walk a win-library-style parent and push all R version subdirectories.
fn push_r_version_subdirs(parent: &Path, out: &mut Vec<PathBuf>) {
    if !parent.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let vpath = entry.path();
            if vpath.is_dir() {
                debug!("R: version subdir candidate: {}", vpath.display());
                out.push(vpath);
            }
        }
    }
}

/// Read InstallPath from HKLM\SOFTWARE\R-core\R via reg query.
#[cfg(windows)]
fn read_r_home_from_registry() -> Option<String> {
    use std::process::Command;

    let result = Command::new("reg")
        .args(["query", r"HKEY_LOCAL_MACHINE\SOFTWARE\R-core\R", "/v", "InstallPath"])
        .output()
        .ok()
        .filter(|o| o.status.success());

    if let Some(output) = result {
        return parse_reg_install_path(&String::from_utf8_lossy(&output.stdout));
    }

    let result32 = Command::new("reg")
        .args([
            "query",
            r"HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\R-core\R",
            "/v",
            "InstallPath",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success());

    if let Some(output) = result32 {
        return parse_reg_install_path(&String::from_utf8_lossy(&output.stdout));
    }

    debug!(r"R: registry key not found");
    None
}

/// Parse InstallPath value from reg query stdout output.
#[cfg(windows)]
fn parse_reg_install_path(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("InstallPath") {
            let parts: Vec<&str> = line.splitn(3, "REG_SZ").collect();
            if let Some(value) = parts.get(1) {
                let path = value.trim().to_string();
                if !path.is_empty() {
                    debug!("R: registry InstallPath = {path}");
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Read the Version field from an installed R package DESCRIPTION file.
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

/// Walk an R package root and emit a WalkedFile for the NAMESPACE file.
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

/// Parse package names from an renv.lock JSON file.
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

    #[test]
    fn discovers_r_externals_from_lib_cache() {
        let tmp = TempDir::new().unwrap();

        fs::write(
            tmp.path().join("DESCRIPTION"),
            "Package: testpkg\nVersion: 1.0\nImports:\n    mypkg\n",
        )
        .unwrap();

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

    /// r_candidate_library_paths must not panic when R is not installed.
    #[test]
    fn no_library_returns_gracefully() {
        let old = std::env::var("BEARWISDOM_R_LIBS").ok();
        std::env::remove_var("BEARWISDOM_R_LIBS");

        let nonexistent = PathBuf::from("/nonexistent/path/that/does/not/exist");
        let _candidates = r_candidate_library_paths(&nonexistent);

        match old {
            Some(v) => std::env::set_var("BEARWISDOM_R_LIBS", v),
            None => std::env::remove_var("BEARWISDOM_R_LIBS"),
        }
    }
}
