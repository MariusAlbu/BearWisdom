use crate::types::*;

pub static LIQUID: LanguageDescriptor = LanguageDescriptor {
    id: "liquid",
    display_name: "Liquid",
    file_extensions: &[".liquid"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: None,
};
