use crate::types::*;

pub static PUG: LanguageDescriptor = LanguageDescriptor {
    id: "pug",
    display_name: "Pug",
    file_extensions: &[".pug", ".jade"],
    filenames: &[],
    aliases: &["jade"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: None,
};
