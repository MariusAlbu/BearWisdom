use crate::types::*;

pub static YAML: LanguageDescriptor = LanguageDescriptor {
    id: "yaml",
    display_name: "YAML",
    file_extensions: &[".yml", ".yaml"],
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
