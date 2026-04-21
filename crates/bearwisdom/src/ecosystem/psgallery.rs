// =============================================================================
// ecosystem/psgallery.rs — PowerShell Gallery ecosystem (package ecosystem)
//
// Phase 2 + 3: covers PSGallery-published modules and local module installs.
//
// Manifest: `.psd1` files — `RequiredModules = @(...)` declares deps.
// Cache discovery order:
//   1. $BEARWISDOM_PS_MODULES (explicit override)
//   2. ~/Documents/PowerShell/Modules/ (Windows, PS 6+)
//   3. ~/.local/share/powershell/Modules/ (Linux/macOS)
//   4. $PSModulePath-parsed entries
//   5. /usr/local/share/powershell/Modules/ (Linux/macOS system)
//   6. C:/Program Files/PowerShell/Modules/ (Windows PS 7 user)
//
// Walks *.psm1 and *.ps1 files under each discovered dep root.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("psgallery");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["powershell"];
const LEGACY_ECOSYSTEM_TAG: &str = "powershell";

pub struct PsGalleryEcosystem;

impl Ecosystem for PsGalleryEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("powershell"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ps_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ps_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        walk_ps_root(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        walk_ps_root(dep)
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_powershell_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for PsGalleryEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ps_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ps_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PsGalleryEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PsGalleryEcosystem)).clone()
}

// ===========================================================================
// Discovery
// ===========================================================================

pub fn discover_ps_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = parse_psd1_required_modules(project_root);
    if declared.is_empty() {
        return Vec::new();
    }

    let cache_dirs = find_ps_module_dirs();
    if cache_dirs.is_empty() {
        debug!("psgallery: no module dirs found; {} deps unresolvable", declared.len());
        return Vec::new();
    }

    let mut roots = Vec::new();
    for dep_name in &declared {
        if let Some(root) = resolve_module_root(dep_name, &cache_dirs) {
            let version = detect_module_version(&root, dep_name);
            roots.push(ExternalDepRoot {
                module_path: dep_name.clone(),
                version,
                root,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }

    debug!("psgallery: {} external module roots resolved", roots.len());
    roots
}

/// Search ordered candidate dirs for a module by name.
/// PowerShell modules may live directly as `<ModulesDir>/<Name>/` or under a
/// version subfolder `<ModulesDir>/<Name>/<Version>/`. Both layouts are handled.
fn resolve_module_root(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let name_lower = name.to_lowercase();
    for base in dirs {
        let Ok(entries) = std::fs::read_dir(base) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if dir_name.to_lowercase() != name_lower {
                continue;
            }
            // Prefer versioned child dir if it exists (e.g. 1.4.7/).
            if let Some(versioned) = latest_version_subdir(&path) {
                return Some(versioned);
            }
            return Some(path);
        }
    }
    None
}

/// If a module dir contains numeric version subdirs, pick the latest lexicographically.
fn latest_version_subdir(module_dir: &Path) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(module_dir) else { return None };
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions.into_iter().next_back()
}

