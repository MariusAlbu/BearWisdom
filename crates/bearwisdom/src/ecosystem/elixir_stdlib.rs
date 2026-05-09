// =============================================================================
// ecosystem/elixir_stdlib.rs — Elixir stdlib (stdlib ecosystem)
//
// Probes the Elixir install's `lib/<app>/lib/` dirs via $ELIXIR_HOME or the
// binary's `code:lib_dir(:elixir)` path. Walks .ex files for every shipped
// application: `elixir` (Kernel/Enum/...), `ex_unit` (assert/refute/...),
// `mix`, `iex`, `eex`, `logger`. Without ex_unit/mix on the walked surface
// most Elixir test suites (Plausible's 5k `assert` calls) stay unresolved.
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

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        super::hex::build_hex_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for ElixirStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

fn discover() -> Vec<ExternalDepRoot> {
    let Some(install_lib) = probe_elixir_install_lib() else {
        debug!("elixir-stdlib: no install lib/ tree probed");
        return Vec::new();
    };
    // Each shipped application sits at lib/<app>/lib/. Emit one root per
    // app so the resolver sees ExUnit's `assert`/`refute`, Mix's tasks, IEx
    // helpers, Logger, EEx etc. — not just Kernel/Enum.
    let mut roots = Vec::new();
    let Ok(entries) = std::fs::read_dir(&install_lib) else { return roots };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue }
        let app_root = entry.path().join("lib");
        if !app_root.is_dir() { continue }
        let app_name = entry
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("elixir-app")
            .to_string();
        roots.push(ExternalDepRoot {
            module_path: app_name,
            version: String::new(),
            root: app_root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
    if roots.is_empty() {
        // Fallback: at least walk the install_lib root directly for
        // installs with a flat layout we don't recognise.
        roots.push(ExternalDepRoot {
            module_path: "elixir".to_string(),
            version: String::new(),
            root: install_lib,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
    roots
}

fn probe_elixir_install_lib() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_ELIXIR_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    if let Some(home) = std::env::var_os("ELIXIR_HOME") {
        let base = PathBuf::from(home);
        let lib = base.join("lib");
        if lib.is_dir() { return Some(lib); }
    }
    // Walk from elixir binary's install root via `:code.lib_dir(:elixir)`.
    // Returns `<install>/lib/elixir`; the parent of that is `<install>/lib`,
    // the install-wide lib/ that contains every shipped app dir.
    //
    // On Windows the canonical entrypoint is `elixir.bat` — Rust's
    // `Command::new("elixir")` won't auto-append `.bat` (PATHEXT lookup is
    // not done by std::process::Command). Try both names; on Windows fall
    // back to `cmd /C elixir ...` so the shell can resolve the shim.
    let probe_via = |program: &str, prefix_args: &[&str]| -> Option<PathBuf> {
        let mut cmd = Command::new(program);
        for a in prefix_args { cmd.arg(a); }
        cmd.args(["-e", "IO.puts(:code.lib_dir(:elixir))"]);
        let out = cmd.output().ok()?;
        if !out.status.success() { return None; }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { return None; }
        let elixir_app = PathBuf::from(s);
        let install_lib = elixir_app.parent()?;
        if install_lib.is_dir() { Some(install_lib.to_path_buf()) } else { None }
    };

    if let Some(p) = probe_via("elixir", &[]) { return Some(p); }
    #[cfg(windows)]
    {
        if let Some(p) = probe_via("elixir.bat", &[]) { return Some(p); }
        if let Some(p) = probe_via("cmd", &["/C", "elixir"]) { return Some(p); }
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
