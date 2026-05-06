// =============================================================================
// ecosystem/nvim_runtime.rs — Neovim Lua runtime (stdlib-style ecosystem)
//
// Neovim ships its own Lua API (vim.api.*, vim.lsp.*, vim.diagnostic.*, …)
// + the busted-style test framework + libuv handle methods as readable .lua
// source under `$VIMRUNTIME/lua/`. Plugin code under `lua-telescope/`,
// `lua-lazy-nvim/`, `lua-nvim-lspconfig/` references this surface
// pervasively — without indexing the runtime, every `vim.api.nvim_*` call,
// every `it(...)` / `describe(...)` test, every `vim.tbl_extend(...)`
// bare method call lands in unresolved_refs.
//
// This ecosystem is the architectural counterpart to `cpython_stdlib`: it
// probes for an installed Neovim and walks the runtime lua tree.
//
// Probe order:
//   1. $BEARWISDOM_NVIM_RUNTIME — explicit dir override.
//   2. $VIMRUNTIME — set by Neovim itself; users running `bw` from a Neovim
//      terminal inherit it.
//   3. `nvim --headless -c 'echo $VIMRUNTIME' -c 'qa'` — query an installed
//      binary directly. Adds ~50ms when nvim is on PATH.
//   4. Standard install paths: `/usr/share/nvim/runtime`,
//      `/opt/homebrew/share/nvim/runtime`, `C:\Program Files\Neovim\
//      share\nvim\runtime`.
//
// Walk: every `.lua` file under `<runtime>/lua/`. Skips test/, fixtures/,
// and other documentation/sample subtrees. The vim.* API surface lives at
// `<runtime>/lua/vim/` (top-level + nested modules); busted assertions live
// at `<runtime>/lua/luassert/` when the user has plenary or busted
// installed alongside.
//
// Activation: any `.lua` file in the project. When Neovim isn't installed,
// the probe returns empty and the ecosystem silently degrades — matching
// BearWisdom's "toolchains must be installed for full resolution" policy.
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

pub const ID: EcosystemId = EcosystemId::new("nvim-runtime");
const LEGACY_ECOSYSTEM_TAG: &str = "nvim-runtime";
const LANGUAGES: &[&str] = &["lua"];

pub struct NvimRuntimeEcosystem;

impl Ecosystem for NvimRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // Lua is used in many non-Neovim contexts (game scripting, OpenResty,
        // LuaTeX, embedded scripting in Redis/nginx). The Neovim signal is
        // an init.lua referencing the `vim.` namespace — both Neovim configs
        // (`init.lua` at config root) and Neovim plugins (`lua/<plugin>/
        // init.lua`) hit this. A plain Lua project with no `vim.` references
        // does not.
        EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/init.lua",
            field_path: "",
            value: "vim.",
        }
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_nvim_runtime()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_runtime_tree(dep)
    }
}

impl ExternalSourceLocator for NvimRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_nvim_runtime()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_runtime_tree(dep)
    }
}

fn discover_nvim_runtime() -> Vec<ExternalDepRoot> {
    let Some(runtime_dir) = probe_runtime_dir() else {
        debug!("nvim-runtime: no Neovim runtime probed");
        return Vec::new();
    };
    debug!("nvim-runtime: using {}", runtime_dir.display());
    vec![ExternalDepRoot {
        module_path: "nvim-runtime".to_string(),
        version: String::new(),
        root: runtime_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_runtime_dir() -> Option<PathBuf> {
    // 1. Explicit override via env var.
    if let Some(explicit) = std::env::var_os("BEARWISDOM_NVIM_RUNTIME") {
        let p = PathBuf::from(explicit);
        if p.is_dir() && has_lua_subdir(&p) { return Some(p); }
    }
    // 2. $VIMRUNTIME set by Neovim itself (running inside `:terminal` etc.).
    if let Some(env_runtime) = std::env::var_os("VIMRUNTIME") {
        let p = PathBuf::from(env_runtime);
        if p.is_dir() && has_lua_subdir(&p) { return Some(p); }
    }
    // 3. Query an installed `nvim` binary directly. Bounded — failures
    //    (binary missing, --headless not supported, timeout) all degrade
    //    to the next probe step.
    if let Some(p) = probe_via_nvim_command() {
        if has_lua_subdir(&p) { return Some(p); }
    }
    // 4. Standard install paths.
    for candidate in [
        "/usr/share/nvim/runtime",
        "/usr/local/share/nvim/runtime",
        "/opt/homebrew/share/nvim/runtime",
        "/opt/local/share/nvim/runtime",
        "C:/Program Files/Neovim/share/nvim/runtime",
        "C:/Program Files (x86)/Neovim/share/nvim/runtime",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_dir() && has_lua_subdir(&p) { return Some(p); }
    }
    None
}

fn has_lua_subdir(runtime: &Path) -> bool {
    runtime.join("lua").is_dir()
}

fn probe_via_nvim_command() -> Option<PathBuf> {
    // `:echo $VIMRUNTIME` writes to the message area; `--headless` runs
    // without UI, `qa` quits cleanly. Stderr captures the echoed value
    // because Neovim writes :echo output to stderr in headless mode.
    let output = Command::new("nvim")
        .args(["--headless", "-c", "echo $VIMRUNTIME", "-c", "qa"])
        .output()
        .ok()?;
    let combined = if !output.stderr.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    let trimmed = combined.trim();
    if trimmed.is_empty() { return None; }
    // Multi-line outputs occasionally happen (errors before the echo);
    // pick the last non-empty line that points at an existing directory.
    for line in trimmed.lines().rev() {
        let candidate = PathBuf::from(line.trim());
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

fn walk_runtime_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    // The vim.* Lua API lives under `<runtime>/lua/`. Walking the rest of
    // the runtime (`<runtime>/syntax/`, `<runtime>/doc/`, etc.) would pick
    // up Vimscript and documentation that doesn't help Lua resolution.
    let lua_root = dep.root.join("lua");
    if !lua_root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk_lua_dir(&lua_root, &mut out, 0);
    out
}

fn walk_lua_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 8 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip test and example trees that don't contribute to the
                // public API surface.
                if matches!(name, "test" | "tests" | "fixtures" | "spec" | "specs") {
                    continue;
                }
                if name.starts_with('.') { continue }
            }
            walk_lua_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".lua") { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:nvim:{display}"),
                absolute_path: path,
                language: "lua",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NvimRuntimeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NvimRuntimeEcosystem)).clone()
}

#[cfg(test)]
#[path = "nvim_runtime_tests.rs"]
mod tests;
