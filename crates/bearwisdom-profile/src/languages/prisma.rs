use crate::types::*;

pub static PRISMA: LanguageDescriptor = LanguageDescriptor {
    id: "prisma",
    display_name: "Prisma",
    file_extensions: &[".prisma"],
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
