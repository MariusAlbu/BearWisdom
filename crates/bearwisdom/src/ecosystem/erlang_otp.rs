// =============================================================================
// ecosystem/erlang_otp.rs — Erlang OTP stdlib walker
//
// OTP ships as `.erl` / `.hrl` source under the Erlang install's `lib/`
// directory. Every Erlang project uses `kernel` and `stdlib` unconditionally
// — they are language substrate, not optional deps. `LanguagePresent("erlang")`
// is therefore the correct activation rule.
//
// Probe order (degrades cleanly to empty when nothing found):
//   1. $BEARWISDOM_OTP_ROOT — explicit override: the dir whose `lib/` and
//      `bin/` subdirs define the install (e.g. the scoop `current/` link).
//   2. $ERL_TOP — sometimes set by Erlang devs doing OTP source work.
//   3. Common install paths per platform (Windows scoop first, then standard
//      Program Files patterns; Linux /usr/lib/erlang; macOS homebrew/local).
//   4. `erl` binary query — slow (JVM-style startup); last resort.
//
// Walking:
//   For each `lib/<app>-<version>/src/` directory, every `.erl` and `.hrl`
//   file is walked. `test/`, `examples/`, `examples_*/`, `doc/`, `priv/`,
//   and hidden directories are skipped.
//
//   `WalkedFile.relative_path` = `ext:erlang:<app-base>/<rel-from-src>` so
//   the virtual path mirrors the OTP app name and file structure without
//   embedding machine-local absolute paths.
//
// Demand pre-pull:
//   `kernel` and `stdlib` are pre-pulled eagerly because every Erlang module
//   uses their symbols (io:format, gen_server:cast, lists:map, etc.) without
//   explicit imports. Other OTP apps remain demand-driven.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("erlang-otp");
const LEGACY_ECOSYSTEM_TAG: &str = "erlang-otp";
const LANGUAGES: &[&str] = &["erlang"];

/// OTP apps that every Erlang module implicitly depends on. Their src trees
/// are pre-pulled so bare-name and qualified resolution (gen_server, lists,
/// io, erlang BIFs) can bind without waiting for the demand BFS.
const SUBSTRATE_APPS: &[&str] = &["kernel", "stdlib"];

pub struct ErlangOtpEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait
// ---------------------------------------------------------------------------

impl Ecosystem for ErlangOtpEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // OTP is the Erlang language substrate — every Erlang project
        // unconditionally uses kernel + stdlib.
        EcosystemActivation::LanguagePresent("erlang")
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["test", "examples", "doc", "priv"]
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }
    fn is_workspace_global(&self) -> bool { true }

    fn demand_pre_pull(&self, dep_roots: &[ExternalDepRoot]) -> Vec<WalkedFile> {
        // Eagerly surface kernel and stdlib so the resolver can bind
        // unqualified and qualified OTP calls on the first pass.
        dep_roots
            .iter()
            .filter(|dep| SUBSTRATE_APPS.contains(&dep.module_path.as_str()))
            .flat_map(walk)
            .collect()
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_otp_symbol_index(dep_roots)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl (back-compat bridge)
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for ErlangOtpEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ErlangOtpEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ErlangOtpEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover() -> Vec<ExternalDepRoot> {
    let Some(lib_dir) = probe_otp_lib() else {
        debug!("erlang-otp: no OTP lib directory found");
        return Vec::new();
    };

    debug!("erlang-otp: scanning {}", lib_dir.display());

    let Ok(entries) = std::fs::read_dir(&lib_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // OTP app directories are named `<app>-<version>/` and contain a
        // `src/` subdir with Erlang sources.
        let src_dir = path.join("src");
        if !src_dir.is_dir() {
            continue;
        }
        let app_name = dir_name.split('-').next().unwrap_or(dir_name).to_string();
        let version = dir_name
            .split_once('-')
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();

        out.push(ExternalDepRoot {
            module_path: app_name,
            version,
            root: src_dir,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }

    // Stable order so recapture runs are deterministic.
    out.sort_by(|a, b| a.module_path.cmp(&b.module_path));
    out
}

/// Probe the OTP installation lib directory via the documented fallback chain.
fn probe_otp_lib() -> Option<PathBuf> {
    // 1. Explicit BearWisdom override — point at the OTP install root whose
    //    `lib/` subdir holds the OTP app directories.
    if let Some(val) = std::env::var_os("BEARWISDOM_OTP_ROOT") {
        let p = PathBuf::from(val);
        if let Some(lib) = check_otp_root(&p) {
            return Some(lib);
        }
    }

    // 2. ERL_TOP — set by Erlang developers building OTP from source.
    if let Some(val) = std::env::var_os("ERL_TOP") {
        let p = PathBuf::from(val).join("lib");
        if p.is_dir() {
            return Some(p);
        }
    }

    // 3. Common install paths, platform-specific.
    for candidate in platform_install_roots() {
        if let Some(lib) = check_otp_root(&candidate) {
            return Some(lib);
        }
    }

    // 4. Ask `erl` binary — slow due to emulator startup; last resort.
    probe_erl_binary()
}

fn check_otp_root(root: &Path) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    let lib = root.join("lib");
    if lib.is_dir() { Some(lib) } else { None }
}

fn platform_install_roots() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // Windows — scoop `current/` link (most common dev install on this
    // machine), then Program Files with `erl*` / `Erlang*` prefixes.
    if cfg!(target_os = "windows") {
        if let Some(home) = std::env::var_os("USERPROFILE") {
            candidates.push(
                PathBuf::from(home)
                    .join("scoop")
                    .join("apps")
                    .join("erlang")
                    .join("current"),
            );
        }
        for prefix in &["C:/Program Files", "C:/Program Files (x86)"] {
            let dir = PathBuf::from(prefix);
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if !p.is_dir() {
                            continue;
                        }
                        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                            continue;
                        };
                        if name.starts_with("erl") || name.starts_with("Erlang") {
                            candidates.push(p);
                        }
                    }
                }
            }
        }
    }

    // Linux standard paths.
    if cfg!(target_os = "linux") {
        candidates.push(PathBuf::from("/usr/lib/erlang"));
        candidates.push(PathBuf::from("/usr/local/lib/erlang"));
        candidates.push(PathBuf::from("/opt/erlang"));
    }

    // macOS — homebrew Apple Silicon, then Intel/local, then /opt.
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/opt/homebrew/lib/erlang"));
        candidates.push(PathBuf::from("/usr/local/lib/erlang"));
        candidates.push(PathBuf::from("/opt/erlang"));
    }

    candidates
}

