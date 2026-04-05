use crate::types::*;

pub static ROBOT: LanguageDescriptor = LanguageDescriptor {
    id: "robot",
    display_name: "Robot Framework",
    file_extensions: &[".robot"],
    filenames: &[],
    aliases: &["robotframework"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
