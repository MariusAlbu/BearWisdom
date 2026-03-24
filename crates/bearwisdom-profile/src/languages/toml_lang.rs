use crate::types::*;

pub static TOML: LanguageDescriptor = LanguageDescriptor {
    id: "toml",
    display_name: "TOML",
    file_extensions: &[".toml"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
