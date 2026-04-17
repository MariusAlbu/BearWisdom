// =============================================================================
// ecosystem/cpan.rs — CPAN ecosystem (Perl)
//
// Phase 2 + 3: consolidates `indexer/externals/perl.rs`.
// No separate manifest reader — cpanfile parsing lives here.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cpan");
const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["perl"];
const LEGACY_ECOSYSTEM_TAG: &str = "perl";

pub struct CpanEcosystem;

impl Ecosystem for CpanEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("perl"),
        ])
    }
    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_perl_externals(ctx.project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_perl_root(dep) }
}

impl ExternalSourceLocator for CpanEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_perl_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_perl_root(dep) }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CpanEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CpanEcosystem)).clone()
}

pub fn discover_perl_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let cpanfile = project_root.join("cpanfile");
    if !cpanfile.is_file() { return Vec::new() }
    let Ok(content) = std::fs::read_to_string(&cpanfile) else { return Vec::new() };
    let declared = parse_cpanfile_requires(&content);
    if declared.is_empty() { return Vec::new() }

    let lib_dirs = perl_lib_dirs(project_root);
    if lib_dirs.is_empty() { return Vec::new() }

    let mut roots = Vec::new();
    for module_name in &declared {
        let path_fragment = module_name.replace("::", std::path::MAIN_SEPARATOR_STR);
        for lib in &lib_dirs {
            let module_dir = lib.join(&path_fragment);
            if module_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: module_name.clone(),
                    version: String::new(),
                    root: module_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                break;
            }
            let module_file = lib.join(format!("{path_fragment}.pm"));
            if module_file.is_file() {
                roots.push(ExternalDepRoot {
                    module_path: module_name.clone(),
                    version: String::new(),
                    root: module_file.parent().unwrap_or(lib).to_path_buf(),
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                break;
            }
        }
    }
    debug!("Perl: {} external module roots", roots.len());
    roots
}

pub fn parse_cpanfile_requires(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') { continue }
        if trimmed.starts_with("requires") {
            let rest = trimmed["requires".len()..].trim();
            let name = rest.trim_start_matches(|c: char| c == '\'' || c == '"' || c.is_whitespace());
            if let Some(end) = name.find(|c: char| c == '\'' || c == '"' || c == ',' || c == ';') {
                let module = &name[..end];
                if !module.is_empty() && module != "perl" {
                    if !deps.contains(&module.to_string()) { deps.push(module.to_string()) }
                }
            }
        }
    }
    deps
}

fn perl_lib_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local = project_root.join("local").join("lib").join("perl5");
    if local.is_dir() { dirs.push(local) }
    for var in &["PERL5LIB", "PERL_LOCAL_LIB_ROOT"] {
        if let Ok(val) = std::env::var(var) {
            for p in val.split(if cfg!(windows) { ';' } else { ':' }) {
                let pb = PathBuf::from(p);
                if pb.is_dir() { dirs.push(pb) }
            }
        }
    }
    dirs
}

fn walk_perl_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "t" | "xt" | "blib" | "examples") || name.starts_with('.') { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".pm") || name.ends_with(".pl")) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:perl:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "perl",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        assert_eq!(CpanEcosystem.id(), ID);
        assert_eq!(Ecosystem::languages(&CpanEcosystem), &["perl"]);
    }

    #[test]
    fn perl_parses_cpanfile() {
        let content = r#"
requires 'perl', 5.014000;
requires 'Carp';
requires 'Clone';
requires 'Config::Any';
requires 'Data::Censor' => '0.04';
"#;
        let deps = parse_cpanfile_requires(content);
        assert_eq!(deps, vec!["Carp", "Clone", "Config::Any", "Data::Censor"]);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
