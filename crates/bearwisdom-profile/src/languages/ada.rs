use crate::types::*;

pub static ADA: LanguageDescriptor = LanguageDescriptor {
    id: "ada",
    display_name: "Ada",
    file_extensions: &[".adb", ".ads"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("--"),
    block_comment: None,
};
