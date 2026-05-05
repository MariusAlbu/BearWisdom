// =============================================================================
// ecosystem/compile_commands.rs — generic C/C++ include-path consumer
//
// Reads `compile_commands.json` from the project (CMake / Bear /
// intercept-build all produce this format) and surfaces every `-I<path>`
// and `-isystem <path>` argument as an external dep root. The header
// indexer is the same one used by PosixHeadersEcosystem / MsvcHeadersEcosystem
// / VcpkgHeadersEcosystem — demand-driven, parse only what the project
// actually `#include`s.
//
// Why this exists
// ---------------
// Per-SDK walkers (qt-runtime, msvc-headers, vcpkg-headers, …) work but
// they multiply for every new C/C++ library/SDK the user encounters
// (Boost, CUDA, ROS, Intel MKL, internal corporate SDKs). For any project
// that's been built once, `compile_commands.json` lists the EXACT set of
// `-I` paths the compiler used, which is the ground truth. One consumer of
// that file replaces every system-SDK walker for every CMake / Bazel
// project in the corpus.
//
// Discovery
// ---------
// Probes (in order):
//   1. `<project_root>/compile_commands.json`            (CMake export)
//   2. `<project_root>/build/compile_commands.json`      (default cmake build dir)
//   3. `<project_root>/build-*/compile_commands.json`    (build-Debug, build-Release, ...)
//   4. `<project_root>/cmake-build-*/compile_commands.json` (CLion convention)
//
// Activation: any C/C++ project. Probes short-circuit when no
// compile_commands.json exists, so non-CMake projects pay nothing.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tracing::debug;

use super::posix_headers::{build_c_header_index, make_root as make_posix_root, resolve_header};
use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("compile-commands");
const TAG: &str = "compile-commands";

pub struct CompileCommandsEcosystem;

impl Ecosystem for CompileCommandsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { &["c", "cpp"] }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("c"),
            EcosystemActivation::LanguagePresent("cpp"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_from_compile_commands(ctx.project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_c_header_index(dep_roots)
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        header: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_header(dep, header).into_iter().collect()
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        resolve_header(dep, fqn).into_iter().collect()
    }
}

impl ExternalSourceLocator for CompileCommandsEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_from_compile_commands(project_root)
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CompileCommandsEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CompileCommandsEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Entry {
    /// The compiler invocation as a single shell string (CMake's default
    /// shape on POSIX). We tokenize ourselves; respect simple quotes but
    /// do not implement full shell parsing — `-I` flags don't need it.
    #[serde(default)]
    command: String,
    /// Pre-tokenized argv (CMake's `Ninja` generator + Bear). When present,
    /// take it verbatim and skip command tokenization.
    #[serde(default)]
    arguments: Vec<String>,
    /// Working directory the compile ran in. Required to resolve relative
    /// `-I./foo/bar` paths. Defaults to the directory holding
    /// compile_commands.json if absent.
    #[serde(default)]
    directory: String,
}

fn discover_from_compile_commands(project_root: &Path) -> Vec<ExternalDepRoot> {
    let cc_path = match locate_compile_commands(project_root) {
        Some(p) => p,
        None => {
            debug!("compile-commands: no compile_commands.json under {:?}", project_root);
            return Vec::new();
        }
    };

    let raw = match std::fs::read_to_string(&cc_path) {
        Ok(s) => s,
        Err(e) => {
            debug!("compile-commands: failed to read {:?}: {}", cc_path, e);
            return Vec::new();
        }
    };

    let entries: Vec<Entry> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            debug!("compile-commands: failed to parse {:?}: {}", cc_path, e);
            return Vec::new();
        }
    };

    let cc_dir = cc_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    // Collect every distinct include path across every entry.
    let mut paths: HashSet<PathBuf> = HashSet::new();
    for entry in &entries {
        let dir = if entry.directory.is_empty() {
            cc_dir.clone()
        } else {
            PathBuf::from(&entry.directory)
        };
        let argv = if !entry.arguments.is_empty() {
            entry.arguments.clone()
        } else {
            tokenize_command(&entry.command)
        };
        extract_include_paths(&argv, &dir, &mut paths);
    }

    if paths.is_empty() {
        debug!("compile-commands: parsed {} entries but no -I/-isystem args found", entries.len());
        return Vec::new();
    }

    // Dedup by canonical path AND filter to existing directories.
    let mut canonical_seen: HashSet<PathBuf> = HashSet::new();
    let mut roots = Vec::new();
    for p in paths {
        if !p.is_dir() { continue }
        let canonical = p.canonicalize().unwrap_or_else(|_| p.clone());
        if !canonical_seen.insert(canonical) { continue }
        roots.push(make_posix_root(&p, TAG));
    }

    debug!(
        "compile-commands: {} unique include roots from {} entries in {:?}",
        roots.len(), entries.len(), cc_path
    );
    roots
}

/// Find compile_commands.json under the project root. Returns the first
/// hit from a list of conventional locations.
fn locate_compile_commands(project_root: &Path) -> Option<PathBuf> {
    let direct = project_root.join("compile_commands.json");
    if direct.is_file() { return Some(direct) }

    let build = project_root.join("build").join("compile_commands.json");
    if build.is_file() { return Some(build) }

    // build-Debug, build-Release, build-RelWithDebInfo, etc. — walk the
    // project root once and pick the first match.
    let Ok(entries) = std::fs::read_dir(project_root) else { return None };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("build-")
            || name_str.starts_with("cmake-build-")
            || name_str == "out"  // VS Code CMake Tools default
        {
            let candidate = entry.path().join("compile_commands.json");
            if candidate.is_file() { return Some(candidate) }
        }
    }
    None
}

/// Walk an argv vector and extract every `-I<path>` / `-isystem <path>`
/// directory, resolving relative paths against `dir`. Inserts canonicalized
/// PathBufs into `out`.
fn extract_include_paths(argv: &[String], dir: &Path, out: &mut HashSet<PathBuf>) {
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        // -I<path> (no space)
        if let Some(rest) = arg.strip_prefix("-I") {
            if !rest.is_empty() {
                push_path(rest, dir, out);
                i += 1; continue;
            }
            // -I <path>
            if i + 1 < argv.len() {
                push_path(&argv[i + 1], dir, out);
                i += 2; continue;
            }
        }
        // -isystem <path>
        if arg == "-isystem" {
            if i + 1 < argv.len() {
                push_path(&argv[i + 1], dir, out);
                i += 2; continue;
            }
        }
        // /I<path>  (MSVC-style, sometimes seen on Windows builds)
        if let Some(rest) = arg.strip_prefix("/I") {
            if !rest.is_empty() {
                push_path(rest, dir, out);
            }
        }
        i += 1;
    }
}

fn push_path(raw: &str, base: &Path, out: &mut HashSet<PathBuf>) {
    let p = Path::new(raw);
    let resolved = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
    out.insert(resolved);
}

/// Best-effort tokenizer for the `command` field. Splits on whitespace,
/// honoring single and double quotes. Doesn't expand shell variables —
/// CMake doesn't emit them in compile_commands.json, and Bear/intercept
/// produce already-resolved invocations.
fn tokenize_command(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if !in_single => {
                if let Some(&next) = chars.peek() {
                    current.push(next);
                    chars.next();
                }
            }
            '\'' if !in_double => { in_single = !in_single; }
            '"' if !in_single => { in_double = !in_double; }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() { out.push(current); }
    out
}

#[cfg(test)]
#[path = "compile_commands_tests.rs"]
mod tests;
