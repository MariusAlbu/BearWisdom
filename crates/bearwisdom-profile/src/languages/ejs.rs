use crate::types::*;

pub static EJS: LanguageDescriptor = LanguageDescriptor {
    id: "ejs",
    display_name: "EJS",
    file_extensions: &[".ejs"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("<%#", "%>")),
};
