use crate::types::*;

pub static CSS: LanguageDescriptor = LanguageDescriptor {
    id: "css",
    display_name: "CSS",
    file_extensions: &[".css"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("/*", "*/")),
};