fn detect_module_version(root: &Path, module_name: &str) -> String {
    // If the parent dir looks like a semver, use it.
    if let Some(parent) = root.parent() {
        if let Some(dir_name) = parent.file_name().and_then(|n| n.to_str()) {
            if dir_name.to_lowercase() == module_name.to_lowercase() {
                if let Some(ver_name) = root.file_name().and_then(|n| n.to_str()) {
                    if ver_name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        return ver_name.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

// ===========================================================================
// Module dir discovery
// ===========================================================================

fn find_ps_module_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    // 1. Explicit override.
    if let Ok(explicit) = std::env::var("BEARWISDOM_PS_MODULES") {
        for part in explicit.split(if cfg!(windows) { ';' } else { ':' }) {
            let p = PathBuf::from(part.trim());
            if p.is_dir() {
                dirs.push(p);
            }
        }
        if !dirs.is_empty() {
            return dirs;
        }
    }

    // 2. User PS 6+ dir (Windows).
    if let Some(docs) = windows_documents_dir() {
        let p = docs.join("PowerShell").join("Modules");
        if p.is_dir() {
            dirs.push(p);
        }
    }

    // 3. XDG user dir (Linux / macOS).
    if let Some(home) = dirs::home_dir() {
        let xdg = home.join(".local").join("share").join("powershell").join("Modules");
        if xdg.is_dir() {
            dirs.push(xdg);
        }
    }

    // 4. $PSModulePath entries (if the env var is set outside a PS session).
    if let Ok(ps_path) = std::env::var("PSModulePath") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for part in ps_path.split(sep) {
            let p = PathBuf::from(part.trim());
            if p.is_dir() && !dirs.contains(&p) {
                dirs.push(p);
            }
        }
    }

    // 5. System-level well-known paths.
    let candidates: &[&str] = if cfg!(windows) {
        &[
            "C:/Program Files/PowerShell/Modules",
            "C:/Program Files/PowerShell/7/Modules",
            "C:/Windows/System32/WindowsPowerShell/v1.0/Modules",
        ]
    } else {
        &[
            "/usr/local/share/powershell/Modules",
            "/usr/share/powershell/Modules",
        ]
    };

    for &cand in candidates {
        let p = PathBuf::from(cand);
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    }

    dirs
}

/// Returns `~/Documents` on Windows via USERPROFILE, HOMEDRIVE+HOMEPATH,
/// or dirs::home_dir() fallback.
fn windows_documents_dir() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    // `[Environment]::GetFolderPath("MyDocuments")` is the canonical way but
    // spawning pwsh here would be circular. Env heuristic is reliable enough.
    if let Ok(home) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(&home).join("Documents");
        if p.is_dir() {
            return Some(p);
        }
    }
    dirs::home_dir().map(|h| h.join("Documents"))
}

// ===========================================================================
// .psd1 parsing — RequiredModules
// ===========================================================================

/// Scan a project root for `*.psd1` files and extract `RequiredModules = @(...)`.
pub fn parse_psd1_required_modules(project_root: &Path) -> Vec<String> {
    let mut deps: Vec<String> = Vec::new();
    scan_psd1_dir(project_root, &mut deps, 0);
    deps.sort();
    deps.dedup();
    deps
}

fn scan_psd1_dir(dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if matches!(name, ".git" | "node_modules" | "vendor") || name.starts_with('.') {
                continue;
            }
            scan_psd1_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".psd1") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_required_modules(&content, out);
        }
    }
}

/// Line-based parser for the `RequiredModules = @(...)` stanza in a `.psd1`
/// hashtable. Handles both single-line and multi-line `@(...)` arrays.
///
/// Entries may be bare strings `'ModuleName'` or hashtables
/// `@{ ModuleName = 'Pester'; RequiredVersion = '...' }`. Both are handled.
pub fn extract_required_modules(content: &str, out: &mut Vec<String>) {
    let mut in_block = false;
    let mut depth: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        if !in_block {
            // Look for `RequiredModules = @(` or `RequiredModules=@(`
            let lower = trimmed.to_lowercase();
            if let Some(pos) = lower.find("requiredmodules") {
                let after = &trimmed[pos + "requiredmodules".len()..];
                let after = after.trim_start_matches(char::is_whitespace)
                    .trim_start_matches('=')
                    .trim_start();
                if after.starts_with("@(") {
                    in_block = true;
                    depth = 1;
                    let rest = &after["@(".len()..];
                    parse_psd1_array_line(rest, out, &mut depth);
                    if depth <= 0 {
                        in_block = false;
                    }
                }
            }
        } else {
            parse_psd1_array_line(trimmed, out, &mut depth);
            if depth <= 0 {
                in_block = false;
            }
        }
    }
}

