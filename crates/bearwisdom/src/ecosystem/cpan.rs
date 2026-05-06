// =============================================================================
// ecosystem/cpan.rs — CPAN ecosystem (Perl)
//
// Phase 2 + 3: consolidates `indexer/externals/perl.rs`.
// No separate manifest reader — cpanfile parsing lives here.
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

pub const ID: EcosystemId = EcosystemId::new("cpan");
const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["perl"];
const LEGACY_ECOSYSTEM_TAG: &str = "perl";

pub struct CpanEcosystem;

impl Ecosystem for CpanEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }
    fn activation(&self) -> EcosystemActivation {
        // Project deps via `cpanfile`. A bare directory of `.pl`/`.pm`
        // files with no manifest can't be resolved against external
        // CPAN distributions — `local::lib` install paths only become
        // useful once cpanfile lists what to look for. Dropping the
        // LanguagePresent shotgun is correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }
    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_perl_externals(ctx.project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_perl_root(dep) }
    fn supports_reachability(&self) -> bool { true }
    fn resolve_import(
        &self, dep: &ExternalDepRoot, _p: &str, _s: &[&str],
    ) -> Vec<WalkedFile> { walk_perl_narrowed(dep) }
    fn resolve_symbol(
        &self, dep: &ExternalDepRoot, _f: &str,
    ) -> Vec<WalkedFile> { walk_perl_narrowed(dep) }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_perl_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for CpanEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_perl_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_perl_root(dep) }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CpanEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CpanEcosystem)).clone()
}

// ===========================================================================
// Manifest reader
// ===========================================================================

/// Surfaces `cpanfile` `requires` directives in
/// `ProjectContext.manifests[ManifestKind::Cpan]`.
pub struct CpanfileManifest;

impl crate::ecosystem::manifest::ManifestReader for CpanfileManifest {
    fn kind(&self) -> crate::ecosystem::manifest::ManifestKind {
        crate::ecosystem::manifest::ManifestKind::Cpan
    }

    fn read(&self, project_root: &Path) -> Option<crate::ecosystem::manifest::ManifestData> {
        let cpanfile = project_root.join("cpanfile");
        if !cpanfile.is_file() { return None }
        let content = std::fs::read_to_string(&cpanfile).ok()?;
        let deps = parse_cpanfile_requires(&content);
        if deps.is_empty() { return None }
        let mut data = crate::ecosystem::manifest::ManifestData::default();
        data.dependencies = deps.into_iter().collect();
        Some(data)
    }
}

pub fn discover_perl_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let cpanfile = project_root.join("cpanfile");
    if !cpanfile.is_file() { return Vec::new() }
    let Ok(content) = std::fs::read_to_string(&cpanfile) else { return Vec::new() };
    let declared = parse_cpanfile_requires(&content);
    if declared.is_empty() { return Vec::new() }

    let lib_dirs = perl_lib_dirs(project_root);
    if lib_dirs.is_empty() { return Vec::new() }

    let user_uses: Vec<String> = collect_perl_user_uses(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    for module_name in &declared {
        let path_fragment = module_name.replace("::", std::path::MAIN_SEPARATOR_STR);
        for lib in &lib_dirs {
            let module_dir = lib.join(&path_fragment);
            if module_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: module_name.clone(),
                    version: String::new(),
                    root: module_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: user_uses.clone(),
                });
                break;
            }
            let module_file = lib.join(format!("{path_fragment}.pm"));
            if module_file.is_file() {
                roots.push(ExternalDepRoot {
                    module_path: module_name.clone(),
                    version: String::new(),
                    root: module_file.parent().unwrap_or(lib).to_path_buf(),
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: user_uses.clone(),
                });
                break;
            }
        }
    }
    debug!("Perl: {} external module roots", roots.len());
    roots
}

fn collect_perl_user_uses(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_perl_uses(project_root, &mut out, 0);
    out
}

fn scan_perl_uses(dir: &Path, out: &mut std::collections::HashSet<String>, depth: usize) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "local" | "blib" | "t" | "xt") || name.starts_with('.') { continue }
            }
            scan_perl_uses(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".pm") || name.ends_with(".pl") || name.ends_with(".t")) { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            for raw in content.lines() {
                let line = raw.trim();
                let rest = match line.strip_prefix("use ").or_else(|| line.strip_prefix("require ")) {
                    Some(r) => r,
                    None => continue,
                };
                let head = rest
                    .split(|c: char| c == ';' || c == ' ' || c == '\t' || c == '(')
                    .next()
                    .unwrap_or("")
                    .trim();
                if head.is_empty() || !head.chars().next().map_or(false, |c| c.is_ascii_alphabetic()) { continue }
                if matches!(head, "strict" | "warnings" | "utf8" | "feature" | "lib" | "vars" | "constant") { continue }
                out.insert(head.to_string());
            }
        }
    }
}

fn perl_fqn_to_path_tail(fqn: &str) -> Option<String> {
    let cleaned = fqn.trim();
    if cleaned.is_empty() { return None }
    Some(format!("{}.pm", cleaned.replace("::", "/")))
}

