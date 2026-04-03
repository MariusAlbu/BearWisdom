use crate::types::*;

pub static LUA: LanguageDescriptor = LanguageDescriptor {
    id: "lua",
    display_name: "Lua",
    file_extensions: &[".lua"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("--"),
    block_comment: Some(("--[[", "]]")),
};
