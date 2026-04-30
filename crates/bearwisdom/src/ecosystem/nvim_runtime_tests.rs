// Sibling tests for `nvim_runtime.rs`. Verifies probe + walk shape against
// stub fixture directories rather than requiring an installed Neovim.

use super::*;
use std::fs;
use tempfile::TempDir;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn make_runtime_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let runtime = tmp.path();
    write_file(&runtime.join("lua/vim/lsp.lua"), "-- vim.lsp module");
    write_file(&runtime.join("lua/vim/api.lua"), "-- vim.api module");
    write_file(&runtime.join("lua/vim/treesitter.lua"), "-- treesitter");
    write_file(&runtime.join("lua/vim/lsp/handlers.lua"), "-- lsp handlers");
    write_file(&runtime.join("lua/vim/test/_helpers.lua"), "-- test only");
    write_file(&runtime.join("lua/luassert/init.lua"), "-- assertion lib");
    write_file(&runtime.join("syntax/lua.vim"), "\" vimscript");
    write_file(&runtime.join("doc/lua.txt"), "vim help text");
    tmp
}

#[test]
fn probe_via_env_override_finds_runtime() {
    let fixture = make_runtime_fixture();
    let key = "BEARWISDOM_NVIM_RUNTIME";
    std::env::set_var(key, fixture.path());
    let probed = probe_runtime_dir();
    std::env::remove_var(key);
    assert_eq!(probed.as_deref(), Some(fixture.path()));
}

#[test]
fn probe_skips_directory_without_lua_subdir() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("syntax/lua.vim"), "\" vim only");
    let key = "BEARWISDOM_NVIM_RUNTIME";
    std::env::set_var(key, tmp.path());
    // Other env probes may still match — clear $VIMRUNTIME to keep this
    // test deterministic in environments where Neovim is running.
    let prior_vimruntime = std::env::var_os("VIMRUNTIME");
    std::env::remove_var("VIMRUNTIME");
    let probed = probe_runtime_dir();
    std::env::remove_var(key);
    if let Some(v) = prior_vimruntime { std::env::set_var("VIMRUNTIME", v); }
    // BEARWISDOM_NVIM_RUNTIME must reject the bad fixture; whatever the
    // remaining probes return is fine, just confirm the override didn't
    // win when the runtime layout is wrong.
    assert_ne!(probed.as_deref(), Some(tmp.path()));
}

#[test]
fn walk_emits_lua_files_under_runtime_lua_dir() {
    let fixture = make_runtime_fixture();
    let dep = ExternalDepRoot {
        module_path: "nvim-runtime".to_string(),
        version: String::new(),
        root: fixture.path().to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_runtime_tree(&dep);
    let names: Vec<String> = walked
        .iter()
        .map(|w| {
            w.absolute_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    // Public-API .lua files are emitted.
    assert!(names.contains(&"lsp.lua".to_string()));
    assert!(names.contains(&"api.lua".to_string()));
    assert!(names.contains(&"treesitter.lua".to_string()));
    assert!(names.contains(&"handlers.lua".to_string()));
    assert!(names.contains(&"init.lua".to_string()));
    // Under the lua/ subtree only — Vimscript and doc files are not picked up.
    assert!(!names.iter().any(|n| n.ends_with(".vim")));
    assert!(!names.iter().any(|n| n.ends_with(".txt")));
    // test/ trees are skipped.
    assert!(!names.contains(&"_helpers.lua".to_string()));
}

#[test]
fn walk_skips_runtime_without_lua_subdir() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("syntax/lua.vim"), "\" vim only");
    let dep = ExternalDepRoot {
        module_path: "nvim-runtime".to_string(),
        version: String::new(),
        root: tmp.path().to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    assert!(walk_runtime_tree(&dep).is_empty());
}

#[test]
fn walked_files_carry_ext_nvim_virtual_prefix() {
    let fixture = make_runtime_fixture();
    let dep = ExternalDepRoot {
        module_path: "nvim-runtime".to_string(),
        version: String::new(),
        root: fixture.path().to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_runtime_tree(&dep);
    assert!(!walked.is_empty());
    for w in &walked {
        assert!(
            w.relative_path.starts_with("ext:nvim:"),
            "unexpected virtual path: {}",
            w.relative_path
        );
        assert_eq!(w.language, "lua");
    }
}

#[test]
fn ecosystem_identity() {
    let eco = NvimRuntimeEcosystem;
    assert_eq!(eco.id().as_str(), "nvim-runtime");
    assert_eq!(eco.kind(), EcosystemKind::Stdlib);
    assert_eq!(eco.languages(), &["lua"]);
}
