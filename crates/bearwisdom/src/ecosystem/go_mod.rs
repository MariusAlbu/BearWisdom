// =============================================================================
// ecosystem/go_mod.rs — Go module ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/go.rs` +
// `indexer/manifest/go_mod.rs`. Go's module cache lives at
// `$GOMODCACHE/{escaped_module_path}@{version}`; indirect deps are walked
// only when a lightweight source scan confirms user code imports them.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("go-mod");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["go"];
const LEGACY_ECOSYSTEM_TAG: &str = "go";

pub struct GoModEcosystem;

impl Ecosystem for GoModEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("go"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_go_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_root(dep)
    }
}

impl ExternalSourceLocator for GoModEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_go_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GoModEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GoModEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (migrated from indexer/manifest/go_mod.rs)
// ===========================================================================

pub struct GoModManifest;

impl ManifestReader for GoModManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::GoMod }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let go_mod_path = find_go_mod(project_root)?;
        let content = std::fs::read_to_string(&go_mod_path).ok()?;
        let parsed = parse_go_mod(&content);
        let mut data = ManifestData::default();
        data.module_path = parsed.module_path;
        for path in parsed.require_paths {
            data.dependencies.insert(path);
        }
        Some(data)
    }
}

pub struct GoModData {
    pub module_path: Option<String>,
    pub require_paths: Vec<String>,
    pub require_deps: Vec<GoModDep>,
}

#[derive(Debug, Clone)]
pub struct GoModDep {
    pub path: String,
    pub version: String,
    pub indirect: bool,
}

pub fn find_go_mod(root: &Path) -> Option<PathBuf> {
    let candidate = root.join("go.mod");
    if candidate.is_file() { return Some(candidate) }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let nested = entry.path().join("go.mod");
                if nested.is_file() { return Some(nested) }
            }
        }
    }
    None
}

pub fn parse_go_mod(content: &str) -> GoModData {
    let mut module_path: Option<String> = None;
    let mut require_paths = Vec::new();
    let mut require_deps = Vec::new();
    let mut in_require_block = false;

    fn parse_dep(fragment: &str) -> Option<GoModDep> {
        let without_comment = fragment.trim();
        let (main, comment) = match without_comment.find("//") {
            Some(idx) => (without_comment[..idx].trim(), &without_comment[idx..]),
            None => (without_comment, ""),
        };
        let mut tokens = main.split_whitespace();
        let path = tokens.next()?.to_string();
        let version = tokens.next()?.to_string();
        let indirect = comment.contains("indirect");
        Some(GoModDep { path, version, indirect })
    }

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") { continue }
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let path = rest.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() { module_path = Some(path.to_string()) }
            continue;
        }
        if trimmed == "require (" || trimmed.starts_with("require (") {
            in_require_block = true;
            continue;
        }
        if trimmed == ")" { in_require_block = false; continue }
        if let Some(rest) = trimmed.strip_prefix("require ") {
            let rest = rest.trim();
            if rest != "(" && !rest.is_empty() {
                if let Some(dep) = parse_dep(rest) {
                    require_paths.push(dep.path.clone());
                    require_deps.push(dep);
                }
            }
            continue;
        }
        if in_require_block && !trimmed.starts_with("//") {
            if let Some(dep) = parse_dep(trimmed) {
                require_paths.push(dep.path.clone());
                require_deps.push(dep);
            }
        }
    }

    GoModData { module_path, require_paths, require_deps }
}

// ===========================================================================
// Discovery — $GOMODCACHE / GOPATH / ~/go/pkg/mod
// ===========================================================================

pub fn discover_go_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(go_mod_path) = find_go_mod(project_root) else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(&go_mod_path) else { return Vec::new() };
    let parsed = parse_go_mod(&content);

    let cache_root = match gomodcache_root() {
        Some(p) => p,
        None => {
            debug!("No GOMODCACHE / GOPATH detected; skipping Go externals");
            return Vec::new();
        }
    };

    let user_imports = collect_go_imports(project_root);

    let mut roots = Vec::new();
    for dep in &parsed.require_deps {
        if dep.indirect && !go_dep_is_imported(&dep.path, &user_imports) { continue }
        if let Some(root) = resolve_go_dep_path(&cache_root, dep) {
            roots.push(ExternalDepRoot {
                module_path: dep.path.clone(),
                version: dep.version.clone(),
                root,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
            });
        } else {
            debug!(
                "Go module cache miss for {}@{} — not found under {}",
                dep.path,
                dep.version,
                cache_root.display()
            );
        }
    }
    roots
}

fn collect_go_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut imports: std::collections::HashSet<String> = std::collections::HashSet::new();
    scan_go_imports_recursive(project_root, &mut imports, 0);
    imports
}

fn scan_go_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        ".git" | "vendor" | "node_modules" | "target"
                            | "build" | "dist" | "testdata"
                    ) { continue }
                }
                scan_go_imports_recursive(&path, out, depth + 1);
            } else if ft.is_file() {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                if !name.ends_with(".go") || name.ends_with("_test.go") { continue }
                let Ok(content) = std::fs::read_to_string(&path) else { continue };
                extract_imports_from_go_source(&content, out);
            }
        }
    }
}

