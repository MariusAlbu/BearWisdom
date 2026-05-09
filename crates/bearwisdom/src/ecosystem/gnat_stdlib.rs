// =============================================================================
// ecosystem/gnat_stdlib.rs — GNAT Ada runtime (stdlib ecosystem)
//
// Every Ada project transitively pulls `Ada.*`, `GNAT.*`, `Interfaces.*`, and
// `System.*` from the GNAT compiler's `adainclude/` directory. This walker
// locates that directory and exposes the `.ads` (specification) files via
// the demand-driven parse path so a project that `with`s `Ada.Text_IO` only
// pulls the matching file, not the full ~900-file runtime.
//
// Probe order (degrades to empty when nothing found — no synthetics, no
// hardcoded API lists):
//   1. $BEARWISDOM_GNAT_LIBDIR — explicit override (path containing
//      `adainclude/` directly, or a `lib/gcc/<triplet>/<ver>/` ancestor).
//   2. `gnatls -v` — the canonical "where is my Ada source" query. Parses
//      the "Source Search Path:" block and emits each entry that looks like
//      an `adainclude` directory.
//   3. Alire-managed toolchains under
//      `<LOCALAPPDATA>/alire/cache/toolchains/gnat_*/` (Windows) or
//      `~/.cache/alire/toolchains/gnat_*/` (Linux/macOS) — Alire is the
//      most common Ada install path on this machine.
//   4. MSYS2 / mingw-w64 GCC tree under
//      `<msys-root>/mingw64/lib/gcc/<triplet>/<ver>/adainclude/` when the
//      user has gnat-mingw installed.
//
// File naming: GNAT applies the "krunch" convention — `Ada.Text_IO` lives
// in `a-textio.ads`, `Interfaces.C.Strings` in `i-cstrin.ads`. We do NOT
// reverse-engineer that mapping. Instead the symbol-index builder reads
// each `.ads` file's `package <Name> is` (or `package body <Name> is`)
// and indexes the qualified name it actually declares — robust to the
// renaming convention and to non-canonical files.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("gnat-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "gnat-stdlib";
const LANGUAGES: &[&str] = &["ada"];

pub struct GnatStdlibEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for GnatStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // Every Ada project unconditionally needs `Ada.*` / `GNAT.*` /
        // `Interfaces.*` / `System.*`. Substrate exception per the
        // ecosystem authoring guide.
        EcosystemActivation::LanguagePresent("ada")
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_gnat_adainclude()
    }

    // Demand-driven: no eager walk. The 910-file runtime is wasteful to
    // parse for projects that only `with` a handful of units.
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }
    fn is_workspace_global(&self) -> bool { true }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_gnat_stdlib_symbol_index(dep_roots)
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_package(dep, package).into_iter().collect()
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        // Strip trailing children: a request for `Ada.Text_IO.Put_Line`
        // must locate the `Ada.Text_IO` spec file.
        let mut probe = fqn.to_string();
        while !probe.is_empty() {
            if let Some(walked) = resolve_package(dep, &probe) {
                return vec![walked];
            }
            match probe.rfind('.') {
                Some(idx) => probe.truncate(idx),
                None => break,
            }
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl (back-compat bridge)
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for GnatStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_gnat_adainclude()
    }
    fn locate_roots_for_package(
        &self,
        _workspace_root: &Path,
        _package_abs_path: &Path,
        _package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        // Workspace-global: same roots regardless of which package asks.
        discover_gnat_adainclude()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GnatStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GnatStdlibEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_gnat_adainclude() -> Vec<ExternalDepRoot> {
    let mut out: Vec<PathBuf> = Vec::new();

    // 1. Explicit override.
    if let Some(explicit) = std::env::var_os("BEARWISDOM_GNAT_LIBDIR") {
        let p = PathBuf::from(explicit);
        // Accept either a direct adainclude/ pointer or a parent that
        // contains one.
        if p.is_dir() {
            if p.file_name().and_then(|n| n.to_str()) == Some("adainclude") {
                out.push(p);
            } else if let Some(found) = find_adainclude_under(&p) {
                out.push(found);
            }
        }
    }

    // 2. `gnatls -v` — canonical query.
    if out.is_empty() {
        out.extend(probe_gnatls());
    }

    // 3. Alire-managed toolchains.
    if out.is_empty() {
        out.extend(probe_alire_toolchains());
    }

    // 4. MSYS2 / mingw-w64 fallback (Windows only).
    #[cfg(target_os = "windows")]
    if out.is_empty() {
        out.extend(probe_msys2_mingw());
    }

    // De-duplicate and emit ExternalDepRoots.
    out.sort();
    out.dedup();

    if out.is_empty() {
        debug!("gnat-stdlib: no adainclude directory probed");
        return Vec::new();
    }

    out.into_iter().map(make_root).collect()
}

fn make_root(dir: PathBuf) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "gnat-stdlib".to_string(),
        version: String::new(),
        root: dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// Run `gnatls -v` and parse the "Source Search Path:" section.
fn probe_gnatls() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for program in ["gnatls", "gnatls.exe"] {
        let Ok(out) = Command::new(program).arg("-v").output() else { continue };
        if !out.status.success() { continue }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut in_source = false;
        for raw in text.lines() {
            let line = raw.trim();
            if line.starts_with("Source Search Path:") {
                in_source = true;
                continue;
            }
            if line.starts_with("Object Search Path:") || line.starts_with("Project Search Path:") {
                in_source = false;
            }
            if !in_source { continue }
            if line.is_empty() || line == "<Current_Directory>" { continue }
            let p = PathBuf::from(line);
            if p.is_dir() && p.file_name().and_then(|n| n.to_str()) == Some("adainclude") {
                found.push(p);
            }
        }
        if !found.is_empty() { break }
    }
    found
}