fn probe_erl_binary() -> Option<PathBuf> {
    for program in ["erl", "erl.exe"] {
        let Ok(out) = Command::new(program)
            .args(["-noshell", "-eval", "io:format(\"~s\", [code:lib_dir()]), halt()."])
            .output()
        else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let raw = String::from_utf8_lossy(&out.stdout);
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let p = PathBuf::from(s);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &dep.root, &dep.module_path, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, src_root: &Path, app_name: &str, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Skip test fixtures, generated docs, example apps, private data,
            // and hidden directories — they are not part of the public OTP API.
            if matches!(name, "test" | "examples" | "doc" | "priv")
                || name.starts_with("examples_")
                || name.starts_with('.')
            {
                continue;
            }
            walk_dir(&path, src_root, app_name, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".erl") && !name.ends_with(".hrl") {
                continue;
            }
            let rel = match path.strip_prefix(src_root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:erlang:{app_name}/{rel}"),
                absolute_path: path,
                language: "erlang",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol index — module-name → file map for demand-driven resolution
// ---------------------------------------------------------------------------

/// Build a `(app, module_name) → file` index by scanning each `.erl` file
/// for its `-module(Name).` declaration. No full tree-sitter parse — the
/// attribute is always near the top of the file.
pub(crate) fn build_otp_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();

    for dep in dep_roots {
        let walked = walk(dep);
        for wf in &walked {
            if !wf.relative_path.ends_with(".erl") {
                continue;
            }
            let Some(module_name) = extract_module_name(&wf.absolute_path) else {
                continue;
            };
            // Index under the OTP app name so `locate("kernel", "gen_server")`
            // resolves, and also under the module name directly so chain
            // walkers that know only the Erlang module name can find the file.
            index.insert(&dep.module_path, module_name.clone(), wf.absolute_path.clone());
            index.insert(module_name.clone(), module_name.clone(), wf.absolute_path.clone());
        }
    }

    index
}

/// Read an `.erl` file's `-module(Name).` attribute from the first 50 lines.
fn extract_module_name(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for raw in content.lines().take(50) {
        let line = strip_erlang_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("-module(") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn strip_erlang_comment(line: &str) -> &str {
    match line.find('%') {
        Some(idx) => &line[..idx],
        None => line,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "erlang_otp_tests.rs"]
mod tests;
