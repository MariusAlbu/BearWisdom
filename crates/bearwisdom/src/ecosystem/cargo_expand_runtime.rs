// =============================================================================
// ecosystem/cargo_expand_runtime.rs — index proc-macro-expanded Rust source
//
// Some Rust ecosystems (modular_bitfield's `#[bitfield]` setters,
// prost-build's protobuf message accessors, sqlx's `query!` row types)
// generate methods/types via proc-macros that:
//   * Don't write to OUT_DIR — the expansion happens in-memory at compile
//     time, so PR 143's `cargo-build-scripts` walker can't reach them.
//   * Don't appear in checked-in source either — `with_m`/`with_f`
//     bitfield setters in scryer-prolog's atom_table only exist after
//     `#[bitfield]` runs.
//
// The canonical tool for capturing this is `cargo expand`. Given the
// binary is installed (`cargo install cargo-expand`), it runs the
// proc-macros and prints the fully-expanded crate source to stdout.
// We capture it once per project, write it under
// `target/.bw-cargo-expand/<package>.rs`, and walk that file as a
// synthetic ext: Rust source.
//
// **Opt-in only**: cargo expand takes 10–30 seconds on a cold cache and
// requires the project to be buildable. Activation gates on the
// `BEARWISDOM_CARGO_EXPAND` env var being set to a non-empty value
// (`1`, `true`, etc.); BW indexes don't pay the cost without explicit
// opt-in.
//
// **Cache**: re-runs only if the output file is missing OR older than
// `Cargo.toml` / `Cargo.lock` / `build.rs`. A one-line content check
// keeps cargo expand idempotent across BW reindexes.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::cargo_build_scripts;
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cargo-expand-runtime");
const ECOSYSTEM_TAG: &str = "cargo-expand-runtime";
const LANGUAGES: &[&str] = &["rust"];
const ENV_OPT_IN: &str = "BEARWISDOM_CARGO_EXPAND";

pub struct CargoExpandRuntimeEcosystem;

impl Ecosystem for CargoExpandRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("rust")
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        ensure_expanded_for(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        let path = dep.root.join("expanded.rs");
        if !path.is_file() {
            return Vec::new();
        }
        let display = path.to_string_lossy().replace('\\', "/");
        vec![WalkedFile {
            relative_path: format!("ext:{ECOSYSTEM_TAG}:{display}"),
            absolute_path: path,
            language: "rust",
        }]
    }

    fn uses_demand_driven_parse(&self) -> bool { false }
}

impl ExternalSourceLocator for CargoExpandRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        ensure_expanded_for(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        let path = dep.root.join("expanded.rs");
        if !path.is_file() {
            return Vec::new();
        }
        let display = path.to_string_lossy().replace('\\', "/");
        vec![WalkedFile {
            relative_path: format!("ext:{ECOSYSTEM_TAG}:{display}"),
            absolute_path: path,
            language: "rust",
        }]
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CargoExpandRuntimeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CargoExpandRuntimeEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery + cache
// ---------------------------------------------------------------------------

fn ensure_expanded_for(project_root: &Path) -> Vec<ExternalDepRoot> {
    if !env_opt_in_set() {
        return Vec::new();
    }
    if !cargo_expand_in_path() {
        tracing::debug!(
            "cargo-expand-runtime: BEARWISDOM_CARGO_EXPAND set but `cargo expand` \
             not in PATH — install with `cargo install cargo-expand`"
        );
        return Vec::new();
    }
    let Some(package_name) = read_package_name(project_root) else {
        return Vec::new();
    };
    let cache_dir = project_root
        .join("target")
        .join(".bw-cargo-expand");
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        tracing::warn!("cargo-expand-runtime: cannot create cache dir: {e}");
        return Vec::new();
    }
    let expanded = cache_dir.join("expanded.rs");

    if cache_is_fresh(project_root, &expanded) {
        tracing::debug!(
            "cargo-expand-runtime: using cached expansion at {}",
            expanded.display()
        );
    } else if let Err(e) = run_cargo_expand(project_root, &expanded) {
        tracing::warn!("cargo-expand-runtime: expand failed: {e}");
        return Vec::new();
    }

    if !expanded.is_file() {
        return Vec::new();
    }
    vec![ExternalDepRoot {
        module_path: format!("{package_name}-expand"),
        version: String::from("local"),
        root: cache_dir,
        ecosystem: ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn env_opt_in_set() -> bool {
    matches!(
        std::env::var(ENV_OPT_IN).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

fn cargo_expand_in_path() -> bool {
    // cargo subcommand check: invoke `cargo expand --version` with a short
    // timeout. If the binary is missing, cargo prints "no such command".
    let Ok(output) = std::process::Command::new("cargo")
        .args(["expand", "--version"])
        .output()
    else {
        return false;
    };
    output.status.success()
}

fn read_package_name(project_root: &Path) -> Option<String> {
    let cargo_toml = project_root.join("Cargo.toml");
    let text = std::fs::read_to_string(&cargo_toml).ok()?;
    cargo_build_scripts::parse_package_name(&text)
}

/// Cache is fresh when the expanded file exists AND its mtime is newer
/// than every input that influences expansion: `Cargo.toml`,
/// `Cargo.lock`, `build.rs`, and every `.rs` file under `src/`.
/// Conservatively returns false (recompute) on any I/O error.
fn cache_is_fresh(project_root: &Path, expanded: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(expanded) else { return false };
    let Ok(out_mtime) = meta.modified() else { return false };

    let inputs = [
        project_root.join("Cargo.toml"),
        project_root.join("Cargo.lock"),
        project_root.join("build.rs"),
    ];
    for input in &inputs {
        if let Some(t) = file_mtime(input) {
            if t > out_mtime {
                return false;
            }
        }
    }
    if let Some(latest) = latest_mtime_in_dir(&project_root.join("src")) {
        if latest > out_mtime {
            return false;
        }
    }
    true
}

fn file_mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok().and_then(|m| m.modified().ok())
}

fn latest_mtime_in_dir(dir: &Path) -> Option<SystemTime> {
    let mut latest: Option<SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                if let Some(t) = file_mtime(&path) {
                    if latest.map_or(true, |best| t > best) {
                        latest = Some(t);
                    }
                }
            }
        }
    }
    latest
}

fn run_cargo_expand(project_root: &Path, output: &Path) -> std::io::Result<()> {
    tracing::info!(
        "cargo-expand-runtime: running `cargo expand --lib` in {}",
        project_root.display()
    );
    let result = std::process::Command::new("cargo")
        .args(["expand", "--lib", "--color=never"])
        .current_dir(project_root)
        .output()?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("cargo expand failed: {}", stderr.lines().take(3).collect::<Vec<_>>().join(" | ")),
        ));
    }
    std::fs::write(output, &result.stdout)?;
    tracing::info!(
        "cargo-expand-runtime: wrote {} bytes to {}",
        result.stdout.len(),
        output.display()
    );
    Ok(())
}

#[cfg(test)]
#[path = "cargo_expand_runtime_tests.rs"]
mod tests;
