// =============================================================================
// ecosystem/opam.rs — opam ecosystem (OCaml)
//
// Phase 2 + 3: consolidates `indexer/externals/ocaml.rs` +
// `indexer/manifest/opam.rs`.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("opam");
const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["ocaml"];
const LEGACY_ECOSYSTEM_TAG: &str = "ocaml";

pub struct OpamEcosystem;

impl Ecosystem for OpamEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("ocaml"),
        ])
    }
    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ocaml_externals(ctx.project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_ocaml_root(dep) }
}

impl ExternalSourceLocator for OpamEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ocaml_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_ocaml_root(dep) }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<OpamEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(OpamEcosystem)).clone()
}

// ===========================================================================
// Manifest reader
// ===========================================================================

pub struct OpamManifest;

impl ManifestReader for OpamManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Opam }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let Ok(entries) = std::fs::read_dir(project_root) else { return None };
        let opam_file = entries.flatten().find(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("opam")
        })?;
        let content = std::fs::read_to_string(opam_file.path()).ok()?;
        let mut data = ManifestData::default();
        for name in parse_opam_depends(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_opam_depends(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("depends:") else { return deps };
    let rest = &content[start + "depends:".len()..];
    let Some(bracket_start) = rest.find('[') else { return deps };
    let rest = &rest[bracket_start + 1..];
    let Some(bracket_end) = rest.find(']') else { return deps };
    let block = &rest[..bracket_end];

    for line in block.lines() {
        let trimmed = line.trim().trim_start_matches('"');
        if trimmed.is_empty() { continue }
        let name = trimmed.split(|c: char| c == '"' || c == ' ' || c == '{')
            .next().unwrap_or("").trim();
        if !name.is_empty()
            && name != "ocaml"
            && !name.starts_with("conf-")
            && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            if !deps.contains(&name.to_string()) { deps.push(name.to_string()) }
        }
    }
    deps
}

// ===========================================================================
// Discovery + walk
// ===========================================================================

pub fn discover_ocaml_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new() };
    let opam_file = entries.flatten().find(|e| {
        e.path().extension().and_then(|x| x.to_str()) == Some("opam")
    });
    let Some(opam_entry) = opam_file else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(opam_entry.path()) else { return Vec::new() };
    let declared = parse_opam_depends(&content);
    if declared.is_empty() { return Vec::new() }

    let lib_dirs = ocaml_lib_dirs(project_root);
    let mut roots = Vec::new();
    for dep in &declared {
        for lib in &lib_dirs {
            let pkg_dir = lib.join(dep);
            if pkg_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::new(),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
                break;
            }
        }
    }
    debug!("OCaml: {} external package roots", roots.len());
    roots
}

fn ocaml_lib_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local_opam = project_root.join("_opam").join("lib");
    if local_opam.is_dir() { dirs.push(local_opam) }
    if let Ok(switch) = std::env::var("OPAM_SWITCH_PREFIX") {
        let lib = PathBuf::from(switch).join("lib");
        if lib.is_dir() { dirs.push(lib) }
    }
    if let Some(home) = dirs::home_dir() {
        let opam = home.join(".opam");
        if opam.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&opam) {
                for e in entries.flatten() {
                    let lib = e.path().join("lib");
                    if lib.is_dir() { dirs.push(lib) }
                }
            }
        }
    }
    dirs
}

fn walk_ocaml_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
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
                if matches!(name, "test" | "tests" | "bench") || name.starts_with('.') { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".ml") || name.ends_with(".mli")) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:ocaml:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "ocaml",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        assert_eq!(OpamEcosystem.id(), ID);
        assert_eq!(Ecosystem::languages(&OpamEcosystem), &["ocaml"]);
    }

    #[test]
    fn parse_opam_deps() {
        let content = r#"
depends: [
  "dune" {>= "2.8.0"}
  "ocaml" {>= "4.08.1"}
  "conf-libpcre"
  "cohttp-lwt-unix"
  "core"
  "lwt"
]
"#;
        let deps = parse_opam_depends(content);
        assert!(deps.contains(&"cohttp-lwt-unix".to_string()));
        assert!(deps.contains(&"core".to_string()));
        assert!(deps.contains(&"lwt".to_string()));
        assert!(!deps.contains(&"ocaml".to_string()));
        assert!(!deps.contains(&"conf-libpcre".to_string()));
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
