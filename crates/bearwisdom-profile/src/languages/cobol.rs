use crate::types::*;

pub static COBOL: LanguageDescriptor = LanguageDescriptor {
    id: "cobol",
    display_name: "COBOL",
    file_extensions: &[".cob", ".cbl", ".cpy"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("*>"),
    block_comment: None,
};
