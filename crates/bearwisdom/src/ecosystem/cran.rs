// =============================================================================
// ecosystem/cran.rs — CRAN ecosystem (R)
//
// Phase 2 + 3: consolidates `indexer/externals/r_lang.rs` +
// `indexer/manifest/description.rs`. R package install paths are searched
// in a rich priority order (renv → R_LIBS_USER → XDG → Windows registry
// → system install).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{debug, info, warn};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::indexer::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::indexer::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cran");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["r"];
const LEGACY_ECOSYSTEM_TAG: &str = "r";

pub struct CranEcosystem;

impl Ecosystem for CranEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("r"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_r_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_r_root(dep)
    }
}

impl ExternalSourceLocator for CranEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_r_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_r_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CranEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CranEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (DESCRIPTION file)
// ===========================================================================

pub struct DescriptionManifest;

impl ManifestReader for DescriptionManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Description }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let description_path = project_root.join("DESCRIPTION");
        if !description_path.is_file() { return None }
        let content = std::fs::read_to_string(&description_path).ok()?;
        let mut data = ManifestData::default();
        for name in parse_description_runtime_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_description_runtime_deps(content: &str) -> Vec<String> {
    parse_description_fields(content, &["Depends", "Imports"])
}

pub fn parse_description_deps(content: &str) -> Vec<String> {
    parse_description_fields(content, &["Depends", "Imports", "LinkingTo", "Suggests"])
}

fn parse_description_fields(content: &str, field_names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for field in field_names {
        if let Some(value) = read_field(content, field) {
            for pkg in split_package_list(&value) {
                if pkg == "R" { continue }
                if seen.insert(pkg.clone()) { out.push(pkg) }
            }
        }
    }
    out
}

fn read_field(content: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}:");
    let mut iter = content.lines().peekable();
    while let Some(line) = iter.next() {
        if let Some(rest) = line.strip_prefix(&prefix) {
            let mut value = rest.trim().to_string();
            while let Some(next) = iter.peek() {
                if next.starts_with(' ') || next.starts_with('\t') {
                    value.push(' ');
                    value.push_str(next.trim());
                    iter.next();
                } else {
                    break;
                }
            }
            return Some(value);
        }
    }
    None
}

fn split_package_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| {
            let trimmed = item.trim();
            let name = match trimmed.find('(') {
                Some(idx) => trimmed[..idx].trim(),
                None => trimmed,
            };
            name.to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

// ===========================================================================
// Discovery + walk
// ===========================================================================

pub fn discover_r_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
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
    if declared.is_empty() { return Vec::new() }

    let candidates = r_candidate_library_paths(project_root);
    if candidates.is_empty() {
        warn!(project = %project_root.display(),
            "R: no library paths found; set BEARWISDOM_R_LIBS");
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
        if !seen.insert(pkg_name.clone()) { continue }
        let mut found = false;
        for lib_path in &candidates {
            let pkg_dir = lib_path.join(pkg_name);
            if pkg_dir.is_dir() && pkg_dir.join("DESCRIPTION").is_file() {
                let version = read_r_package_version(&pkg_dir).unwrap_or_default();
                result.push(ExternalDepRoot {
                    module_path: pkg_name.clone(),
                    version,
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                found = true;
                break;
            }
        }
        if !found { not_found.push(pkg_name.as_str()) }
    }

    info!(
        project = %project_root.display(),
        found = result.len(),
        declared = declared.len(),
        "R: package discovery complete"
    );
    result
}

fn r_candidate_library_paths(project_root: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(override_libs) = std::env::var("BEARWISDOM_R_LIBS") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for entry in override_libs.split(sep) {
            let p = PathBuf::from(entry);
            if p.is_dir() { candidates.push(p) }
        }
        if !candidates.is_empty() { return candidates }
    }

    let renv = project_root.join("renv").join("library");
    if renv.is_dir() {
        if let Ok(platform_entries) = std::fs::read_dir(&renv) {
            for platform in platform_entries.flatten() {
                let ppath = platform.path();
                if ppath.is_dir() {
                    if let Ok(version_entries) = std::fs::read_dir(&ppath) {
                        for ver in version_entries.flatten() {
                            let vpath = ver.path();
                            if vpath.is_dir() { candidates.push(vpath) }
                        }
                    }
                }
            }
        }
    }

    if let Ok(user_libs) = std::env::var("R_LIBS_USER") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for entry in user_libs.split(sep) {
            let p = PathBuf::from(entry);
            if p.is_dir() { candidates.push(p) }
        }
    }

    if let Some(home) = dirs::home_dir() {
        if let Some(local_app_data) = dirs::data_local_dir() {
            push_r_version_subdirs(&local_app_data.join("R").join("win-library"), &mut candidates);
        }
        push_r_version_subdirs(&home.join("R").join("win-library"), &mut candidates);
        push_r_version_subdirs(&home.join("Documents").join("R").join("win-library"), &mut candidates);

        let r_dir = home.join("R");
        if r_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&r_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if p.is_dir() && name.ends_with("-library") && !name.starts_with("win") {
                        push_r_version_subdirs(&p, &mut candidates);
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Some(r_home) = read_r_home_from_registry() {
            let lib = PathBuf::from(&r_home).join("library");
            if lib.is_dir() { candidates.push(lib) }
        }
        for root in ["C:/Program Files/R", "C:/Program Files (x86)/R"] {
            let base = PathBuf::from(root);
            if base.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&base) {
                    for entry in entries.flatten() {
                        let lib = entry.path().join("library");
                        if lib.is_dir() { candidates.push(lib) }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        for p in ["/usr/lib/R/library", "/usr/local/lib/R/library", "/usr/lib/R/site-library"] {
            let path = PathBuf::from(p);
            if path.is_dir() { candidates.push(path) }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for p in [
            "/Library/Frameworks/R.framework/Resources/library",
            "/opt/homebrew/lib/R/library",
            "/opt/local/lib/R/library",
        ] {
            let path = PathBuf::from(p);
            if path.is_dir() { candidates.push(path) }
        }
    }

    let mut seen_set = std::collections::HashSet::new();
    candidates.retain(|p| seen_set.insert(p.clone()));
    candidates
}

fn push_r_version_subdirs(parent: &Path, out: &mut Vec<PathBuf>) {
    if !parent.is_dir() { return }
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let vpath = entry.path();
            if vpath.is_dir() { out.push(vpath) }
        }
    }
}

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
            "query", r"HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\R-core\R",
            "/v", "InstallPath",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success());
    if let Some(output) = result32 {
        return parse_reg_install_path(&String::from_utf8_lossy(&output.stdout));
    }
    None
}

