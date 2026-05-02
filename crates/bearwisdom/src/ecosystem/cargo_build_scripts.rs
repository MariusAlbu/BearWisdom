// =============================================================================
// ecosystem/cargo_build_scripts.rs — index Cargo build-script OUT_DIR sources
//
// The canonical `include!(concat!(env!("OUT_DIR"), "/<file>.rs"))` pattern
// is widespread in the Rust ecosystem (prost / tonic gRPC, sqlx compile-
// time SQL, build-time codegen like scryer-prolog's `Instruction` enum,
// any project with a `build.rs` that emits Rust source). The generated
// file lives at `target/<profile>/build/<crate-hash>/out/<file>.rs` and
// only exists after a `cargo build` / `cargo check` populates it. Without
// indexing those files, the symbols they declare (enum variants, type
// aliases, derive-macro outputs) all land in unresolved_refs.
//
// **Discovery strategy**:
//   1. Activate when the project has a `Cargo.toml` AND the package
//      declares a build script (`build = "..."` in `[package]`, or a
//      `build.rs` at the package root).
//   2. Locate `target/{debug,release}/build/<package-name>-*/out/` and
//      enumerate every `.rs` file inside.
//   3. Emit each as a `WalkedFile` — the standard Rust extractor walks
//      it normally, contributing its symbols to the same SymbolIndex
//      as the rest of the project (resolution can pick them up via
//      same-package by-name lookups).
//
// Only the host package's OUT_DIR is indexed — transitive dependencies'
// generated code (markup5ever's HTML entities, typenum's test fixtures)
// would multiply the index by hundreds of files of derive-macro output
// that user code rarely needs. The host package is identified by the
// `[package].name` field in the project root's Cargo.toml; the build
// directory under `target/.../build/` is matched as
// `<package-name>-<16-hex-hash>`.
//
// When `cargo check` hasn't been run, `target/` may not exist —
// activation succeeds but `locate_roots` returns empty. The honest
// signal: codegen-derived symbols stay unresolved until the user
// builds.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cargo-build-scripts");
const ECOSYSTEM_TAG: &str = "cargo-build-scripts";
const LANGUAGES: &[&str] = &["rust"];

pub struct CargoBuildScriptsEcosystem;

impl Ecosystem for CargoBuildScriptsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("rust")
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_out_dirs(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_out_dir(dep)
    }

    fn uses_demand_driven_parse(&self) -> bool { false }
}

impl ExternalSourceLocator for CargoBuildScriptsEcosystem {
    fn ecosystem(&self) -> &'static str { ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_out_dirs(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_out_dir(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CargoBuildScriptsEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CargoBuildScriptsEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_out_dirs(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(package_name) = read_package_name(project_root) else {
        return Vec::new();
    };
    if !package_has_build_script(project_root) {
        return Vec::new();
    }
    let mut roots = Vec::new();
    for profile in &["debug", "release"] {
        let build_dir = project_root.join("target").join(profile).join("build");
        if !build_dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&build_dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.starts_with(&format!("{package_name}-")) {
                continue;
            }
            let out = path.join("out");
            if !out.is_dir() {
                continue;
            }
            roots.push(ExternalDepRoot {
                module_path: format!("{package_name}-build-out-{profile}"),
                version: String::from("local"),
                root: out,
                ecosystem: ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }
    if !roots.is_empty() {
        tracing::info!(
            "cargo-build-scripts: indexing {} OUT_DIR root(s) for {}",
            roots.len(),
            package_name
        );
    }
    roots
}

/// Read the `[package].name` field from `Cargo.toml`. Returns None when
/// the project root isn't a Cargo package (workspace-only manifests
/// without `[package]`, or no Cargo.toml at all).
fn read_package_name(project_root: &Path) -> Option<String> {
    let cargo_toml = project_root.join("Cargo.toml");
    let text = std::fs::read_to_string(&cargo_toml).ok()?;
    parse_package_name(&text)
}

pub(crate) fn parse_package_name(toml_text: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml_text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        // `name = "scryer-prolog"` — strip whitespace around `=`, accept
        // either single or double quotes.
        let Some(rest) = line.strip_prefix("name") else { continue };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim();
        let value = rest
            .trim_matches(|c| c == '"' || c == '\'')
            .trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// True iff Cargo.toml declares a build script (`build = "build.rs"`,
/// `build = "build/main.rs"`, etc.) OR a `build.rs` file sits at the
/// package root (Cargo's default discovery).
pub(crate) fn package_has_build_script(project_root: &Path) -> bool {
    if project_root.join("build.rs").is_file() {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(project_root.join("Cargo.toml")) else {
        return false;
    };
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if trimmed.starts_with("build") {
            let rest = trimmed.trim_start_matches("build").trim_start();
            if rest.starts_with('=') {
                let value = rest[1..].trim().trim_matches(|c| c == '"' || c == '\'');
                if !value.is_empty() && value != "false" {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

fn walk_out_dir(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".rs") {
                continue;
            }
            let display = path.to_string_lossy().replace('\\', "/");
            let rel = format!("ext:{ECOSYSTEM_TAG}:{display}");
            out.push(WalkedFile {
                relative_path: rel,
                absolute_path: path.clone(),
                language: "rust",
            });
        }
    }
}

#[cfg(test)]
#[path = "cargo_build_scripts_tests.rs"]
mod tests;

// Helpers exposed for sibling tests.
#[cfg(test)]
pub(crate) fn _test_discover_out_dirs(p: &Path) -> Vec<ExternalDepRoot> {
    discover_out_dirs(p)
}
