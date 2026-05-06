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

use rayon::prelude::*;
use tracing::{debug, info, warn};
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
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

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        // R packages declare metadata in DESCRIPTION (case-sensitive,
        // capitalized).
        &[("DESCRIPTION", "r")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // R doesn't have a strong project-local cache convention.
        &[]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `DESCRIPTION` (or renv.lock). A bare directory
        // of `.R`/`.Rmd` files with no manifest can't be resolved against
        // external CRAN coordinates — the R resolver also needs DESCRIPTION
        // imports to classify `pkg::fn` refs as external. Dropping the
        // LanguagePresent shotgun is correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_r_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_r_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn resolve_import(
        &self, dep: &ExternalDepRoot, _p: &str, _s: &[&str],
    ) -> Vec<WalkedFile> { walk_r_narrowed(dep) }
    fn resolve_symbol(
        &self, dep: &ExternalDepRoot, _f: &str,
    ) -> Vec<WalkedFile> { walk_r_narrowed(dep) }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_r_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
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

    let user_uses: Vec<String> = collect_r_user_uses(project_root)
        .into_iter()
        .collect();

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
                    requested_imports: user_uses.clone(),
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

// R3 — `library(pkg)` / `requireNamespace("pkg")` / `pkg::func()` scanner.
// Module-granular: walk_r_narrowed only emits a dep's NAMESPACE when the
// project's R source actually references it. Skips deps that are declared
// in DESCRIPTION but never used.

fn collect_r_user_uses(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_r_uses(project_root, &mut out, 0);
    out
}

fn scan_r_uses(dir: &Path, out: &mut std::collections::HashSet<String>, depth: usize) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "renv" | "packrat" | "tests") || name.starts_with('.') { continue }
            }
            scan_r_uses(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".R") || name.ends_with(".r")
                || name.ends_with(".Rmd") || name.ends_with(".rmd"))
            { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_r_uses(&content, out);
        }
    }
}

fn extract_r_uses(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        // `library(pkg)` / `library("pkg")`
        if let Some(rest) = line.strip_prefix("library(") {
            let arg = rest.split(')').next().unwrap_or("").trim()
                .trim_matches(|c: char| c == '"' || c == '\'');
            if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
                out.insert(arg.to_string());
            }
        }
        if let Some(rest) = line.strip_prefix("require(") {
            let arg = rest.split(')').next().unwrap_or("").trim()
                .trim_matches(|c: char| c == '"' || c == '\'');
            if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
                out.insert(arg.to_string());
            }
        }
        // `pkg::func(...)` and `pkg:::func(...)`
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_alphabetic() {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_') {
                    i += 1;
                }
                if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':' {
                    let pkg = &line[start..i];
                    if !pkg.is_empty() {
                        out.insert(pkg.to_string());
                    }
                }
            } else {
                i += 1;
            }
        }
    }
}

fn walk_r_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() { return walk_r_root(dep); }
    if !dep.requested_imports.iter().any(|m| m == &dep.module_path) {
        return Vec::new();
    }
    walk_r_root(dep)
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
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

fn build_r_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_r_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_r_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();
    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Header-only tree-sitter scan of an R source file. R packages surface
/// their public API via top-level assignments `name <- function(...) {...}`
/// (or `=` / `->` variants) and via `setClass` / `setGeneric` / `setMethod`
/// S4 declarations. We record every top-level LHS identifier of an `<-`
/// assignment whose RHS is a `function`, `call`, or literal.
fn scan_r_header(source: &str) -> Vec<String> {
    let language = tree_sitter_r::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_r_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_r_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "left_assignment" | "equals_assignment" | "super_assignment"
        | "binary_operator" | "assignment" => {
            // LHS is an identifier / string / dollar; RHS is the value. We
            // only care about the LHS identifier.
            let lhs = node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("lhs"))
                .or_else(|| node.child_by_field_name("left"))
                .or_else(|| node.named_child(0));
            if let Some(lhs) = lhs {
                if matches!(lhs.kind(), "identifier" | "string") {
                    if let Ok(t) = lhs.utf8_text(bytes) {
                        out.push(t.trim_matches('"').to_string());
                    }
                }
            }
        }
        _ => {}
    }
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
            requested_imports: Vec::new(),
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

    #[test]
    fn r_extract_uses_handles_library_and_double_colon() {
        let mut out = std::collections::HashSet::new();
        extract_r_uses(
            "library(dplyr)\nrequire('tidyr')\nx <- ggplot2::ggplot(data)\ny <- stats:::internal(z)\n",
            &mut out,
        );
        assert!(out.contains("dplyr"));
        assert!(out.contains("tidyr"));
        assert!(out.contains("ggplot2"));
        assert!(out.contains("stats"));
    }

    #[test]
    fn r_narrowed_walk_skips_unused_packages() {
        let tmp = std::env::temp_dir().join("bw-test-cran-r3");
        let _ = std::fs::remove_dir_all(&tmp);
        let pkg_dir = tmp.join("dplyr");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("NAMESPACE"), "export(filter)\n").unwrap();

        let used = ExternalDepRoot {
            module_path: "dplyr".to_string(),
            version: "1.0".to_string(),
            root: pkg_dir.clone(),
            ecosystem: "r",
            package_id: None,
            requested_imports: vec!["dplyr".to_string()],
        };
        assert_eq!(walk_r_narrowed(&used).len(), 1);

        let unused = ExternalDepRoot {
            module_path: "dplyr".to_string(),
            version: "1.0".to_string(),
            root: pkg_dir.clone(),
            ecosystem: "r",
            package_id: None,
            requested_imports: vec!["other".to_string()],
        };
        assert!(walk_r_narrowed(&unused).is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
