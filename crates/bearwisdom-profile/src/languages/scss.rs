use crate::types::*;

pub static SCSS: LanguageDescriptor = LanguageDescriptor {
    id: "scss",
    display_name: "SCSS",
    file_extensions: &[".scss", ".sass"],
    filenames: &[],
    aliases: &["sass"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
