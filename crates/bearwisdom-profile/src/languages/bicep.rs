use crate::types::*;

pub static BICEP: LanguageDescriptor = LanguageDescriptor {
    id: "bicep",
    display_name: "Bicep",
    file_extensions: &[".bicep"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