fn extract_imports_from_go_source(content: &str, out: &mut std::collections::HashSet<String>) {
    enum Mode { Top, InBlock }
    let mut mode = Mode::Top;
    for line in content.lines() {
        let trimmed = line.trim();
        match mode {
            Mode::Top => {
                if trimmed.starts_with("import (") { mode = Mode::InBlock; continue }
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    let rest = rest.trim_start_matches('_').trim();
                    let quoted = rest
                        .rsplit_once('"')
                        .map(|(head, _)| head)
                        .and_then(|head| head.rsplit_once('"').map(|(_, s)| s));
                    if let Some(path) = quoted {
                        if !path.is_empty() { out.insert(path.to_string()); }
                    }
                }
            }
            Mode::InBlock => {
                if trimmed == ")" { mode = Mode::Top; continue }
                let bytes = trimmed.as_bytes();
                let first = bytes.iter().position(|&b| b == b'"');
                let Some(start) = first else { continue };
                let after = &trimmed[start + 1..];
                let Some(end_rel) = after.find('"') else { continue };
                let path = &after[..end_rel];
                if !path.is_empty() { out.insert(path.to_string()); }
            }
        }
    }
}

fn go_dep_is_imported(
    dep_path: &str,
    user_imports: &std::collections::HashSet<String>,
) -> bool {
    if user_imports.contains(dep_path) { return true }
    let prefix = format!("{dep_path}/");
    user_imports.iter().any(|imp| imp.starts_with(&prefix))
}

pub fn gomodcache_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GOMODCACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p) }
    }
    if let Some(gopath) = std::env::var_os("GOPATH") {
        let first = PathBuf::from(gopath)
            .to_string_lossy()
            .split(|c| c == ':' || c == ';')
            .next()
            .map(PathBuf::from);
        if let Some(p) = first {
            let candidate = p.join("pkg").join("mod");
            if candidate.is_dir() { return Some(candidate) }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join("go").join("pkg").join("mod");
    if candidate.is_dir() { Some(candidate) } else { None }
}

fn resolve_go_dep_path(cache_root: &Path, dep: &GoModDep) -> Option<PathBuf> {
    let escaped = escape_module_path(&dep.path);
    let dirname = format!("{}@{}", escaped, dep.version);
    let candidate = cache_root.join(dirname.replace('/', std::path::MAIN_SEPARATOR_STR));
    if candidate.is_dir() { return Some(candidate) }
    let mut segments: Vec<&str> = escaped.split('/').collect();
    let last = segments.pop()?;
    let mut path = cache_root.to_path_buf();
    for seg in segments { path.push(seg); }
    path.push(format!("{last}@{}", dep.version));
    if path.is_dir() { Some(path) } else { None }
}

fn escape_module_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 4);
    for ch in path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_go_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "vendor" | "testdata" | ".git" | "_examples") { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".go") { continue }
            if name.ends_with("_test.go") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{}@{}/{}", dep.module_path, dep.version, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "go",
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
        let g = GoModEcosystem;
        assert_eq!(g.id(), ID);
        assert_eq!(Ecosystem::kind(&g), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&g), &["go"]);
    }

    #[test]
    fn legacy_locator_tag_is_go() {
        assert_eq!(ExternalSourceLocator::ecosystem(&GoModEcosystem), "go");
    }

    #[test]
    fn escape_preserves_lowercase_paths() {
        assert_eq!(
            escape_module_path("github.com/gin-gonic/gin"),
            "github.com/gin-gonic/gin"
        );
    }

    #[test]
    fn escape_handles_uppercase_segments() {
        assert_eq!(
            escape_module_path("github.com/Microsoft/go-winio"),
            "github.com/!microsoft/go-winio"
        );
        assert_eq!(
            escape_module_path("github.com/AlecAivazis/survey"),
            "github.com/!alec!aivazis/survey"
        );
    }

    #[test]
    fn discover_returns_empty_without_go_mod() {
        let tmp = std::env::temp_dir().join("bw-test-gomod-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let result = discover_go_externals(&tmp);
        assert!(result.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_go_mod_basic() {
        let content = r#"module foo.example/bar

go 1.21

require (
    github.com/gin-gonic/gin v1.9.1
    github.com/stretchr/testify v1.9.0 // indirect
)

require github.com/other/pkg v1.0.0
"#;
        let data = parse_go_mod(content);
        assert_eq!(data.module_path.as_deref(), Some("foo.example/bar"));
        assert_eq!(data.require_deps.len(), 3);
        assert!(data.require_deps.iter().any(|d| d.path == "github.com/gin-gonic/gin" && !d.indirect));
        assert!(data.require_deps.iter().any(|d| d.path == "github.com/stretchr/testify" && d.indirect));
        assert!(data.require_deps.iter().any(|d| d.path == "github.com/other/pkg"));
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
