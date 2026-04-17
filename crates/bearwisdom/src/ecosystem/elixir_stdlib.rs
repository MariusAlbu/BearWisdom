// =============================================================================
// ecosystem/elixir_stdlib.rs — Elixir stdlib (stdlib ecosystem)
//
// Probes the Elixir install's `lib/elixir/lib/` dir (containing Kernel.ex,
// Enum.ex, ...) via $ELIXIR_HOME or the binary's `code:lib_dir(:elixir)`
// path. Walks .ex files.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("elixir-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "elixir-stdlib";
const LANGUAGES: &[&str] = &["elixir"];

pub struct ElixirStdlibEcosystem;

impl Ecosystem for ElixirStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("elixir")
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

impl ExternalSourceLocator for ElixirStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

fn discover() -> Vec<ExternalDepRoot> {
    let Some(src_dir) = probe_elixir_src() else {
        debug!("elixir-stdlib: no source tree probed");
        return Vec::new();
    };
    vec![ExternalDepRoot {
        module_path: "elixir".to_string(),
        version: String::new(),
        root: src_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
    }]
}

fn probe_elixir_src() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_ELIXIR_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    if let Some(home) = std::env::var_os("ELIXIR_HOME") {
        let base = PathBuf::from(home);
        for candidate in [
            base.join("lib").join("elixir").join("lib"),
            base.join("lib"),
        ] {
            if candidate.is_dir() { return Some(candidate); }
        }
    }
    // Walk from elixir binary's install root.
    if let Ok(output) = Command::new("elixir")
        .args(["-e", "IO.puts(:code.lib_dir(:elixir))"])
        .output()
    {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                let ebin = PathBuf::from(s);
                // ebin sits next to lib/ in the Elixir install; we want lib/.
                if let Some(parent) = ebin.parent() {
                    let lib = parent.join("lib");
                    if lib.is_dir() { return Some(lib); }
                }
            }
        }
    }
    None
}

fn walk(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 8 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests") { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".ex") || name.ends_with(".exs")) { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:elixir:{}", display),
                absolute_path: path,
                language: "elixir",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ElixirStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ElixirStdlibEcosystem)).clone()
}