fn probe_alire_toolchains() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        bases.push(
            PathBuf::from(local)
                .join("alire")
                .join("cache")
                .join("toolchains"),
        );
    }
    if let Some(home) = dirs::home_dir() {
        bases.push(home.join(".cache").join("alire").join("toolchains"));
        bases.push(
            home.join("Library")
                .join("Caches")
                .join("alire")
                .join("toolchains"),
        );
    }

    let mut out = Vec::new();
    for base in &bases {
        if !base.is_dir() { continue }
        let Ok(entries) = std::fs::read_dir(base) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // Alire crate-cache dirs look like `gnat_native_15.2.1_<hash>`
            // or `gnat_arm_elf_<ver>_<hash>`.
            if !name.starts_with("gnat") || !path.is_dir() { continue }
            if let Some(adainclude) = find_adainclude_under(&path) {
                out.push(adainclude);
            }
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn probe_msys2_mingw() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(prefix) = std::env::var_os("MSYSTEM_PREFIX") {
        if let Some(found) = find_adainclude_under(&PathBuf::from(prefix)) {
            out.push(found);
        }
    }
    for env in ["MSYS2_ROOT", "MINGW_PREFIX", "MINGW_ROOT", "MINGW_HOME"] {
        if let Some(p) = std::env::var_os(env) {
            for sub in ["mingw64", "ucrt64", "clang64", ""] {
                let cand = if sub.is_empty() {
                    PathBuf::from(&p)
                } else {
                    PathBuf::from(&p).join(sub)
                };
                if let Some(found) = find_adainclude_under(&cand) {
                    out.push(found);
                }
            }
        }
    }
    out
}

/// Walk down from `dir` looking for `lib/gcc/<triplet>/<ver>/adainclude`.
/// Bounded depth — the GCC layout never buries adainclude deeper than 5
/// levels below the install root.
fn find_adainclude_under(dir: &Path) -> Option<PathBuf> {
    walk_for_adainclude(dir, 0)
}

