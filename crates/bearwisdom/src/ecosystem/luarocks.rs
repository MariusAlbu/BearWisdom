// =============================================================================
// ecosystem/luarocks.rs — LuaRocks ecosystem (Lua)
//
// Phase 2 + 3: consolidates `indexer/externals/lua.rs` +
// `indexer/manifest/rockspec.rs`.
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

pub const ID: EcosystemId = EcosystemId::new("luarocks");
const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["lua"];
const LEGACY_ECOSYSTEM_TAG: &str = "lua";

pub struct LuarocksEcosystem;

impl Ecosystem for LuarocksEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("lua"),
        ])
    }
    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_lua_externals(ctx.project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_lua_root(dep) }
}

impl ExternalSourceLocator for LuarocksEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_lua_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_lua_root(dep) }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<LuarocksEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(LuarocksEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (rockspec)
// ===========================================================================

pub struct RockspecManifest;

impl ManifestReader for RockspecManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Rockspec }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let Ok(entries) = std::fs::read_dir(project_root) else { return None };
        let rockspec = entries.flatten().find(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("rockspec")
        })?;
        let content = std::fs::read_to_string(rockspec.path()).ok()?;
        let mut data = ManifestData::default();
        for name in parse_rockspec_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_rockspec_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("dependencies") else { return deps };
    let rest = &content[start..];
    let Some(brace) = rest.find('{') else { return deps };
    let rest = &rest[brace + 1..];
    let Some(end) = rest.find('}') else { return deps };
    let block = &rest[..end];

    for part in block.split(',') {
        let trimmed = part.trim().trim_matches(|c: char| c == '\'' || c == '"' || c.is_whitespace());
        if trimmed.is_empty() { continue }
        let name = trimmed.split(|c: char| c.is_whitespace() || c == '>' || c == '<' || c == '=' || c == '~')
            .next().unwrap_or("").trim();
        if !name.is_empty() && name != "lua" { deps.push(name.to_string()) }
    }
    deps
}

// ===========================================================================
// Discovery + walk
// ===========================================================================

pub fn discover_lua_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new() };
    let rockspec_file = entries.flatten().find(|e| {
        e.path().extension().and_then(|x| x.to_str()) == Some("rockspec")
    });
    let Some(rs_entry) = rockspec_file else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(rs_entry.path()) else { return Vec::new() };
    let declared = parse_rockspec_deps(&content);
    if declared.is_empty() { return Vec::new() }

    let lua_dirs = lua_module_dirs(project_root);
    let mut roots = Vec::new();
    for dep in &declared {
        let dep_file = format!("{}.lua", dep.replace('.', std::path::MAIN_SEPARATOR_STR));
        let dep_dir = dep.replace('.', std::path::MAIN_SEPARATOR_STR);
        for lib in &lua_dirs {
            let as_file = lib.join(&dep_file);
            let as_dir = lib.join(&dep_dir);
            if as_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::new(),
                    root: as_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
                break;
            } else if as_file.is_file() {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::new(),
                    root: as_file.parent().unwrap_or(lib).to_path_buf(),
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
                break;
            }
        }
    }
    debug!("Lua: {} external module roots", roots.len());
    roots
}

fn lua_module_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local = project_root.join("lua_modules").join("share").join("lua").join("5.1");
    if local.is_dir() { dirs.push(local) }
    let local2 = project_root.join("lua_modules").join("lib").join("lua").join("5.1");
    if local2.is_dir() { dirs.push(local2) }
    if let Ok(path) = std::env::var("LUA_PATH") {
        for p in path.split(';') {
            let p = p.replace('?', "").replace("/init.lua", "");
            let pb = PathBuf::from(p.trim_end_matches('/'));
            if pb.is_dir() { dirs.push(pb) }
        }
    }
    dirs
}

fn walk_lua_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
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
                if matches!(name, "tests" | "test" | "spec" | "examples") || name.starts_with('.') { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".lua") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:lua:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "lua",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        assert_eq!(LuarocksEcosystem.id(), ID);
        assert_eq!(Ecosystem::languages(&LuarocksEcosystem), &["lua"]);
    }

    #[test]
    fn parse_rockspec() {
        let content = r#"
dependencies = {
  'lua == 5.1',
  'plenary.nvim',
}
"#;
        let deps = parse_rockspec_deps(content);
        assert_eq!(deps, vec!["plenary.nvim"]);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
