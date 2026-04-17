// =============================================================================
// ecosystem/erlang_otp.rs — Erlang OTP stdlib (stdlib ecosystem)
//
// Probes the Erlang install's lib/ dir (`$ERL_HOME/lib` or the result of
// `erl -noshell -eval 'io:format("~s", [code:lib_dir()]), halt().'`).
// Each OTP application is a subdir like `kernel-8.5.4/` containing a
// `src/*.erl` tree.
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

pub const ID: EcosystemId = EcosystemId::new("erlang-otp");
const LEGACY_ECOSYSTEM_TAG: &str = "erlang-otp";
const LANGUAGES: &[&str] = &["erlang"];

pub struct ErlangOtpEcosystem;

impl Ecosystem for ErlangOtpEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("erlang"),
            EcosystemActivation::LanguagePresent("elixir"),
        ])
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

impl ExternalSourceLocator for ErlangOtpEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

fn discover() -> Vec<ExternalDepRoot> {
    let Some(lib_dir) = probe_otp_lib() else {
        debug!("erlang-otp: no OTP lib dir probed");
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&lib_dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        // Keep only OTP app dirs (have a src/ subdir). Skip tooling leftovers.
        let src_dir = path.join("src");
        if !src_dir.is_dir() { continue }
        // Module path: strip the trailing "-X.Y.Z" version.
        let module = name.split('-').next().unwrap_or(name).to_string();
        out.push(ExternalDepRoot {
            module_path: module,
            version: name
                .split_once('-')
                .map(|(_, v)| v.to_string())
                .unwrap_or_default(),
            root: src_dir,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
    out
}

fn probe_otp_lib() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_OTP_LIB_DIR") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    if let Some(home) = std::env::var_os("ERL_HOME") {
        let p = PathBuf::from(home).join("lib");
        if p.is_dir() { return Some(p); }
    }
    if let Ok(output) = Command::new("erl")
        .args(["-noshell", "-eval", "io:format(\"~s\", [code:lib_dir()]), halt()."])
        .output()
    {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(s);
                if p.is_dir() { return Some(p); }
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
    if depth >= 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".erl") || name.ends_with(".hrl")) { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:erlang:{}", display),
                absolute_path: path,
                language: "erlang",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ErlangOtpEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ErlangOtpEcosystem)).clone()
}