#[cfg(windows)]
fn parse_reg_install_path(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("InstallPath") {
            let parts: Vec<&str> = line.splitn(3, "REG_SZ").collect();
            if let Some(value) = parts.get(1) {
                let path = value.trim().to_string();
                if !path.is_empty() { return Some(path) }
            }
        }
    }
    None
}

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

fn walk_r_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let namespace_path = dep.root.join("NAMESPACE");
    if !namespace_path.is_file() { return Vec::new() }
    let virtual_path = format!("ext:r:{}/NAMESPACE", dep.module_path);
    vec![WalkedFile {
        relative_path: virtual_path,
        absolute_path: namespace_path,
        language: "r",
    }]
}

pub fn parse_renv_lock_packages(renv_lock: &Path) -> Option<Vec<String>> {
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

    #[test]
    fn ecosystem_identity() {
        let c = CranEcosystem;
        assert_eq!(c.id(), ID);
        assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&c), &["r"]);
    }

    #[test]
    fn legacy_locator_tag_is_r() {
        assert_eq!(ExternalSourceLocator::ecosystem(&CranEcosystem), "r");
    }

    #[test]
    fn parses_description_fields() {
        let sample = "Imports:\n    cli (>= 3.6.2),\n    rlang\nSuggests:\n    ggplot2\n";
        let deps = parse_description_deps(sample);
        assert!(deps.contains(&"cli".to_string()));
        assert!(deps.contains(&"rlang".to_string()));
        assert!(deps.contains(&"ggplot2".to_string()));
    }

    #[test]
    fn strips_r_version_pin() {
        let deps = parse_description_deps("Depends:\n    R (>= 4.1.0),\n    methods\n");
        assert!(!deps.contains(&"R".to_string()));
        assert!(deps.contains(&"methods".to_string()));
    }

    #[test]
    fn runtime_deps_excludes_suggests() {
        let deps = parse_description_runtime_deps("Imports:\n    rlang\nSuggests:\n    ggplot2\n");
        assert!(deps.contains(&"rlang".to_string()));
        assert!(!deps.contains(&"ggplot2".to_string()));
    }

    #[test]
    fn walk_emits_namespace_as_walked_file() {
        let tmp = std::env::temp_dir().join("bw-test-cran-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        let pkg_dir = tmp.join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("NAMESPACE"), "export(foo)\n").unwrap();
        let dep = ExternalDepRoot {
            module_path: "mypkg".to_string(),
            version: "1.0".to_string(),
            root: pkg_dir,
            ecosystem: "r",
            package_id: None,
        };
        let walked = walk_r_root(&dep);
        assert_eq!(walked.len(), 1);
        assert_eq!(walked[0].relative_path, "ext:r:mypkg/NAMESPACE");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_renv_lock_extracts_package_names() {
        let tmp = std::env::temp_dir().join("bw-test-cran-renv");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let lock = tmp.join("renv.lock");
        std::fs::write(
            &lock,
            r#"{"Packages":{"rlang":{"Package":"rlang"},"vctrs":{"Package":"vctrs"}}}"#,
        ).unwrap();
        let names = parse_renv_lock_packages(&lock).unwrap();
        assert!(names.contains(&"rlang".to_string()));
        assert!(names.contains(&"vctrs".to_string()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
