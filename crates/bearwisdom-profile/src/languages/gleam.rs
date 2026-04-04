use crate::types::*;

pub static GLEAM: LanguageDescriptor = LanguageDescriptor {
    id: "gleam",
    display_name: "Gleam",
    file_extensions: &[".gleam"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: None,
};
