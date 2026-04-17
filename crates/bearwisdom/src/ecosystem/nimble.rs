// =============================================================================
// ecosystem/nimble.rs — Nimble ecosystem (Nim)
//
// Phase 2 + 3: consolidates `indexer/externals/nim.rs`. No separate
// manifest reader — .nimble file parsing lives here.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("nimble");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["nim"];
const LEGACY_ECOSYSTEM_TAG: &str = "nim";

pub struct NimbleEcosystem;

impl Ecosystem for NimbleEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("nim"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_nim_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_nim_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn resolve_import(
        &self, dep: &ExternalDepRoot, _p: &str, _s: &[&str],
    ) -> Vec<WalkedFile> { walk_nim_narrowed(dep) }
    fn resolve_symbol(
        &self, dep: &ExternalDepRoot, _f: &str,
    ) -> Vec<WalkedFile> { walk_nim_narrowed(dep) }
}

impl ExternalSourceLocator for NimbleEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_nim_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_nim_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NimbleEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NimbleEcosystem)).clone()
}

// ===========================================================================
// Discovery
// ===========================================================================

pub fn discover_nim_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = parse_nimble_requires(project_root);
    if declared.is_empty() { return Vec::new() }
    let Some(pkgs_dir) = find_nimble_pkgs_dir() else { return Vec::new() };

    let user_imports: Vec<String> = collect_nim_user_imports(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    let Ok(entries) = std::fs::read_dir(&pkgs_dir) else { return Vec::new() };
    let all_entries: Vec<_> = entries.flatten().collect();

    for dep_name in &declared {
        let prefix = format!("{dep_name}-");
        let mut matches: Vec<PathBuf> = all_entries
            .iter()
            .filter(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.starts_with(&prefix) && e.path().is_dir()
            })
            .map(|e| e.path())
            .collect();
        matches.sort();
        if let Some(best) = matches.pop() {
            let version = best
                .file_name().and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix(&prefix))
                .unwrap_or("").to_string();
            roots.push(ExternalDepRoot {
                module_path: dep_name.clone(),
                version,
                root: best,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: user_imports.clone(),
            });
        }
    }
    debug!("Nim: {} external package roots", roots.len());
    roots
}

// R3 — `import strutils` / `import std/strutils` / `import foo/[bar, baz]`
// scanner + narrowed walk. Stored as the leaf module name plus the dotted
// path; narrowing maps both to file tails (`strutils.nim` / `foo/bar.nim`).

fn collect_nim_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_nim_imports(project_root, &mut out, 0);
    out
}

fn scan_nim_imports(dir: &Path, out: &mut std::collections::HashSet<String>, depth: usize) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "nimcache" | "tests" | "test" | "examples")
                    || name.starts_with('.') { continue }
            }
            scan_nim_imports(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".nim") { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_nim_imports(&content, out);
        }
    }
}

fn extract_nim_imports(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        let rest = match line
            .strip_prefix("import ")
            .or_else(|| line.strip_prefix("from "))
        {
            Some(r) => r,
            None => continue,
        };
        // Strip trailing `; comment` pieces.
        let rest = rest.split('#').next().unwrap_or("").trim();
        // `from x import y` — only the `x` part matters.
        let rest = rest.split(" import ").next().unwrap_or("").trim();
        // Group form: `import foo/[a, b, c]`
        if let Some(open) = rest.find('[') {
            if let Some(close) = rest.find(']') {
                let prefix = rest[..open].trim_end_matches('/').trim();
                let inner = &rest[open + 1..close];
                for sel in inner.split(',') {
                    let sel = sel.trim();
                    if sel.is_empty() { continue }
                    out.insert(if prefix.is_empty() { sel.to_string() } else { format!("{prefix}/{sel}") });
                }
                continue;
            }
        }
        // Comma-separated: `import foo, bar`
        for part in rest.split(',') {
            let part = part.trim();
            if part.is_empty() { continue }
            // `foo as F` → drop alias
            let head = part.split(" as ").next().unwrap_or("").trim();
            if head.is_empty() { continue }
            out.insert(head.to_string());
        }
    }
}

