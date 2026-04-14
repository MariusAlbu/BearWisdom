use crate::types::*;

pub static MDX: LanguageDescriptor = LanguageDescriptor {
    id: "mdx",
    display_name: "MDX",
    file_extensions: &[".mdx"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("{/*", "*/}")),
};
