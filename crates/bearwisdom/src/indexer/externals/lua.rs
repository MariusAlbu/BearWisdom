// Lua / LuaRocks externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Lua LuaRocks → `discover_lua_externals` + `walk_lua_external_root`.
///
/// LuaRocks packages installed locally live in `lua_modules/share/lua/5.1/` or
/// `lua_modules/lib/lua/5.1/`. Global packages are on `LUA_PATH`.
/// Declared deps come from `*.rockspec` `dependencies` field.
/// Walk: `*.lua` files under the module root.
pub struct LuaExternalsLocator;

impl ExternalSourceLocator for LuaExternalsLocator {
    fn ecosystem(&self) -> &'static str { "lua" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_lua_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_lua_external_root(dep)
    }
}

pub fn discover_lua_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::rockspec::parse_rockspec_deps;

    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new(); };
    let rockspec_file = entries.flatten().find(|e| {
        e.path().extension().and_then(|x| x.to_str()) == Some("rockspec")
    });
    let Some(rs_entry) = rockspec_file else { return Vec::new(); };
    let Ok(content) = std::fs::read_to_string(rs_entry.path()) else { return Vec::new(); };
    let declared = parse_rockspec_deps(&content);
    if declared.is_empty() { return Vec::new(); }

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
                    ecosystem: "lua",
                    package_id: None,
                });
                break;
            } else if as_file.is_file() {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::new(),
                    root: as_file.parent().unwrap_or(lib).to_path_buf(),
                    ecosystem: "lua",
                    package_id: None,
                });
                break;
            }
        }
    }
    debug!("Lua: discovered {} external module roots", roots.len());
    roots
}

fn lua_module_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local = project_root.join("lua_modules").join("share").join("lua").join("5.1");
    if local.is_dir() { dirs.push(local); }
    let local2 = project_root.join("lua_modules").join("lib").join("lua").join("5.1");
    if local2.is_dir() { dirs.push(local2); }
    if let Ok(path) = std::env::var("LUA_PATH") {
        for p in path.split(';') {
            let p = p.replace('?', "").replace("/init.lua", "");
            let pb = PathBuf::from(p.trim_end_matches('/'));
            if pb.is_dir() { dirs.push(pb); }
        }
    }
    dirs
}

pub fn walk_lua_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_lua_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_lua_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_lua_dir_bounded(dir, root, dep, out, 0);
}

fn walk_lua_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "spec" | "examples") || name.starts_with('.') { continue; }
            }
            walk_lua_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".lua") { continue; }
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