fn nim_module_to_path_tail(module: &str) -> Option<String> {
    let cleaned = module.trim();
    if cleaned.is_empty() { return None }
    // `std/strutils` / `pkg/foo` / `foo` → all map to file paths.
    Some(format!("{}.nim", cleaned.replace('.', "/")))
}

fn walk_nim_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() { return walk_nim_root(dep); }
    let tails: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|m| nim_module_to_path_tail(m))
        .collect();
    if tails.is_empty() { return walk_nim_root(dep); }

    let mut out = Vec::new();
    walk_nim_narrowed_dir(&dep.root, &dep.root, dep, &tails, &mut out, 0);
    out
}

fn walk_nim_narrowed_dir(
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
                if matches!(name, "tests" | "test" | "examples" | "docs" | "nimcache") || name.starts_with('.') { continue }
            }
            subdirs.push(path);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".nim") { continue }
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
                relative_path: format!("ext:nim:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "nim",
            });
        }
    }
    for sub in subdirs {
        walk_nim_narrowed_dir(&sub, root, dep, tails, out, depth + 1);
    }
}

pub fn parse_nimble_requires(project_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new() };
    let nimble_file = entries
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("nimble"));
    let Some(entry) = nimble_file else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(entry.path()) else { return Vec::new() };

    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("requires") {
            for part in trimmed.split('"') {
                let dep = part.trim();
                if dep.is_empty() || dep.starts_with("requires") || dep == "," { continue }
                let name = dep
                    .split(|c: char| c == '>' || c == '<' || c == '=' || c == '#' || c == '@' || c.is_whitespace())
                    .next().unwrap_or("").trim();
                if !name.is_empty() && name != "nim"
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    if !deps.contains(&name.to_string()) {
                        deps.push(name.to_string());
                    }
                }
            }
        }
    }
    deps
}

fn find_nimble_pkgs_dir() -> Option<PathBuf> {
    if let Ok(nimble_dir) = std::env::var("NIMBLE_DIR") {
        let p = PathBuf::from(&nimble_dir).join("pkgs2");
        if p.is_dir() { return Some(p) }
        let p = PathBuf::from(nimble_dir).join("pkgs");
        if p.is_dir() { return Some(p) }
    }
    let home = dirs::home_dir()?;
    let pkgs2 = home.join(".nimble").join("pkgs2");
    if pkgs2.is_dir() { return Some(pkgs2) }
    let pkgs = home.join(".nimble").join("pkgs");
    if pkgs.is_dir() { return Some(pkgs) }
    None
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_nim_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
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
                if matches!(name, "tests" | "test" | "examples" | "docs" | "nimcache")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".nim") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:nim:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "nim",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let n = NimbleEcosystem;
        assert_eq!(n.id(), ID);
        assert_eq!(Ecosystem::kind(&n), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&n), &["nim"]);
    }

    #[test]
    fn legacy_locator_tag_is_nim() {
        assert_eq!(ExternalSourceLocator::ecosystem(&NimbleEcosystem), "nim");
    }

    #[test]
    fn nim_parses_nimble_requires() {
        let tmp = std::env::temp_dir().join("bw-test-nimble-parse");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("test.nimble"), r#"
requires "nim >= 2.0.0"
requires "jester#baca3f"
requires "karax#5cf360c"
"#).unwrap();
        let deps = parse_nimble_requires(&tmp);
        assert_eq!(deps, vec!["jester", "karax"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    #[test]
    fn nim_extract_imports_handles_group_form() {
        let mut out = std::collections::HashSet::new();
        extract_nim_imports(
            "import strutils\nimport std/strformat\nimport pkg/foo/[bar, baz]\nfrom os import getEnv\nimport other as O\n",
            &mut out,
        );
        assert!(out.contains("strutils"));
        assert!(out.contains("std/strformat"));
        assert!(out.contains("pkg/foo/bar"));
        assert!(out.contains("pkg/foo/baz"));
        assert!(out.contains("os"));
        assert!(out.contains("other"));
    }

    #[test]
    fn nim_module_to_path_tail_converts() {
        assert_eq!(nim_module_to_path_tail("strutils"), Some("strutils.nim".to_string()));
        assert_eq!(nim_module_to_path_tail("std/strutils"), Some("std/strutils.nim".to_string()));
    }
}