fn walk_perl_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() { return walk_perl_root(dep); }
    let tails: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|f| perl_fqn_to_path_tail(f))
        .collect();
    if tails.is_empty() { return walk_perl_root(dep); }

    let mut out = Vec::new();
    walk_perl_narrowed_dir(&dep.root, &dep.root, dep, &tails, &mut out, 0);
    out
}

fn walk_perl_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    tails: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut dir_files: Vec<(PathBuf, String)> = Vec::new();
    let mut any_match = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "t" | "xt" | "blib" | "examples") || name.starts_with('.') { continue }
            }
            subdirs.push(path);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".pm") || name.ends_with(".pl")) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if tails.iter().any(|t| rel_sub.ends_with(t)) { any_match = true; }
            dir_files.push((path, rel_sub));
        }
    }

    if any_match {
        for (path, rel_sub) in dir_files {
            out.push(WalkedFile {
                relative_path: format!("ext:perl:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "perl",
            });
        }
    }
    for sub in subdirs {
        walk_perl_narrowed_dir(&sub, root, dep, tails, out, depth + 1);
    }
}

pub fn parse_cpanfile_requires(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') { continue }
        if trimmed.starts_with("requires") {
            let rest = trimmed["requires".len()..].trim();
            let name = rest.trim_start_matches(|c: char| c == '\'' || c == '"' || c.is_whitespace());
            if let Some(end) = name.find(|c: char| c == '\'' || c == '"' || c == ',' || c == ';') {
                let module = &name[..end];
                if !module.is_empty() && module != "perl" {
                    if !deps.contains(&module.to_string()) { deps.push(module.to_string()) }
                }
            }
        }
    }
    deps
}

fn perl_lib_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local = project_root.join("local").join("lib").join("perl5");
    if local.is_dir() { dirs.push(local) }
    for var in &["PERL5LIB", "PERL_LOCAL_LIB_ROOT"] {
        if let Ok(val) = std::env::var(var) {
            for p in val.split(if cfg!(windows) { ';' } else { ':' }) {
                let pb = PathBuf::from(p);
                if pb.is_dir() { dirs.push(pb) }
            }
        }
    }
    dirs
}

fn walk_perl_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "t" | "xt" | "blib" | "examples") || name.starts_with('.') { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".pm") || name.ends_with(".pl")) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:perl:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "perl",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------
//
// Perl has no tree-sitter grammar in this crate (ABI conflict). Line-based
// scan: `package Foo::Bar;` declarations and `sub name { ... }` definitions
// are captured. Names stored as simple segments (`Bar`) so `find_by_name`
// resolves typical `use Foo::Bar;` references.

fn build_perl_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_perl_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_perl_header(&src)
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

pub(crate) fn scan_perl_header(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("package ") {
            let pkg = rest
                .trim()
                .trim_end_matches(';')
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !pkg.is_empty() {
                out.push(pkg.clone());
                // Also store the last `::`-separated segment so `use Foo::Bar;`
                // can resolve via find_by_name("Bar").
                if let Some(last) = pkg.rsplit("::").next() {
                    if last != pkg { out.push(last.to_string()) }
                }
            }
        } else if let Some(rest) = t.strip_prefix("sub ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        assert_eq!(CpanEcosystem.id(), ID);
        assert_eq!(Ecosystem::languages(&CpanEcosystem), &["perl"]);
    }

    #[test]
    fn perl_parses_cpanfile() {
        let content = r#"
requires 'perl', 5.014000;
requires 'Carp';
requires 'Clone';
requires 'Config::Any';
requires 'Data::Censor' => '0.04';
"#;
        let deps = parse_cpanfile_requires(content);
        assert_eq!(deps, vec!["Carp", "Clone", "Config::Any", "Data::Censor"]);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    #[test]
    fn perl_fqn_to_path_tail_converts_colons() {
        assert_eq!(perl_fqn_to_path_tail("Foo::Bar"), Some("Foo/Bar.pm".to_string()));
        assert_eq!(perl_fqn_to_path_tail("Carp"), Some("Carp.pm".to_string()));
    }

    #[test]
    fn perl_narrowed_walk_excludes_unreferenced() {
        let tmp = std::env::temp_dir().join("bw-test-cpan-r3");
        let _ = std::fs::remove_dir_all(&tmp);
        let dep_root = tmp.join("Carp");
        std::fs::create_dir_all(&dep_root).unwrap();
        std::fs::write(dep_root.join("Carp.pm"), "package Carp; 1;\n").unwrap();
        std::fs::write(dep_root.join("Heavy.pm"), "package Carp::Heavy; 1;\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "Carp".to_string(),
            version: String::new(),
            root: dep_root.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: vec!["Carp".to_string()],
        };
        let files = walk_perl_narrowed(&dep);
        // Sibling rule walks both Carp.pm and Heavy.pm since they're in
        // the same directory and Carp.pm matched.
        assert_eq!(files.len(), 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