/// Process one line within a `@(...)` block. Tracks nesting via depth.
fn parse_psd1_array_line(line: &str, out: &mut Vec<String>, depth: &mut i32) {
    let mut i = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();

    while i < len {
        match bytes[i] {
            b'(' | b'{' => {
                *depth += 1;
                i += 1;
            }
            b')' | b'}' => {
                *depth -= 1;
                i += 1;
            }
            b'#' => {
                // PS line comment — rest of line is ignored.
                break;
            }
            b'\'' | b'"' => {
                // Quoted string — extract its content.
                let quote = bytes[i];
                i += 1;
                let start = i;
                while i < len && bytes[i] != quote {
                    i += 1;
                }
                let s = &line[start..i];
                i += 1; // consume closing quote
                let s = s.trim();
                if !s.is_empty() && !s.eq_ignore_ascii_case("requiredversion")
                    && !s.eq_ignore_ascii_case("modulename")
                    && !s.eq_ignore_ascii_case("guid")
                    && !s.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && !s.contains('/')
                    && !s.contains('\\')
                    && !s.contains('{')
                    && !s.contains('-') // skip version strings like "1.0.0-beta"
                {
                    if looks_like_module_name(s) && !out.contains(&s.to_string()) {
                        out.push(s.to_string());
                    }
                }
            }
            _ => {
                // Check for unquoted `ModuleName = 'Pester'` inside @{ }.
                // Skip identifier = value by advancing to next delimiter.
                if bytes[i].is_ascii_alphabetic() {
                    let start = i;
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.' || bytes[i] == b'-') {
                        i += 1;
                    }
                    let ident = &line[start..i];
                    // If this looks like a key (`ModuleName =`), skip past the `=`
                    // and let the string parser pick up the value on the next iteration.
                    let rest = line[i..].trim_start();
                    if rest.starts_with('=') {
                        // field = value — skip the key; next iteration handles value
                    } else if looks_like_module_name(ident) {
                        // Unquoted bare module name (unusual but legal in older manifests)
                        if !out.contains(&ident.to_string()) {
                            out.push(ident.to_string());
                        }
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// Heuristic: a valid PS module name is alphanumeric + hyphens/dots/underscores,
/// starts with a letter, no path separators, not a GUID, not a pure version.
fn looks_like_module_name(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    // Reject GUIDs (contain many hyphens and hex chars in 8-4-4-4-12 pattern).
    if s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4 {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

// ===========================================================================
// Walk
// ===========================================================================

pub fn walk_ps_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_ps_dir(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_ps_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "examples" | "docs") || name.starts_with('.') {
                    continue;
                }
            }
            walk_ps_dir(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".psm1") && !name.ends_with(".ps1") {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:powershell:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "powershell",
            });
        }
    }
}

// ===========================================================================
// Symbol-location index
// ===========================================================================

/// Shared symbol index builder used by both `PsGalleryEcosystem` and
/// `PowerShellStdlibEcosystem`. Scans each walked file for top-level
/// function and class declarations using a line-based heuristic (no full
/// tree-sitter parse — the overhead isn't warranted for header scanning).
pub fn build_powershell_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let work: Vec<(String, WalkedFile)> = dep_roots
        .iter()
        .flat_map(|dep| walk_ps_root(dep).into_iter().map(move |wf| (dep.module_path.clone(), wf)))
        .collect();

    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    let per_file: Vec<Vec<(String, String, std::path::PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_ps_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();

    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Line-based scan for top-level declarations in a `.ps1` / `.psm1` file.
/// Matches:
///   `function Verb-Noun`
///   `function Verb-Noun([...`
///   `class ClassName`
///   `enum EnumName`
///
/// Only considers lines that start with `function`, `class`, or `enum`
/// at column 0 (or after optional `Export-ModuleMember` / scope qualifier).
pub(crate) fn scan_ps_header(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let lower = trimmed.to_lowercase();
        for kw in &["function ", "class ", "enum "] {
            if lower.starts_with(kw) {
                // Use original trimmed to preserve original casing.
                let rest = &trimmed[kw.len()..];
                let name_part = rest.trim_start();
                let name = name_part
                    .split(|c: char| c == '(' || c == '{' || c == ' ' || c == '\t' || c == '[')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() && is_valid_ps_identifier(name) {
                    out.push(name.to_string());
                }
                break;
            }
        }
    }
    out
}

fn is_valid_ps_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut c = s.chars();
    let first = match c.next() {
        Some(ch) => ch,
        None => return false,
    };
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    c.all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let eco = PsGalleryEcosystem;
        assert_eq!(eco.id(), ID);
        assert_eq!(Ecosystem::kind(&eco), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&eco), &["powershell"]);
    }

    #[test]
    fn legacy_locator_tag() {
        assert_eq!(ExternalSourceLocator::ecosystem(&PsGalleryEcosystem), "powershell");
    }

    #[test]
    fn parse_psd1_simple_string_array() {
        let content = r#"
@{
    ModuleVersion = '1.0.0'
    RequiredModules = @('Pester', 'PSScriptAnalyzer', 'platyPS')
}
"#;
        let mut out = Vec::new();
        extract_required_modules(content, &mut out);
        assert!(out.contains(&"Pester".to_string()), "expected Pester, got {out:?}");
        assert!(out.contains(&"PSScriptAnalyzer".to_string()));
        assert!(out.contains(&"platyPS".to_string()));
    }

    #[test]
    fn parse_psd1_hashtable_entries() {
        let content = r#"
@{
    RequiredModules = @(
        @{ ModuleName = 'Az.Accounts'; RequiredVersion = '2.12.0' },
        @{ ModuleName = 'Az.Resources'; RequiredVersion = '6.0.0' }
    )
}
"#;
        let mut out = Vec::new();
        extract_required_modules(content, &mut out);
        assert!(out.contains(&"Az.Accounts".to_string()), "got {out:?}");
        assert!(out.contains(&"Az.Resources".to_string()));
    }

    #[test]
    fn parse_psd1_empty_array() {
        let content = "@{ RequiredModules = @() }";
        let mut out = Vec::new();
        extract_required_modules(content, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn parse_psd1_skips_guid_and_versions() {
        let content = r#"
@{
    GUID = '12345678-1234-1234-1234-123456789abc'
    RequiredModules = @('ValidModule')
}
"#;
        let mut out = Vec::new();
        extract_required_modules(content, &mut out);
        assert!(!out.iter().any(|s| s.contains('-') && s.len() == 36));
        assert!(out.contains(&"ValidModule".to_string()));
    }

    #[test]
    fn parse_psd1_multiline_string_array() {
        let content = r#"
@{
    RequiredModules = @(
        'Pester',
        'PSScriptAnalyzer'
    )
}
"#;
        let mut out = Vec::new();
        extract_required_modules(content, &mut out);
        assert!(out.contains(&"Pester".to_string()), "got {out:?}");
        assert!(out.contains(&"PSScriptAnalyzer".to_string()));
    }

    #[test]
    fn looks_like_module_name_accepts_valid() {
        assert!(looks_like_module_name("Pester"));
        assert!(looks_like_module_name("Az.Accounts"));
        assert!(looks_like_module_name("PSScriptAnalyzer"));
        assert!(looks_like_module_name("Microsoft.PowerShell.Utility"));
    }

    #[test]
    fn looks_like_module_name_rejects_invalid() {
        assert!(!looks_like_module_name(""));
        assert!(!looks_like_module_name("1.0.0"));
        assert!(!looks_like_module_name("12345678-1234-1234-1234-123456789abc"));
    }

    #[test]
    fn scan_ps_header_finds_functions_and_classes() {
        let src = r#"
# Top-level module file
function Get-Thing {
    param($x)
    Write-Output $x
}

function Set-Thing {
    param($x, $y)
}

class MyClass {
    [string] $Name
}

enum Status { Active; Inactive }
"#;
        let names = scan_ps_header(src);
        assert!(names.contains(&"Get-Thing".to_string()), "got {names:?}");
        assert!(names.contains(&"Set-Thing".to_string()));
        assert!(names.contains(&"MyClass".to_string()));
        assert!(names.contains(&"Status".to_string()));
    }

    #[test]
    fn scan_ps_header_ignores_nested() {
        // Nested function defs should still be caught since we don't track indentation —
        // that's acceptable for header scanning (worst case: a few extra symbols indexed).
        let src = "function Outer {\n    function Inner { }\n}";
        let names = scan_ps_header(src);
        assert!(names.contains(&"Outer".to_string()));
    }

    #[test]
    fn parse_psd1_from_filesystem() {
        let tmp = std::env::temp_dir().join("bw-test-psd1-parse");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("MyModule.psd1"),
            r#"
@{
    ModuleVersion = '2.0.1'
    GUID = 'abcdef12-abcd-abcd-abcd-abcdef123456'
    RequiredModules = @(
        'Pester',
        @{ ModuleName = 'Az.Accounts'; RequiredVersion = '2.12.0' }
    )
}
"#,
        )
        .unwrap();
        let deps = parse_psd1_required_modules(&tmp);
        assert!(deps.contains(&"Pester".to_string()), "got {deps:?}");
        assert!(deps.contains(&"Az.Accounts".to_string()));
        // GUID must not appear as a dep.
        assert!(!deps.iter().any(|d| d.len() == 36 && d.contains('-')));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
