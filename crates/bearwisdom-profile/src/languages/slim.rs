use crate::types::*;

pub static SLIM: LanguageDescriptor = LanguageDescriptor {
    id: "slim",
    display_name: "Slim",
    file_extensions: &[".slim"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("/"),
    block_comment: None,
};