fn walk_for_adainclude(dir: &Path, depth: u32) -> Option<PathBuf> {
    if depth > 5 { return None }
    if !dir.is_dir() { return None }
    if dir.file_name().and_then(|n| n.to_str()) == Some("adainclude") {
        return Some(dir.to_path_buf());
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return None };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue }
        let path = entry.path();
        // Cheap pruning: stay on lib/, gcc/, triplet/, version/, adainclude/.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if matches!(name, "share" | "doc" | "info" | "man" | "include" | "bin" | "libexec") {
                continue;
            }
        }
        if let Some(found) = walk_for_adainclude(&path, depth + 1) {
            return Some(found);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Demand-driven resolution
// ---------------------------------------------------------------------------

/// Locate the `.ads` file that declares the given package qualified name.
fn resolve_package(dep: &ExternalDepRoot, package: &str) -> Option<WalkedFile> {
    let needle = package.to_ascii_lowercase();
    let mut hit: Option<PathBuf> = None;
    let _ = walk_adainclude(&dep.root, &mut |path| {
        if let Some(decl) = scan_package_decl(path) {
            if decl.eq_ignore_ascii_case(&needle) {
                hit = Some(path.to_path_buf());
                return false; // stop walking
            }
        }
        true
    });
    let path = hit?;
    Some(make_walked_file(dep, &path))
}

fn make_walked_file(dep: &ExternalDepRoot, path: &Path) -> WalkedFile {
    let rel = path
        .strip_prefix(&dep.root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    WalkedFile {
        relative_path: format!("ext:gnat-stdlib:{rel}"),
        absolute_path: path.to_path_buf(),
        language: "ada",
    }
}

fn walk_adainclude<F>(dir: &Path, on_file: &mut F) -> bool
where
    F: FnMut(&Path) -> bool,
{
    walk_adainclude_inner(dir, on_file, 0)
}

fn walk_adainclude_inner<F>(dir: &Path, on_file: &mut F, depth: u32) -> bool
where
    F: FnMut(&Path) -> bool,
{
    if depth >= MAX_WALK_DEPTH { return true }
    let Ok(entries) = std::fs::read_dir(dir) else { return true };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if !walk_adainclude_inner(&path, on_file, depth + 1) { return false }
        } else if ft.is_file() {
            if path.extension().and_then(|e| e.to_str()) != Some("ads") { continue }
            if !on_file(&path) { return false }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Symbol index — scan each .ads for its `package <Name>` declaration
// ---------------------------------------------------------------------------

pub(crate) fn build_gnat_stdlib_symbol_index(
    dep_roots: &[ExternalDepRoot],
) -> SymbolLocationIndex {
    // Collect every .ads file across the dep roots.
    let mut files: Vec<PathBuf> = Vec::new();
    for dep in dep_roots {
        let mut found = Vec::new();
        let _ = walk_adainclude(&dep.root, &mut |path| {
            found.push(path.to_path_buf());
            true
        });
        files.extend(found);
    }

    if files.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Header-only scan in parallel — extracts the package's qualified name
    // from the first relevant declaration. No tree-sitter overhead: Ada
    // package decls are line-stable and always start with `package`.
    let pairs: Vec<(String, PathBuf)> = files
        .par_iter()
        .filter_map(|path| {
            scan_package_decl(path).map(|qname| (qname, path.clone()))
        })
        .collect();

    let mut index = SymbolLocationIndex::new();
    for (qname, file) in pairs {
        // Index under the canonical case (lowercase) so `with Ada.Text_IO;`
        // and `with ada.text_io;` both resolve.
        let key = qname.to_ascii_lowercase();
        index.insert("gnat-stdlib", key.clone(), file.clone());
        // Also register the original-case form so case-preserving lookups
        // hit without normalization.
        if key != qname {
            index.insert("gnat-stdlib", qname, file);
        }
    }
    index
}

/// Read an `.ads` file and return the qualified name of the package it
/// declares. Recognises:
///
/// ```text
/// package Foo.Bar is
/// package Foo.Bar
/// private package Foo.Bar is
/// generic package Foo.Bar is
/// package body Foo.Bar is
/// ```
///
/// Comments (`--`), pragmas, generic formal parameters, and `with`
/// clauses preceding the decl are skipped.
fn scan_package_decl(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for raw in content.lines() {
        let line = strip_ada_comment(raw).trim();
        if line.is_empty() { continue }
        // Strip leading `private` / `generic` qualifiers.
        let mut tail = line;
        for prefix in ["private ", "generic "] {
            if let Some(rest) = tail.strip_prefix(prefix) {
                tail = rest.trim_start();
            }
        }
        let after_kw = if let Some(r) = tail.strip_prefix("package body ") {
            r
        } else if let Some(r) = tail.strip_prefix("package ") {
            r
        } else {
            continue;
        };
        let qname: String = after_kw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_')
            .collect();
        if qname.is_empty() { continue }
        return Some(qname);
    }
    None
}

fn strip_ada_comment(line: &str) -> &str {
    match line.find("--") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "gnat_stdlib_tests.rs"]
mod tests;
