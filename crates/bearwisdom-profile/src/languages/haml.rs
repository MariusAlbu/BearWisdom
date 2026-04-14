use crate::types::*;

pub static HAML: LanguageDescriptor = LanguageDescriptor {
    id: "haml",
    display_name: "Haml",
    file_extensions: &[".haml"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("-#"),
    block_comment: None,
};
